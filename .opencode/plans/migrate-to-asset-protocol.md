# 迁移计划：从 pms:// 自定义协议到 asset:// 内置协议

## 背景

当前使用 `pms://` 自定义协议访问本地文件，导致 CSP 配置复杂、图片加载失败等问题。
Tauri 2.x 内置了 `asset://` 协议，可以简化配置。

## 修改清单

### 1. 修改 `tauri.conf.json`

**文件**: `src-tauri/tauri.conf.json`

```json
{
  "app": {
    "security": {
      "assetProtocol": {
        "enable": true,
        "scope": ["**"]
      },
      "csp": "default-src 'self' asset: https://asset.localhost; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' asset: https://asset.localhost blob: data:; font-src 'self'; connect-src 'self' https://api.openai.com https://api.siliconflow.cn https://dashscope.aliyuncs.com; media-src 'self'"
    }
  }
}
```

**变更**:
- 启用 `assetProtocol`
- CSP 中移除 `pms:` 协议，使用 `asset: https://asset.localhost`
- 移除 `dangerousRemoteUrlIpcAccess`（无效配置）

### 2. 修改 `email_polling.rs` - 邮件图片 URL 生成

**文件**: `src-tauri/src/services/email_polling.rs`

将所有 `pms://file/asset/{id}/content` 替换为使用 `convertFileSrc` 或直接使用文件路径：

```rust
// 旧代码
let api_url = format!("pms://file/asset/{}/content", asset_id);

// 新代码 - 使用 asset:// 协议
let file_path = crate::commands::file_commands::flat_disk_path(&hash);
let api_url = format!("asset://localhost/{}", file_path.replace("\\", "/"));
```

**需要修改的位置**:
- 第 224 行: `cid_to_asset` 映射中的 URL
- 第 235 行: `inline_images` 追加的 URL

### 3. 修改 `email_commands.rs` - 邮件图片 URL 生成

**文件**: `src-tauri/src/commands/email_commands.rs`

```rust
// 第 736 行
// 旧代码
let api_url = format!("pms://file/asset/{}/content", asset_id);

// 新代码
let file_path = crate::commands::file_commands::flat_disk_path(&hash);
let api_url = format!("asset://localhost/{}", file_path.replace("\\", "/"));
```

### 4. 删除 `lib.rs` 中的自定义协议处理器

**文件**: `src-tauri/src/lib.rs`

删除第 70-128 行的 `register_asynchronous_uri_scheme_protocol("pms", ...)` 代码块。

### 5. 前端 DOMPurify 配置调整

**文件**: `src/renderer/src/views/EmailPage.vue` 和 `EmailDetail.vue`

将 `ALLOW_UNKNOWN_PROTOCOLS: true` 改为允许 `asset` 协议：

```typescript
const renderedHtml = computed(() => {
  if (!email.value?.body_html) return ''
  return DOMPurify.sanitize(email.value.body_html, {
    ADD_TAGS: ['img', 'a'],
    ADD_ATTR: ['src', 'alt', 'width', 'height', 'href', 'target', 'id', 'style', 'border'],
    ALLOW_UNKNOWN_PROTOCOLS: true,  // 保留，因为 asset:// 不是标准协议
  })
})
```

### 6. 数据库迁移 - 更新已有邮件的图片 URL

**文件**: `src-tauri/src/database/db_migrations.rs`

添加迁移脚本，将 `emails` 表中已有的 `pms://file/asset/` 替换为新的 URL 格式：

```sql
UPDATE emails SET body_html = REPLACE(body_html, 'pms://file/asset/', 'asset://localhost/file/asset/')
WHERE body_html LIKE '%pms://file/asset/%';
```

**注意**: 这个迁移可能需要根据实际的 URL 格式调整。

## 执行顺序

1. ✅ 修改 `tauri.conf.json` - 启用 assetProtocol
2. ✅ 修改 `email_polling.rs` - 更新 URL 生成逻辑
3. ✅ 修改 `email_commands.rs` - 更新 URL 生成逻辑
4. ✅ 删除 `lib.rs` 中的自定义协议处理器
5. ✅ 前端 DOMPurify 配置确认
6. ✅ 添加数据库迁移脚本
7. ✅ 重新构建并测试

## 验证方式

1. 启动应用，检查邮件中的图片是否正常显示
2. 检查浏览器控制台是否有 CSP 错误
3. 检查是否有 `ERR_UNKNOWN_URL_SCHEME` 错误

## 风险评估

- **低风险**: 配置修改，不会影响现有功能
- **中风险**: 数据库迁移需要测试，建议先备份
- **低风险**: 前端配置调整，向后兼容

## 回滚方案

如果出现问题，可以：
1. 恢复 `tauri.conf.json` 中的 CSP 配置
2. 恢复 `lib.rs` 中的自定义协议处理器
3. 恢复 `email_polling.rs` 和 `email_commands.rs` 中的 URL 格式
