# PMS Desktop 代码库审计报告
> 生成时间：2026-06-05  
> 对比基准：上次会话的备份报告

---

## 一、后端 Rust 代码（src-tauri/src/）

### 1.1 已确认存在的文件（14 commands + 15 services + 6 core）

**核心文件：**
- `main.rs` — 入口，一行调用 `arkmind_lib::run()`
- `lib.rs` — Tauri 应用，~177 行，151+ 个命令注册
- `database.rs` — SQLite schema（35+ 表）、FTS5、AI 队列触发器
- `license.rs` — License 校验
- `oss.rs` — OSS 对象存储
- `time_utils.rs` — 时间工具（新增，报告未提及）

**Commands（14 个文件）：**
`db_commands.rs`、`license_commands.rs`、`project_commands.rs`、`product_commands.rs`、`file_commands.rs`、`notes_commands.rs`、`kb_commands.rs`、`ai_commands.rs`、`asset_commands.rs`、`settings_commands.rs`、`partner_commands.rs`、`export_commands.rs`、`window_commands.rs` + `mod.rs`

**Services（15 个文件）：**
`ai_sync.rs`、`ai_tag.rs`、`file_parser.rs`、`hybrid_knowledge.rs`、`knowledge_pipeline.rs`、`search_assets.rs`、`embedding_client.rs`、`lancedb_client.rs`、`chat_api.rs`、`smart_chunker.rs`、`data_pipeline.rs`、`reranker_client.rs`、`synapse.rs`、`agent_workflow.rs` + `mod.rs`

### 1.2 与备份报告的差异（重要架构变更）

| 备份报告中的文件 | 当前状态 | 说明 |
|---|---|---|
| `services/ai_tag_new.rs` | **不存在** | 可能已被合并到 ai_tag.rs |
| `services/ai_sync_new.rs` | **不存在** | 已被 knowledge_pipeline.rs 替代 |
| `services/rag_parent_tracker.rs` | **不存在** | RAG Parent-Child 逻辑已集成到 ai_sync.rs + knowledge_pipeline.rs |
| `services/agent_chat.rs` | → `agent_workflow.rs` | 重命名，Agent Chat 逻辑迁入 |
| `services/table_chunker.rs` | → `smart_chunker.rs` | 重命名，自适应分块引擎 |
| `services/ocr.rs` | **不存在** | OCR 功能可能未实现 |
| `services/pdf_table.rs` | **不存在** | PDF 表格提取未独立实现 |
| `services/asset_vector.rs` | **不存在** | 向量同步已合并到 ai_sync.rs |
| `services/object_ingestion.rs` | **不存在** | 知识对象抽取未独立实现 |
| `services/circuit_breaker.rs` | **不存在** | 熔断器未独立实现 |
| `services/notes.rs` | **不存在** | 笔记业务逻辑直接在 commands 中 |

### 1.3 新增文件（报告未提到）

| 文件 | 用途 |
|---|---|
| `time_utils.rs` | `now_db()` 和 `now_utc_rfc3339()` 时间工具 |
| `knowledge_pipeline.rs` | 8 步闭环 AI 数据管线编排器 |
| `data_pipeline.rs` | 数据清理/转换管道 |
| `smart_chunker.rs` | 自适应 Parent→Child 分块 |
| `reranker_client.rs` | Reranker 客户端 |
| `synapse.rs` | 神经突触强化（hit_count + neural_energy） |
| `agent_workflow.rs` | Agentic RAG 工作流 |

### 1.4 Schema 重大变化

| 表名 | 变化 |
|---|---|
| `files` → `file_assets` | 表重命名，新增 neural_energy、tags、hit_count 字段 |
| `file_versions` → 保留 | 仍使用 |
| `knowledge_chunks` | `parent_id` 从 TEXT(UUID) 改为 INTEGER（引用 documents_parent.id） |
| `documents_parent` | 新增表：600-1200 char 大块存储 |
| `kk_chunks_fts` | 新增 tags 列，对应 FTS5 content + tags 双字段索引 |
| `content_fts` | 仍存在，传统 FTS5（doc_id + source_table + title + content） |
| `assets_fts` | 新增：对 file_assets 的 FTS5 全文索引 |
| `file_tags_mapping` / `product_tags_mapping` / `project_tags_mapping` / `material_tags_mapping` | 新增：打标多对多关联表 |
| `knowledge_objects` / `knowledge_objects_fts` | 新增：知识对象抽取 |
| `system_notifications` | 新增：系统通知 |
| `solutions` / `solution_versions` / `solution_items` | 新增：方案管理 |

### 1.5 ai_commands.rs 退化为桩代码

当前 `ai_commands.rs`（10 行）全部返回假数据：
```rust
pub async fn ai_sync_status() -> Result<Value, String> { Ok(json!({"running":false})) }
```
实际逻辑已迁移到 `knowledge_pipeline.rs`，但它不在 Tauri 命令注册中——`lib.rs` 注册了 `knowledge_pipeline_status` 命令但也是个桩。

**这是严重问题：AI 同步/打标的前端按钮全部失效。**

### 1.6 kb_commands.rs 部分桩代码

`kb_search`、`kb_import_file`、`kb_save_note`、`kb_delete`、`kb_list_documents` 等命令返回空数据。只有 `kb_agent_chat`（调用 `agent_workflow`）和 `ext_tool_search_knowledge_base`（完整混合检索管线）有真实实现。

---

## 二、前端 Vue 代码（src/renderer/src/）

### 2.1 视图页面（11 个）

`Dashboard.vue`、`ProjectHub.vue`、`ProjectDetail.vue`、`ProductCenter.vue`、`NotesPage.vue`、`AssetLibrary.vue`、`KnowledgeGraph.vue`（占位）、`KnowledgeInspector.vue`（占位）、`KnowledgeBase.vue`、`KnowledgeLayout.vue`、`Settings.vue`

### 2.2 路由配置（router/index.js）

全部路由已配置，共 9 条路由：
`/dashboard`、`/projects`、`/projects/:id`、`/products`、`/assets`、`/notes`、`/knowledge-graph`、`/inspector`、`/knowledge/brain`

### 2.3 本轮修复

1. **ProductCenter.vue** — `<script setup>` → `<script setup lang="ts">`（TS 类型注解在 JS 模式下导致编译失败）
2. **FileManager.vue** — 同上修复
3. **Sidebar.vue** — navItems 从 4 项扩展为 8 项（新增：笔记库、资料库、知识库、智能问答），并补充 icons 映射

---

## 三、已知风险项

| 风险 | 严重程度 | 说明 |
|---|---|---|
| AI 命令为桩代码 | **高** | `ai_sync_*`、`tag_*` 等命令返回假数据，前端按钮无效 |
| KB 查询命令为桩代码 | **中** | `kb_search`、`kb_list_documents` 等无真实实现 |
| 两个占位页面 | **低** | KnowledgeGraph.vue 和 KnowledgeInspector.vue 只有静态文字 |
| knowledge_pipeline.rs 不在命令注册中 | **中** | 管线状态命令 `knowledge_pipeline_status` 是桩代码 |
| 无 OCR 模块 | **低** | ocr.rs、pdf_table.rs 不存在，PDF 图片内容无法识别 |
