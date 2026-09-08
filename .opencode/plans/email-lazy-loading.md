# 邮件大图分批/懒加载优化计划

## 背景
打开含大图的邮件时，`email_render_html` 一次性在后端读取所有内嵌图片并 base64 编码，返回超大 HTML，前端一次性渲染 → 慢、占内存。

## 决策（已与用户确认）
- 小邮件（内嵌图片总字节 ≤ 256KB）保留**一次性内联**快速路径（lazy=false）。
- 大邮件返回占位 HTML（src 置空 + `data-emimg="<token>"`），前端用 **IntersectionObserver 视口懒加载**，并发上限 ~3 逐张拉取。

## 后端修改 — `src-tauri/src/commands/email_commands.rs`
1. 新增常量 `EMAIL_EAGER_THRESHOLD: i64 = 256 * 1024`。
2. 抽出纯解析辅助：
   - `email_local_token(src, cid_to_uuid) -> Option<String>`：把 `cid:` / `pms://file/asset/{uuid}/content` / `asset://` / `asset.localhost/...` 归一为规范 token（file 的 uuid 或 64 位 content_hash），无法解析返回 None。
3. `email_render_html(id)` 改为：
   - 查询 `SELECT COALESCE(SUM(size),0) FROM email_attachments WHERE email_id=? AND is_inline=1` 得总字节。
   - **eager（≤阈值）**：沿用现有逻辑，把本地图 src 替换为 dataURL，返回 `{ html, lazy:false }`（当前行为，不动）。
   - **lazy（>阈值）**：对每个本地 `<img>` 重写为 `src=""` + 追加 `data-emimg="<token>"`（保留原 width/height/style），返回 `{ html, lazy:true }`。注意：这一路径不读文件、不做 base64，只是纯字符串替换，秒回。
   - 实现：用 `<img\b[^>]*>` 整标签正则逐段处理，避免 Rust regex 回溯引用限制。
4. 新增命令 `email_inline_asset(email_id: String, token: String) -> { success, dataUrl }`：
   - token 为 uuid 或 hash 直接解析（复用 `email_asset_path_by_uuid` / `email_asset_path_by_hash` / `email_build_data_url`）；
   - 兼容 cid：用 `email_id` 查 `email_attachments` 把 cid→uuid 后解析；
   - 单张返回，作为懒加载的逐图拉取接口。

## 后端注册 — `src-tauri/src/lib.rs`
- `generate_handler![...]` 中加入 `commands::email_inline_asset`。

## 前端修改
### `src/renderer/src/api/email-client.ts`
- `emailRenderHtml` 返回值类型补 `lazy?: boolean`。
- 新增 `emailInlineAsset(emailId: string, token: string)`。

### 新增 `src/renderer/src/utils/emailLazyImages.ts`
- `lazyLoadEmailImages(container: HTMLElement, emailId: string)`：
  - 收集 `img[data-emimg]`；
  - `IntersectionObserver`，root 取正文滚动容器（拿不到则视口），`rootMargin:'300px'`，进入视口才发起加载；
  - 并发池上限 3，拉取到就 `img.src = dataUrl`、移除 `data-emimg`；
  - 取消保护（切换邮件/组件卸载时丢弃后续回调）；
  - 无 IntersectionObserver 时退化为受限并发按序加载。

### `EmailPage.vue` / `EmailDetail.vue`
- 正文 `v-html` 容器加模板 ref（如 `emailBodyEl`）。
- `loadRenderedHtml`：收到 `r.lazy` 时，`nextTick` 后调用 `lazyLoadEmailImages(emailBodyEl, id)`；eager 路径不变。
- 占位图加载前给 `img[data-emimg]` 加淡灰背景，减少闪烁。

## 验证
- 邮件列表邮件（Huawei 含多张 1~4MB JPEG 那类）：打开秒出文字/占位，滚动逐步加载，不再卡顿、无 `ERR_UNKNOWN_URL_SCHEME`。
- 小邮件仍一次性渲染。
- 切到另一封邮件时旧图片加载请求不生效（取消保护）。
