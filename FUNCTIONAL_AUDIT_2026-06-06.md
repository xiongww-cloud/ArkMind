# PMS Desktop 端到端功能审计报告

**日期：** 2026-06-06  
**方法：** 逐链路走查每个用户操作——从 Vue 按钮点击 → IPC invoke → Tauri 命令 → SQL → 返回值 → 模板渲染。每个字段名、参数名、返回形状都逐一对比。

---

## 🔴 致命 Bug (操作完全不可用)

### 1. 物料下载：参数键名不匹配 → 下载永远返回"文件不存在"
| 环节 | 位置 | 内容 |
|------|------|------|
| 前端调用 | `ProductCenter.vue:190` → `db-client.ts:81` | `downloadMaterial({ materialId: mid })` → `invoke('file_download_product', { data: { materialId: mid } })` |
| Rust 读取 | `file_commands.rs:218` | `data["fileId"].as_str()` → 对象里没有 `fileId` 键，只有 `materialId` |
| 结果 | — | `file_id` 始终为空串，SQL 查不到任何行，返回"文件不存在" |

**根因：** 前端键名是 `materialId`，Rust 读的是 `fileId`，不一致。  
**影响：** 所有物料的下载按钮点击后静默失败。

---

## 🟡 功能缺陷 (操作"成功"但无效)

### 2. 产品描述编辑静默丢弃
| 环节 | 位置 | 内容 |
|------|------|------|
| 前端发送 | `ProductCenter.vue:87` | `scheduleSaveField('description', localDesc)` → `updateProduct(id, { description: value })` |
| Rust 处理 | `product_commands.rs:26` | 只检查 `name`, `category`, `seriesId`, `fullSpecs`, `standardSpecs`，**没有 `description`** |
| 结果 | — | `fields` 为空时直接 `return Ok(())`，UPDATE 从未执行 |

**影响：** 用户在产品详情页编辑"产品描述"文本域，离开页面后内容丢失——但没有任何错误提示，用户以为保存成功了。

### 3. 物料卡片类型标签显示英文枚举值
| 环节 | 位置 | 内容 |
|------|------|------|
| 上传存储 | `ProductCenter.vue:177` | `type: 'BROCHURE'/'WHITEPAPER'/'TEST_REPORT'/'OTHER'` |
| 查询返回 | `file_commands.rs:179` | `pfm.type as note` → 直接返回 `BROCHURE` 等枚举值 |
| 模板渲染 | `ProductCenter.vue:301` | `{{ m.note }}` → 显示 "BROCHURE" 而非 "彩页" |

**影响：** 物料卡片的副标题行显示类似 `. PDF · BROCHURE` 而非 `. PDF · 彩页`。体验差但不影响功能。

---

## 🟠 数据残留问题

### 4. 上传忽略了 `customName` 字段
| 环节 | 位置 | 内容 |
|------|------|------|
| 前端发送 | `ProductCenter.vue:173` | `customName: "彩页 - 产品名"` |
| Rust 写入 | `file_commands.rs:164` | 从磁盘路径提取文件名作为 `global_name`，完全不读 `data["customName"]` |
| 模板回退 | `ProductCenter.vue:300` | `m.global_name || m.custom_name || m.file_name` — `custom_name` 永远是空的 |

**影响：** `custom_name` 列被浪费，模板的 `|| m.custom_name` 回退分支永远不会命中。文件显示名始终是原始文件名。

### 5. `m.file_name` 回退是死代码
| 位置 | 内容 |
|------|------|
| `ProductCenter.vue:300` | `m.file_name` 在 `file_assets` 表中不存在该列，永远为 `undefined` |
| 结果 | 当 `global_name` 为空时直接显示"未命名"，不会尝试其他来源 |

**影响：** 无实际影响（`global_name` 在上传时总是被赋值），但死代码造成误导。

---

## 📋 完整链路验证（每条独立走通）

### ✅ 物料上传
```
前端: confirmUpload → uploadMaterial({ filePath, fileName, fileExt, productId, customName, note, type, ... })
IPC:  invoke('file_upload_for_product', { data })
Rust: file_upload_for_product(data: Value) 
      → 读 productId/filePath/note/type ✅ 全部匹配
      → 写入 file_assets ✅ + file_versions ✅ + product_file_mapping ✅（事务包裹 ✅）
      → 返回 { fileId }
前端: loadMaterials() 刷新列表 ✅
```

### ✅ 物料列表
```
前端: loadMaterials → listMaterials(productId)
IPC:  invoke('file_list_by_product', { productId })
Rust: file_list_by_product(product_id: String)
      → SQL: SELECT fa.*, pfm.type as mapping_type, pfm.type as note
      → 返回蛇形列名数组 ✅
前端: materials.value = r || [] ✅ （r 就是数组）
模板: m.id / m.global_name / m.file_ext / m.note ✅ 都存在
      ⚠️ m.custom_name 空 — 见问题 #4
      ⚠️ m.note = 英文枚举 — 见问题 #3
```

### ✅ 物料预览
```
前端: handlePreviewMaterial(mid) → openMaterial(mid)
IPC:  invoke('file_open_product', { fileId: mid })  ✅ fileId 匹配
Rust: file_open_product(file_id: String)
      → 策略1 content_hash ✅ → 策略2 file_versions.file_path ✅
      → opener::open(p) 用系统默认程序打开 ✅
```

### 🔴 物料下载
```
前端: handleDownloadMaterial(mid)
      → downloadMaterial({ materialId: mid })
IPC:  invoke('file_download_product', { data: { materialId: mid } })
Rust: data["fileId"] → ❌ 键名不匹配，应为 data["materialId"]
      返回 { success: false, error: "文件不存在" }
```

### ✅ 物料删除
```
前端: deleteMaterial(mid, selectedProductId) ✅ 传入正确 productId
IPC:  invoke('file_unlink_from_product', { fileId, productId })
Rust: file_unlink_from_product(file_id, product_id)
      → DELETE product_file_mapping ✅
      → 检查剩余引用数 ✅ → 为 0 时清理文件 + file_versions + file_assets ✅
```

### ✅ 设置加载
```
前端: fetchSettings → invoke('settings_get')
Rust: settings_get 
      → SELECT ... FROM system_config
      → 显式映射 camelCase 返回 ✅
前端: data.activeStorage / data.localPath / data.ossRegion 全部 camelCase ✅
```

### ✅ 设置保存
```
前端: updateSettings({ activeStorage, localPath, ossRegion, ... })
IPC:  invoke('settings_update', { data: { activeStorage, localPath, ... } })
Rust: data["activeStorage"] → active_storage ✅
      data["localPath"] → local_path ✅ → 触发 invalidate_storage_base() ✅
      每个字段都正确映射 ✅
```

### 🔴 产品描述保存
```
前端: scheduleSaveField('description', localDesc)
      → store.updateProduct(id, { description: value })
IPC:  invoke('product_update', { id, data: { description: value } })
Rust: 检查 data["name"], data["category"], data["seriesId"], data["fullSpecs"], data["standardSpecs"]
      → ❌ 没有 data["description"]，fields 为空，直接 return Ok(())
```

### ✅ 产品其他字段保存
```
fullSpecs → data["fullSpecs"] → full_specs= ✅
standardSpecs → data["standardSpecs"] → standard_specs= ✅
```

---

## 🔧 建议修复清单

| 优先级 | 问题 | 修复位置 | 修改内容 |
|--------|------|----------|---------|
| **P0** | 下载参数键名不匹配 | `file_commands.rs:218` | `data["fileId"]` → `data["materialId"]` |
| **P0** | 产品描述丢弃 | `product_commands.rs:26` | 增加 `if let Some(v)=data["description"].as_str()` 分支 |
| **P1** | 物料类型显示英文 | `ProductCenter.vue:301` | 映射 BROCHURE→彩页, WHITEPAPER→白皮书, TEST_REPORT→检测报告, OTHER→其他 |
| **P2** | uploadMaterial 忽略 customName | `file_commands.rs:164` | 用 `data["customName"].as_str()` 覆盖默认文件名 |

---

数据完整性方面，6 条核心操作链路中 4 条完全正常（上传、列表、预览、删除），2 条有缺陷（下载参数不匹配、描述保存被丢弃）。设置模块加载和保存均已正常工作。
