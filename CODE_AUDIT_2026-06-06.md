# PMS Desktop 代码与功能审计报告

**日期：** 2026-06-06  
**范围：** 设置保存/加载、文件上传存储、物料预览、IPC 通信、数据一致性、错误处理  
**后端：** Tauri v2 (Rust) + Electron (legacy)

---

## 🔴 严重 (Critical)

### 1. `file_unlink_from_product` 删除逻辑不完整
**文件：** `src-tauri/src/commands/file_commands.rs:195` / `src/renderer/src/api/db-client.ts:82`  
**问题：** 前端 `deleteMaterial` 调用 `file_unlink_from_product` 只删除 `product_file_mapping` 关联，不删除 `file_assets` 和 `file_versions` 记录，也不删除磁盘文件。  
**影响：** 用户点击删除物料后数据库和磁盘残留孤立记录，磁盘空间无法回收。  
**建议：** 应同时删除关联的 `file_versions`、`file_assets` 记录和物理文件（或至少检查是否还有其他产品引用）。

### 2. `handleDownloadMaterial` 调用错误的 API  
**文件：** `src/renderer/src/views/ProductCenter.vue:190`  
**问题：** 商品物料下载调用 `downloadFile(mid)`，但 `downloadFile` 是项目文件的下载 API，参数 `versionId` 语义不同。物料下载应调用 `downloadMaterial`。  
**影响：** 物料下载功能完全不工作——会打开保存对话框但传入的是错误的参数，最终调用 `file_download` 而非 `file_download_product`。

### 3. `file_upload` 中 `create_dir_all` 仍静默吞错误  
**文件：** `src-tauri/src/commands/file_commands.rs:31`  
**问题：** 第 31 行 `std::fs::create_dir_all(p).ok()` 在项目文件上传时吞掉目录创建失败错误，而上品物料上传（第 135 行）已正确修复。  
**影响：** 当磁盘目录因权限不足等原因无法创建时，后续 `write` 会失败，但错误信息不精确。

### 4. `file_open` 返回值与前端约定不匹配  
**文件：** `src-tauri/src/commands/file_commands.rs:71`  
**问题：** 项目文件 `file_open` 返回 `{"success":true}` 但前端 `FileManager.vue` 可能期望 `{"success":true,"filePath":"..."}`。  
**影响：** 跨组件不一致可能导致布尔判断出错。

---

## 🟡 高 (High)

### 5. `settings_update` 使用字符串拼接构建 SQL，存在注入风险  
**文件：** `src-tauri/src/commands/settings_commands.rs:46`  
**问题：** `format!("UPDATE system_config SET {} WHERE …", fields.join(","))` 字段名来自 JSON 键名但不在拼接前做白名单校验。  
**影响：** 虽然字段名固定由前端传入且都经过了 `as_str()` 过滤，理论上受控，但不如使用参数化 SQL 安全。  
**建议：** 对字段名做白名单校验后再拼接。

### 6. `flat_disk_path` 每次调用都查询数据库  
**文件：** `src-tauri/src/commands/file_commands.rs:6-19`  
**问题：** `flat_disk_path` 在每次文件上传/打开时都执行 `SELECT local_path FROM system_config`。高频调用场景下产生不必要的 DB 查询。  
**建议：** 在启动时读取一次并缓存（用 `once_cell::sync::OnceCell`），设置变更时使缓存失效。

### 7. `file_migrate_files` 是空操作  
**文件：** `src-tauri/src/commands/file_commands.rs:85`  
**问题：** `file_migrate_files` 只返回 `{"success":true}` 但不执行任何实际操作。前端 `Settings.vue:621` 在路径变更时会调用它来搬迁文件。  
**影响：** 用户修改存储路径后旧文件不会自动搬迁到新路径，导致旧文件"丢失"（虽然磁盘上还在旧路径下）。

---

## 🟠 中 (Medium)

### 8. `file_upload` 和 `file_add_version` 目录创建仍用 `.ok()`  
**文件：** `src-tauri/src/commands/file_commands.rs:31,54`  
**问题：** 这两处 `create_dir_all` 与 `file_upload_for_product` 不一致，后者已修复为 `.map_err()`。  
**建议：** 统一改为 `.map_err()` 以获取准确错误信息。

### 9. `file_delete` 不删除磁盘文件  
**文件：** `src-tauri/src/commands/file_commands.rs:69`  
**问题：** 删除操作只删除了 `file_project_mapping`、`file_versions`、`file_assets` 数据库记录，但物理磁盘文件未被删除。  
**影响：** 磁盘空间泄露。

### 10. `product_delete` 不删除关联的磁盘文件  
**文件：** `src-tauri/src/commands/product_commands.rs:28`  
**问题：** 产品删除时级联删除了 `product_file_mapping`、`product_tags_mapping`、`product_materials`，但不检查并清理 `file_assets` 中的孤立文件和物理文件。

### 11. ProductCenter.vue 物料数据显示字段名可能不匹配  
**文件：** `src/renderer/src/views/ProductCenter.vue:300-301`  
**问题：** 模板使用 `m.global_name`、`m.file_ext`、`m.note`，但 `file_list_by_product` 返回的是 `file_assets` 表的原始列名（`global_name`、`file_ext`），没有 `note` 字段。`note` 在前端上传时存入了 `file_versions.description` 和 `file_assets.description`，但查询时不返回 `note` 列。`mapping_type` 字段也被忽略。  
**影响：** 物料卡片上的类型标记（彩页/白皮书等）可能显示为空。

### 12. `pms://` URI scheme handler 不支持新版 `file_assets` 表  
**文件：** `src-tauri/src/lib.rs:45-68`  
**问题：** 自定义协议处理器 `pms://` 用于在 WebView 中显示文件预览（PDF/图片等），但它只查 `product_materials` 和 `file_versions`，不查 `file_assets` + `flat_disk_path`。  
**影响：** 如果前端使用 `pms://material/{file_id}` 来预览产品物料文件，URL 可能无法解析。

### 13. `storage_update_config` 和 `storage_migrate` 是空操作  
**文件：** `src-tauri/src/commands/settings_commands.rs:56,58`  
**问题：** 这两个命令只返回成功但不执行任何操作，而 Electron 版本 (`src/main/ipc-handlers.ts`) 有完整的 OSS 配置保存和搬迁逻辑。  
**影响：** Tauri 模式下 OSS 存储和搬迁功能不可用。

---

## 🔵 低 (Low)

### 14. `file_update_version_meta` 缺少 `content_hash` 更新  
**文件：** `src-tauri/src/commands/file_commands.rs:97-105`  
**问题：** 只支持更新 `description` 和 `versionNum`，不支持更新 `content_hash`。

### 15. 多个 `.ok()` 错误吞没  
**文件：** `src-tauri/src/commands/file_commands.rs:69,70,126`  
**问题：** `file_delete` 中前两个 DELETE 用 `.ok()`，`file_upload_for_product` 的 INSERT 没有用事务包裹（与 `file_upload` 不一致）。

### 16. ProductCenter.vue 用 `any` 类型过多  
**文件：** `src/renderer/src/views/ProductCenter.vue:34,64,97`  
**问题：** 大量 `as any[]` 和 `as any` 类型断言绕过了 TypeScript 检查，掩盖了潜在的类型不匹配。

### 17. `handleDeleteMaterial` 传递空的 `productId`  
**文件：** `src/renderer/src/views/ProductCenter.vue:188` / `src/renderer/src/api/db-client.ts:82`  
**问题：** `deleteMaterial` 调用 `invoke('file_unlink_from_product', { fileId: materialId, productId: '' })` 传了空字符串作为 `productId`，但后端条件 `product_id.is_empty()` 会导致 DELETE 无效（WHERE product_id=''），正确做法是传当前选中产品的 ID 或清除所有关联。

### 18. Electron IPC 处理器中存在大量仅 OSS 模式才有效的逻辑  
**文件：** `src/main/ipc-handlers.ts:228-261`  
**问题：** `getStorageBase`、`getProjectFilesDir`、`getMaterialsDir` 等函数的文件存储结构与 Tauri 版本的 `flat_disk_path` 完全不同。Electron 使用 `项目清单/{projectId}/` 和 `产品物料/` 目录结构，Tauri 使用 `{base}/{hash}` 扁平结构。  
**影响：** 如果数据在两个后端之间迁移，文件路径不兼容。

---

## ✅ 已修复（本次审计前）

1. `file_open_product` 三级回退查找策略（`file_assets` → `file_versions` → `product_materials`）
2. `file_upload_for_product` 同时写入 `file_versions` 表
3. `flat_disk_path` 改为读取 `system_config.local_path`
4. `settings_get` 返回 camelCase 键名
5. `settings_update` 保存 OSS 相关字段

---

## 📋 建议修复优先级

| 优先级 | 问题 | 影响范围 |
|--------|------|---------|
| P0 | #1 删除物料不彻底 | 磁盘泄露 |
| P0 | #2 下载调用错误 API | 物料下载不能用 |
| P1 | #3 #8 目录创建错误吞没 | 排查困难 |
| P1 | #7 文件搬迁空操作 | 路径变更后文件丢失 |
| P1 | #11 物料类型显示缺失 | 界面体验 |
| P2 | #9 #10 磁盘文件清理 | 磁盘空间 |
| P2 | #6 每次查 DB | 性能 |
| P3 | #5 SQL 注入（低风险） | 安全 |
| P3 | #12 pms:// 不支持新表 | WebView 预览 |
