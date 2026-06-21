# 前端功能审计报告 — 后端已有但前端缺失
> 生成时间：2026-06-05

---

## 一、严重：前端调用了 db-client.ts 中不存在的函数（运行时错误）

| 调用位置 | 调用方式 | db-client.ts 实际导出 | 后果 |
|--------|---------|---------------------|------|
| `stores/project.js:210` | `api.addDynamic(backendId, event)` | `createProjectDynamic` | 运行时报 undefined is not a function |
| `stores/product.js:193` | `api.uploadProductMaterial(productId, fd)` | `uploadMaterial(data)` | 运行时报 undefined is not a function |
| `stores/product.js:198` | `api.deleteProductMaterial(productId, materialId)` | `deleteMaterial(materialId)` | 运行时报 undefined is not a function |
| `components/FileManager.vue:248` | `api.uploadFile(backendId, fd)` | `uploadFile(data)` 只接受 1 个参数 | 后端收到错误参数 |

---

## 二、后端有但 db-client.ts 没有封装的命令（共 12 个）

| 后端命令 | 功能 | 是否
|----------|------|------|
| `kb_search_debug` | KB 搜索调试 | db-client 缺失 |
| `kb_data_browse` | KB 数据浏览 | db-client 缺失 |
| `kb_inspect_fts5` | KB FTS5 检查 | db-client 缺失 |
| `kb_fts5_inspector` | KB FTS5 检查器 | db-client 缺失 |
| `execute_production_rag` | 执行生产 RAG | db-client 缺失 |
| `ext_tool_search_knowledge_base` | Agent 工具-知识库搜索 | 仅在 kb_commands.rs 内部直接调用 |
| `ext_tool_get_document_page` | Agent 工具-获取文档页 | db-client 缺失 |
| `ext_tool_find_related_knowledge` | Agent 工具-查找关联知识 | db-client 缺失 |
| `ext_tool_search_by_tag` | Agent 工具-按标签搜索 | db-client 缺失 |
| `ext_tool_get_project_context` | Agent 工具-获取项目上下文 | db-client 缺失 |
| `ai_sync_diagnose_failed_files` | AI同步-诊断失败文件 | db-client 缺失 |
| `window_minimize`/`window_toggle_maximize`/`window_close` | 窗口控制 | db-client 缺失 (但前端可能直接用 Tauri API) |

---

## 三、db-client 有封装但前端页面从未使用（可访问但无 UI 入口）

### 同步/标签管理（Settings.vue 未使用）
| API 函数 | 后端实际状态 | 前端使用情况 |
|---------|------------|------------|
| `aiSyncStatus/Trigger/Diagnose/Reset/ForceResync/RetryFailed` | 6 个命令均为桩代码（返回假数据） | 无任何页面调用 |
| `knowledgePipelineStatus` | 桩代码 | 无任何页面调用 |
| `tagGetState/ListRegistered/ForceResync` | `tag_force_resync` 有真实实现 | 无任何页面调用（仅 ProductCenter 调 tagTagSingle） |
| `assetSyncStatus/Trigger/Diagnose` | 后端注册但未验证是否桩代码 | 无任何页面调用 |

### 知识库工具（KnowledgeGraph.vue 和 KnowledgeInspector.vue 是占位页）
| API 函数 | 后端状态 | 前端使用 |
|---------|---------|---------|
| `kbSearch/Del/ListDocuments/Chat` | 桩代码（返回空/假数据） | KnowledgeBase.vue 调了 kbAgentChat（真实）+ kbListDocuments（桩） |
| `kbDiagnose/TestEmbedding/TestChat` | 桩代码 | 无任何页面调用 |

### 数据导入导出
| API 函数 | 数据库端 | 前端使用 |
|---------|---------|---------|
| `exportData/importData/dbExportRaw` | 后端注册 | 无任何页面调用 |
| `productsExport/ExportFile/Import/ImportFile` | 后端注册 | 无任何页面调用 |

---

## 四、两个占位页（仅静态文字，无功能）

| 页面 | 路由 | 内容 |
|-----|------|------|
| `KnowledgeGraph.vue` | `/knowledge-graph` | 仅显示"知识库索引浏览器" |
| `KnowledgeInspector.vue` | `/inspector` | 仅显示"知识库索引诊断工具" |

---

## 五、后端桩代码所在的位置（对前端透明，但功能不可用）

| 文件 | 受影响命令 | 每行一句 |
|-----|-----------|---------|
| `commands/ai_commands.rs` (10行) | `ai_sync_status`/`trigger`/`diagnose`/`reset`/`retry_failed`/`force_resync`/`diagnose_failed_files`/`knowledge_pipeline_status` — 全部返回假数据 |
| `commands/kb_commands.rs` (多处) | `kb_search`/`kb_import_file`/`kb_save_note`/`kb_delete`/`kb_list_documents`/`kb_chat`/`kb_diagnose` 等返回空或假数据 | 仅 `kb_agent_chat` 和 `ext_tool_search_knowledge_base` 有真实实现 |

---

## 六、建议优先修复项

1. **修复 store/api 调用名不匹配**（4 处运行时错误）— 立即影响项目管理、产品物料、文件上传功能
2. **将 knowledge_pipeline.rs 与 ai_commands.rs 对接** — ai_commands 仍是桩，knowledge_pipeline 已有完整实现但未注册到命令
3. **补全 KnowledgeGraph/KBI Inspector 页面** — 两个占位页目前只有静态文字
4. **在 Settings 页面增加 AI 同步/打标管理面板** — 后端有 pipeline 但前端无监控入口
5. **补全 kb_commands.rs 桩代码** — kb_search 等核心功能需接入 knowledge_pipeline
