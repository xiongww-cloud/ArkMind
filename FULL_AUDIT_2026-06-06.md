# PMS Desktop 全功能审计报告

**日期：** 2026-06-06  
**范围：** 全部功能模块（项目管理、方案配置、笔记、待办、文件、渠道/厂商、设置）  
**方法：** 每条操作链路完整走查：Vue模板事件 → 前端函数 → IPC invoke → Rust命令参数名 → SQL列名 → 返回值形状 → 模板渲染字段

---

## 🔴 致命缺陷 (P0)

### 1. 方案配置保存：deviceItems 被彻底丢弃
| 环节 | 位置 | 内容 |
|------|------|------|
| 前端 | `SolutionConfigurator.vue:458` | `projectStore.savePlanVersion(...)` → 构建 `deviceItems` 数组 |
| Store | `project.js:652` | `api.createVersion(solutionId, { versionNum, tags, content, creator, deviceItems })` |
| Rust | `project_commands.rs:52` | `solution_version_create`：插入了 `solution_versions` 行，只写了 `V1.0` 和 `creator`，**完全不读 `deviceItems`，不写 `solution_items`** |
| Rust | `project_commands.rs:55` | `solution_version_save_draft`：`_data` 参数被忽略，只 UPDATE `updated_at` |
| Rust | `project_commands.rs:54` | `solution_version_finalize`：`_data` 参数被忽略，只 UPDATE `status='final'` |

**影响：** 用户在方案配置器中添加的设备项（产品型号、数量、价格、参数标注等）提交后全部丢失，`solution_items` 表永远是空的。已保存的版本从后端重新加载时 `deviceItems` 为空数组。

### 2. 项目字段更新：仅5个字段生效，其余全部静默丢弃
| 环节 | 位置 | 内容 |
|------|------|------|
| 前端 | `ProjectDetail.vue:159,180,201,228,730` | 发送 `bid_time`, `description`, `channelName` 等多种字段 |
| Rust | `project_commands.rs:17` | 只匹配 `name`, `status`, `owner`, `customer`, `budget` (含 f64) |
| 遗漏字段 | — | `bid_time`, `description`, `channel_name`, `channel_phone`, `channel_company`, `sales_name`, `sales_phone`, `vendor_sales_name`, `vendor_sales_phone`, `vendor_tech_name`, `vendor_tech_phone`, `customer_contact`, `customer_phone` |

**影响：** 项目详情页编辑投标时间、描述、渠道信息、销售信息、客户联系方式后，数据静默丢失——用户以为保存成功了，但下次刷新全部恢复旧值。

---

## 🟡 功能缺失 (P1)

### 3. `project_update` 缺少的 13 个可编辑字段
`project_commands.rs:17` 需补充：
- `bidTime` → `bid_time`（投标时间）
- `description` → `description`（项目描述）
- `channelName` → `channel_name`
- `channelPhone` → `channel_phone`
- `channelCompany` → `channel_company`
- `salesName` → `sales_name`
- `salesPhone` → `sales_phone`
- `vendorSalesName` → `vendor_sales_name`
- `vendorSalesPhone` → `vendor_sales_phone`
- `vendorTechName` → `vendor_tech_name`
- `vendorTechPhone` → `vendor_tech_phone`
- `customerContact` → `customer_contact`
- `customerPhone` → `customer_phone`

### 4. SolutionConfigurator 保存需实现完整的 solution_items 写入/更新
当前 `solution_version_create` + `solution_version_save_draft` 中完全没有 `solution_items` 的处理逻辑。需要实现：
- `solution_version_create` 接收 `deviceItems` 数组，插入 `solution_items` 行
- `solution_version_save_draft` 接收 `deviceItems`，删除旧 items 并重插（或 diff 更新）

### 5. `project_update` 字段名双重匹配
前端可能传 camelCase (`bidTime`) 或 snake_case (`bid_time`) 字段名。`project_update` Rust 只处理了 5 个字段且都是简单 str 匹配，没有做 camelCase→snake_case 转换。

### 6. `solution_version_list` 不返回 items
| 位置 | 内容 |
|------|------|
| `project_commands.rs:51` | `SELECT * FROM solution_versions` 不包括关联的 `solution_items` |
| `project.js:369-391` | Store 中 `fetchPlanVersions` 期望 `v.deviceItems` 数组，但后端未 JOIN |

**影响：** 已保存版本的设备清单从后端加载时为空。

---

## 🟠 数据完整性问题 (P2)

### 7. `project_delete` 不清理关联文件
| 位置 | 内容 |
|------|------|
| `project_commands.rs:25-31` | 只删除了 `file_project_mapping`, `note_project_mapping`, `project_tags_mapping`, `project_dynamics`, `projects` |
| 遗漏 | `solutions`, `solution_versions`, `solution_items`, `channel_partners`, `channel_contacts`, `vendor_partners`, `vendor_contacts`, `file_assets` 孤立检查 |

对比 `product_delete`（已修复）有完整的孤立文件清理逻辑，但 `project_delete` 完全没有。

### 8. 笔记 `notes_update` 的 `notebookId` null 值处理
| `notes_commands.rs:56` | `if let Some(v) = data["notebookId"].as_str()` 只匹配非空字符串 |
| 问题 | 前端想将笔记移出笔记本时（设置 `notebook_id` 为 null），后端因 `as_str()` 返回 `None` 而跳过 |

### 9. FileManager 标签系统两套并存
| 文件 | 旧标签系统 | 新 AI 标签系统 |
|------|-----------|--------------|
| `FileManager.vue:106-147` | `f.tags` 逗号分隔字符串，用 `updateFileMeta` 更新 | — |
| `FileManager.vue:155-184` | — | `file_tags_mapping` 表，用 `fileAddTagMapping`/`fileRemoveTagMapping` |
| Rust | `file_update_meta` 无 `tags` 字段 | `file_add_tag_mapping`/`file_remove_tag_mapping` 正常 |

两套标签系统互不感知——逗号分隔标签存在 `file_assets.tags` 列中，AI 标签存在 `file_tags_mapping` 中。用户在 FileManager 添加的逗号标签不会被 AI 打标队列处理。

### 10. 导出功能全部为空壳
| 命令 | 位置 | 状态 |
|------|------|------|
| `project_export` | `project_commands.rs:41` | `Ok(json!({"success":true}))` — 空壳 |
| `project_import` | `project_commands.rs:42` | 空壳 |
| `project_import_file` | `project_commands.rs:43` | 空壳 |
| `project_export_files` | `project_commands.rs:44` | 空壳 |
| `projects_batch_export` | `project_commands.rs:45` | 空壳 |
| `solution_download_excel` | `project_commands.rs:57` | 空壳 |

前端 Dashboard 和 ProjectDetail 都调用了这些导出功能，用户点击后没反应也不会报错。

---

## 🔵 低风险问题 (P3)

### 11. 渠道/厂商联系人无 `sort_order` 排序
`partner_commands.rs` 中所有 SELECT 都不带 `ORDER BY sort_order`，联系人按插入顺序或随机顺序排列。

### 12. `notes_list_all` 返回嵌套数组
`notes_commands.rs:7` 使用 `GROUP_CONCAT(npm.project_id) as project_ids` 但返回值为字符串（逗号分隔），前端可能当数组使用。

### 13. `file_list` 不返回版本信息
`file_commands.rs:33-34` 的 `file_list` 只查 `file_assets`，不 JOIN `file_versions`。FileManager 需要额外调用。

### 14. `product_update` SQL 注入风险（低）
`product_commands.rs:26`、`project_commands.rs:21` 等使用 `format!` 拼接 SQL，字段名来自 JSON 键名，未做白名单校验。

---

## ✅ 已验证正常的模块

| 模块 | 链路 | 状态 |
|------|------|------|
| 产品中心 | 上传/列表/预览/删除 | ✅ 已修复全部问题 |
| 笔记 | CRUD/标签/笔记本/回收站 | ✅ 数据流完整 |
| 待办 | CRUD/切换完成/关联项目 | ✅ 数据流完整 |
| 渠道/厂商 | CRUD/联系人 | ✅ 数据流基本完整 |
| 物料下载 | 从前端到后端参数 | ✅ 已修复键名不匹配 |
| 产品描述 | 保存到后端 | ✅ 已补全字段 |
| 设置 | 加载/保存/路径缓存 | ✅ 已修复完整 |
| 文件管理 | 上传/列表/下载/打开 | ✅ 主要流程完整 |
| 知识库 | kb_* 命令注册 | ✅ 接口存在 |

---

## 📋 修复建议优先级

| 优先级 | 问题 | 影响 |
|--------|------|------|
| **P0** | #1 方案配置 items 丢失 | 方案配置功能完全不可用 |
| **P0** | #2/3/5 项目字段编辑丢弃 13 个字段 | 项目详情大部分编辑无效 |
| **P1** | #4 solution_items 存储实现 | 方案配置依赖此问题 |
| **P1** | #6 solution_version_list 不返回 items | 版本历史加载为空 |
| **P2** | #7 project_delete 不清理关联数据 | 磁盘和数据库残留 |
| **P2** | #8 笔记 notebookId 清空 | 移出笔记本功能失效 |
| **P2** | #9 双标签系统 | 功能混淆 |
| **P3** | #10 导出空壳 | 用户无反馈 |

---

**总结：** 12 个功能模块中，4 个有致命缺陷（方案配置、项目编辑、项目删除、方案加载），2 个有中等缺陷（笔记移出、双标签），6 个基本正常（产品中心、待办、渠道/厂商、设置、文件管理、知识库接口层）。
