# ArkMind 知识库审计报告

**审计日期**：2025-06-18  
**范围**：`src-tauri/src/services/agent_workflow.rs`、`knowledge_pipeline.rs`、`ai_sync.rs`、`database.rs`、`file_commands.rs`、`lib.rs`、`reranker_client.rs`  
**审计类型**：代码级功能审计（边界情况、死代码、stub、数据完整性）

---

## 一、Agent 工作流 (agent_workflow.rs)

### 1.1 工具定义完整

| 工具 | 定义 | 实现 | 调度 |
|------|:---:|:---:|:---:|
| `ext_tool_search` | ✅ | `execute_search_knowledge` | ✅ |
| `ext_tool_object` | ✅ | `execute_object_get` | ✅ |
| `ext_tool_traverse` | ✅ | `execute_traverse` | ✅ |
| `ext_tool_act` | ✅ | `execute_act` | ✅ |

**结论**：4 个工具的完整链路（定义 → 实现 → ReAct 调度）无缺失。

### 1.2 死码函数（3 个）

| 函数 | 行 | 说明 |
|------|----|------|
| `execute_search_by_tag(_tag_name)` | :873 | 返回空字符串，从未被调用 |
| `execute_get_doc_page(_doc_id, _page)` | :874 | 同上 |
| `execute_find_related(_doc_id)` | :875 | 同上 |

这三个函数未暴露为工具定义，也未在 ReAct match 臂中出现。

### 1.3 SessionMemory 无上限

| 字段 | 类型 | 界限 |
|------|------|------|
| `visited_chunks` | `HashSet<String>` | ❌ 无上限 |
| `visited_sources` | `HashSet<String>` | ❌ 无上限 |
| `search_keywords` | `Vec<Vec<String>>` | ❌ 无上限 |
| `active_objects` | `Vec<ActiveObject>` | ✅ LRU 衰减 (阈值 20) |
| `observations` | `Vec<String>` | ✅ 截断到 5 条 |
| `scratchpad` | 生成文本 | ✅ 截断到 500 字符 |

**风险**：长时间会话中 `visited_chunks` 和 `visited_sources` 会无限增长，可能影响内存和遍历性能。

### 1.4 ReAct 回路断路器（3 种）

1. **Stale rounds**（:1310）— iter ≥ 4 时连续两轮结果 <50 字符 → 停止
2. **关键词循环检测**（:1321）— Jaccard 相似度 > 0.6 → 停止
3. **无新 ChunkID**（:1343）— search 没有新 chunk → 停止

断路器触发后调用 `call_llm_without_tools()` 强制执行总结，不直接返回部分结果。

### 1.5 search 空结果处理

- `fused.is_empty()` → 返回诊断信息 + 三条建议（:754）
- `out.trim().is_empty()` + `obj_head.is_empty()` → 返回类似诊断（:827-831）

### 1.6 traverse 边界检查

- 连续遍历同一文件 > 5 次 → 警告（:923）
- 到达文件首/尾 → 明确提示（:928-934）
- 已访问 chunk 显示 `⏭ 已在前轮展示` 而非隐藏（:907）
- radius 上限 20（:884）

---

## 二、知识管线 (knowledge_pipeline.rs + ai_sync.rs)

### 2.1 启动流程（8 步）

1. 清理僵尸队列
2. 删除旧的 ai_sync_products trigger
3. 种子同步队列
4. 重置可重试失败
5. 修复重复任务 → **`heal_duplicate_tasks()` 是空操作**（:134-137）
6. 清理孤立 vector_cache
7. 自动清理已删除超过 30 天的笔记
8. 持久化 WAL

### 2.2 materialize_task 处理类型

| 来源表 | 已处理 | 备注 |
|--------|:---:|------|
| `files` | ✅ | — |
| `notes` | ✅ | < 2000 字符跳过 |
| `projects` | ✅ | — |
| `todos` | ✅ | — |
| `products` | ❌ | 回退到 `_ => empty`（:256） |

### 2.3 关键发现：笔记长度阈值不一致

- `ai_sync_queue` 触发器：`LENGTH(...) > 1000` 时入队（database.rs:823）
- `materialize_task`：`content.len() < 2000` 时跳过（ai_sync.rs:241）
- **影响**：1000-2000 字符的笔记被反复出队又丢弃，标记出队但不产生 chunk

### 2.4 关键发现：`documents_parent.id` 和 `knowledge_chunks.parent_id` 连接断裂

- `documents_parent.id` 存储 `"p_{uuid}"`（前缀）（ai_sync.rs:170）
- `knowledge_chunks.parent_id` 存储 `"{uuid}"`（无前缀）
- `batch_get_parents_by_chunks` 中的 JOIN（database.rs:1411）和 `synapse.rs:139` 中的连接永远不会匹配
- **影响**：父块检索路径完全不工作

### 2.5 调度器循环

- 50ms tick，有任务 = 50ms sleep，无任务 = 5s sleep
- 每次处理 5 个 sync 任务（6 秒超时）+ 5 个向量任务
- 每约 30 秒增量种子，每约 2 分钟完整种子
- Lancedb 搜索异步超时 6 秒（:213）

### 2.6 防抖物化器（debounced_sync.rs）

- 5 秒防抖窗口
- 最大 4 个并发
- 在实体源内容稳定前抑制重复物化

---

## 三、数据库触发器与数据模型 (database.rs)

### 3.1 source_data 触发器覆盖率

| 实体 | INSERT | UPDATE | DELETE |
|------|:---:|:---:|:---:|
| notes | ✅ | ✅ | ✅ |
| products | ✅ | ✅ | ✅ |
| projects | ✅ | ✅ | ✅ |
| todos | ✅ | ✅ | ✅ |
| files | ❌ (无 trigger) | ❌ | ❌ (已删除) |

文件通过 `file_sync_to_source_data()` 显式处理（:1388）。

### 3.2 关键发现：notes INSERT/UPDATE trigger 被立即删除

`init_struct_source_data_triggers` 创建 `sd_notes_insert/update`（:438-447），但 `drop_source_doc_insert_triggers`（:422-428）在 `init_ai_queue_triggers` 之后立即删除它们。注释说 notes INSERT/UPDATE 由 `source_doc_sync` 模块接管——需要验证。

### 3.3 ai_sync_queue 自动入队触发器

| 表 | INSERT | UPDATE |
|----|:---:|:---:|
| `file_assets` | ✅ | ✅ |
| `notes` | ✅（>1000 字符） | ✅（>1000 字符，5 分钟冷却） |
| `projects` | ✅ | ✅ |
| `todos` | ✅ | ✅ |
| `products` | ❌ 缺失 | ❌ 缺失 |

products 有 `ai_tag_queue` 触发器但没有 `ai_sync_queue` 触发器——按设计如此（产品不会被分块）。

### 3.4 过时队列清理：

- `database.rs::purge_stale_queue_entries`：清理所有 6 个类型（:1311-1329）
- `ai_sync.rs::purge_stale_queue_entries`：仅清理 3 个类型（:433-439），缺少 products、todos、project_dynamics
- 管道版本（ai_sync.rs）在启动时调用，数据库版本在 init_database 中调用

### 3.5 缺失的 ai_tag_queue 触发器

todos 的 `ai_tag_queue` 有 INSERT 和 UPDATE 触发器；`seed_tag_queue` 也未包含 todos。

### 3.6 迁移风险

- **破坏性 DROP+RECREATE**：如果 `chunk_type` 列缺少，`knowledge_chunks` 被整个删除重建（:1123-1149），数据丢失
- **静默去重**：`idx_products_name` UNIQUE 约束强制每个名称一个产品（:1219-1236），同名不同系列的产品以最低 rowid 为主
- `ai_sync_queue` 的 `is_embedded` 列上**缺少索引**——调度器每 tick 全表扫描

---

## 四、文件系统 (file_commands.rs + database.rs)

### 4.1 上传流程

`upload_file_core()`（:186-245）：
1. SHA-256 哈希 → 内容寻址复用（CAS）
2. `flat_disk_path(hash)` → copy
3. INSERT `file_assets`
4. INSERT `file_relations(file_id, entity_type, entity_id, custom_name)`

重构后无 `file_objects` 或 `file_versions`，`file_id` 直接指向 `file_assets.id`。

### 4.2 文件元数据更新 Bug（已修复）

`file_update_meta()` 中的拖拽文件夹写入（:520）：之前 `WHERE id=?` 使用 file_assets.id，但 `file_relations.id` 是不同的 UUID。已修复为 `WHERE id=?`，其中 id 是关系行的 UUID。

### 4.3 cleanup_kb

统一清理（:9-37）：收集 chunk_ids → 删除 entity_mentions → FTS5 删除 → 删除 knowledge_chunks → 重建触发器 → 删除 source_data → 删除 vector_cache → LanceDB。自包含且幂等。

---

## 五、Stub 和死代码

### 5.1 Stub Tauri 命令（27 个）

返回硬编码成功但无实际实现的已注册命令：

- **Export/Import**：`export_data`、`import_data`、`db_export_raw`、`products_export`、`products_export_file`、`products_import`、`products_import_file`（export_commands.rs:2-8）
- **项目 Export/Import**：`project_export`、`project_import`、`project_import_file`、`project_export_files`、`projects_batch_export`（project_commands.rs:64-68）
- **Storage/OSS**：`storage_update_config`、`storage_migrate`、`storage_migrate_progress`、`storage_default_path`、`oss_test_connection`、`oss_raw_diagnose`（settings_commands.rs:145-150）
- **Semantic/Graph**：`notes_rebuild_fts`、`notes_upgrade_fts_triggers`、`notes_get_project_knowledge_graph`、`notes_semantic_recall`（notes_commands.rs:109-113）
- **Production RAG**：`execute_production_rag`（kb_commands.rs:137）
- **文件遗留**：`file_get_path`、`file_has_oss_files`、`file_migrate_files`、`file_cleanup_if_orphan`（file_commands.rs:515-541）

### 5.2 死模块和死代码（10 个）

| 代码 | 文件 | 说明 |
|------|------|------|
| `reranker_client.rs` | services/mod.rs:14 | 整个模块未使用；`is_reranker_available()` 始终返回 false |
| `OssClient` 结构体 | oss.rs:2-3 | 标记为 "stub"，从未被实例化 |
| `HtmlToMarkdownTransformer` | data_pipeline.rs:3-4 | 已定义但从未被实例化 |
| `DataPipeline::process_file_stream` | data_pipeline.rs:7 | 传入文本原样返回 |
| `execute_search_by_tag` | agent_workflow.rs:873 | 返回空，未被调用 |
| `execute_get_doc_page` | agent_workflow.rs:874 | 返回空，未被调用 |
| `execute_find_related` | agent_workflow.rs:875 | 返回空，未被调用 |
| `repair_file_product_mapping_fk` | database.rs:912 | 空函数体 |
| `repair_empty_file_ext` | database.rs:913 | 空函数体 |
| `fix_assets_fts_schema` | database.rs:1054 | 空函数体 |

`#![allow(dead_code)]`（lib.rs:2）抑制了所有死代码编译警告。

### 5.3 无 TODO/FIXME/BUG 注释

整个代码库中未找到 `// TODO`、`// FIXME`、`// HACK` 或 `// BUG` 注释。唯一相关的注释是 `oss.rs:1` 中的 `// OSS client stub` 和 `file_commands.rs:512` 中的 `// ── Stubs / legacy compat ──`——两者都是部分的存根标记。

---

## 六、关键发现汇总

| # | 严重性 | 位置 | 问题 |
|---|:---:|------|------|
| 1 | **HIGH** | database.rs:1411, synapse.rs:139 | `documents_parent.id` ↔ `knowledge_chunks.parent_id` 连接断裂（前缀不匹配） |
| 2 | **HIGH** | database.rs:438-447 vs :422-428 | Notes INSERT/UPDATE 的 source_data 触发器被立即删除——需验证 source_doc_sync 是否真的处理了 |
| 3 | **MEDIUM** | ai_sync.rs:241 vs database.rs:823 | 笔记长度阈值不匹配：触发器 1000 字符，materialize_task 2000 字符 |
| 4 | **MEDIUM** | ai_sync.rs:433-439 | 管道版本的 `purge_stale_queue_entries` 缺少 products、todos、project_dynamics |
| 5 | **MEDIUM** | database.rs | ai_sync_queue 的 `is_embedded` 列无索引——每 tick 全表扫描 |
| 6 | **LOW** | lib.rs:2 | `#![allow(dead_code)]` 抑制了 27 个 stub 命令和 10 个死代码模块的警告 |
| 7 | **LOW** | knowledge_pipeline.rs:134-137 | `heal_duplicate_tasks()` 是空操作 |
| 8 | **LOW** | agent_workflow.rs:81-88 | `visited_chunks` 和 `visited_sources` 无上限——长时间会话中内存增长 |
| 9 | **LOW** | database.rs:1219-1236 | `idx_products_name` UNIQUE 约束静默去重同名产品 |
| 10 | **LOW** | database.rs:1123-1149 | `knowledge_chunks` DROP+RECREATE 迁移——若错误触发会破坏数据 |
| 11 | **LOW** | services/reranker_client.rs | 整个模块未使用，重排序完全依赖 RRF |
