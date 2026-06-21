# ArkMind 知识库设计报告

**版本**: v10.0.1  
**日期**: 2026-06-17

---

## 一、系统概览

ArkMind 知识库是一个完整的 RAG (Retrieval-Augmented Generation) 系统，集成在 Tauri 桌面应用中。核心能力：

- **多源数据采集**: 笔记、项目、产品、文件、待办五种业务实体自动入库
- **双层知识块**: parent/child 层级设计，兼顾上下文完整性和检索精度
- **混合检索**: FTS5 关键词匹配 + LanceDB 向量语义检索 + RRF 融合
- **Agent 工作流**: ReAct 循环 + 4 种工具调用，自主决策检索策略
- **突触演化**: 检索命中权重动态调整，高频知识自动提升排名

---

## 二、数据架构

### 2.1 表结构关系

```
┌─────────────────────────────────────────────────────────┐
│                    业务实体层                              │
│  projects / products / notes / file_assets / todos       │
└───────────────────────┬─────────────────────────────────┘
                        │ SQL 触发器 (AFTER INSERT/UPDATE/DELETE)
                        ▼
┌─────────────────────────────────────────────────────────┐
│                    统一源数据层                            │
│  source_data (id, source_type, source_id, title, content)│
│  UNIQUE(source_type, source_id)                          │
└───────────────────────┬─────────────────────────────────┘
                        │
            ┌───────────┼───────────┐
            ▼           ▼           ▼
    ┌──────────┐ ┌──────────┐ ┌──────────────┐
    │知识块层   │ │全文检索层 │ │ 实体图谱层    │
    │knowledge_│ │global_   │ │ entities     │
    │chunks    │ │search_fts│ │ entity_      │
    │(parent/  │ │(FTS5)   │ │ mentions     │
    │ child)   │ │          │ │              │
    └────┬─────┘ └──────────┘ └──────────────┘
         │
    ┌────┼────┐
    ▼         ▼
┌───────┐ ┌──────────┐
│向量缓存│ │ LanceDB  │
│vector_│ │knowledge_│
│cache  │ │vectors   │
└───────┘ └──────────┘
```

### 2.2 核心表详解

#### source_data — 统一源数据表

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键 |
| `source_type` | TEXT | NOTE / FILE / PRODUCT / PROJECT / TODO / PROJECT_DYNAMICS |
| `source_id` | TEXT | 业务实体 ID |
| `title` | TEXT | 实体标题 |
| `content` | TEXT | 清洗后的全文 |
| `UNIQUE(source_type, source_id)` | | 一个实体只有一条记录 |

**写入时机**: 由 `source_doc_sync::sync_entity_to_source_docs()` 写入，每当业务实体发生增删改时触发。

#### knowledge_chunks — 双层知识块

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键，也是 LanceDB 的 rowid |
| `chunk_id` | TEXT | 业务标识 |
| `chunk_type` | TEXT | `parent` (600-1800字符) / `child` (150-350字符) |
| `parent_id` | TEXT | child → parent.chunk_id；parent = NULL |
| `source_data_id` | INTEGER | parent → source_data.id；child = NULL |
| `content` | TEXT | 文本内容 |
| `doc_title` | TEXT | 文档标题 |
| `chunk_index` | INTEGER | 在源文档/父块中的顺序 |
| `content_hash` | TEXT | 去重用 |
| `base_weight` | REAL | 初始语义位能 (默认 1.0) |
| `last_access_at` | INTEGER | 上次被 RAG 命中的 Unix 时间戳 |
| `access_count` | INTEGER | 累计命中次数 |

#### vector_cache — 向量标记表

| 字段 | 类型 | 说明 |
|------|------|------|
| `chunk_id` | INTEGER PK | knowledge_chunks.id |

#### FTS5 虚拟表

| 表名 | 数据来源 | 用途 |
|------|----------|------|
| `kk_chunks_fts` | knowledge_chunks (child-only) | 知识块全文检索 |
| `global_search_fts` | source_data (external content) | 全局统一搜索 |

#### 实体图谱

| 表 | 用途 |
|----|------|
| `entities` | 实体定义 (name, type, aliases, description) |
| `entity_mentions` | 实体在 chunk/source 中的引用关系 |

#### 管线队列

| 表 | 用途 |
|----|------|
| `ai_sync_queue` | 同步队列 (source_table, source_id, is_embedded, retry_count) |
| `ai_tag_queue` | 标签队列 (source_type, source_id, status) |

---

## 三、数据采集流程

### 3.1 文件入库

```
用户上传文件
    │
    ▼
upload_file_core()
    │ 1. SHA-256 去重 → file_assets (content_hash)
    │ 2. 物理存储 → 文件系统
    │ 3. 建立关系 → file_relations (entity_type, entity_id)
    │ 4. 文件解析 → file_parser::parse_file_smart()
    │ 5. 同步源数据 → file_sync_to_source_data()
    │ 6. 入队 AI 标签 → ai_tag::enqueue_tag_task()
    ▼
source_data + ai_sync_queue
```

### 3.2 文件解析器 (`file_parser.rs`)

| 格式 | 解析方式 | 优先级 |
|------|----------|--------|
| PDF | 视觉模型 OCR → pdf-extract 兜底 | OCR 优先 |
| 图片 (PNG/JPG/WEBP/BMP) | 视觉模型 OCR | 唯一路径 |
| DOCX | XML 提取 (mammoth) | |
| XLSX/XLS | calamine 表格解析 → KV 转换 | |
| PPTX | XML slide 提取 | |
| TXT/MD/CSV/JSON | 直接读取 | |

### 3.3 业务实体同步 (`source_doc_sync.rs`)

业务实体变更 → `sync_entity_to_source_docs(type, id, title, content)`:
1. 清洗内容
2. INSERT OR REPLACE → `source_data`
3. 触发防抖物化
4. 入队 AI 标签任务

### 3.4 SQL 触发器链

```
DELETE FROM notes/products/projects/todos
    ↓ trigger (sd_notes_delete / sd_products_delete / ...)
    DELETE FROM source_data
    ↓ trigger (sd_chunks_delete)
    DELETE FROM knowledge_chunks
    ↓ trigger (sd_fts_delete)
    DELETE FROM global_search_fts
```

---

## 四、知识管线 (Knowledge Pipeline)

### 4.1 八步闭环管线

```
Step 1: 去重上传 (SHA-256 file_assets)
    │
Step 2: 触发点火 (SQL INSERT 触发器 → ai_sync_queue + ai_tag_queue)
    │
Step 3: 控制平面 (许可证检查 + API 就绪 + 队列健康)
    │
Step 4: 全局标签 (全文 → LLM NER → 3-5 标签)
    │
Step 5: 语义物化 + 智能分块
    │
Step 6: 双存储 (knowledge_chunks → FTS5 + LanceDB)
    │
Step 7: 突触强化 (RAG 命中 → lerp toward MAX=5.0, R=0.3)
    │
Step 8: 人机审计 (project_risk 扫描 → system_notifications)
```

### 4.2 智能分块 (`smart_chunker.rs`)

```
source_data.content
    │
    ▼
parse_blocks()           → 按标题/段落/表格拆分为原始块
    │
    ▼
build_sections()         → 合并短块、保留表格结构
    │
    ▼
accumulate_adaptive_parents()  → 累积到 MAX_PARENT=1800 字符
    │                          → 表格 → KV 对格式
    ▼
split_into_focused_heading_aware()  → 按标题感知切分
                                   → FOCUSED_TARGET=350 字符
    ▼
Parent Chunks (600-1800 char) → knowledge_chunks (chunk_type='parent')
Child Chunks (150-350 char)   → knowledge_chunks (chunk_type='child')
```

### 4.3 向量化流程 (`hybrid_knowledge.rs`)

```rust
vectorize_new_chunks(batch_size)
    │
    ├─ SELECT child chunks NOT IN vector_cache
    │
    ├─ embed_texts_with_caller(texts.take(150), "vectorize")
    │   └─ OpenAI-compatible /v1/embeddings API
    │   └─ 默认模型: text-embedding-3-small (1024 维)
    │
    ├─ lancedb_client::insert_vectors_full()
    │   └─ LanceDB table: knowledge_vectors
    │   └─ Schema: rowid, vector[FP32 1024], parent_id, doc_id, content_hash, tags
    │
    └─ INSERT vector_cache (标记已完成)
```

### 4.4 突触演化机制 (Synapse)

检索命中时：
```
new_weight = lerp(old_weight, MAX_WEIGHT=5.0, R=0.3)
```

- `base_weight` 初始 1.0，每次命中向 5.0 靠近
- 高权重 chunk 在检索中排名更高（演化式记忆）
- `access_count` 递增，`last_access_at` 更新

---

## 五、检索消费流程

### 5.1 Agent 工作流 (`agent_workflow.rs`)

```
用户输入 query
    │
    ▼
ReAct Loop (最多 maxIterations 轮)
    │
    ├─ Tool 1: ext_tool_search (全局全文检索)
    │   ├─ RAG Interceptor: 标签/领域 → file_id 过滤
    │   ├─ FTS5 检索 (kk_chunks_fts)
    │   ├─ LanceDB 向量检索 (RRF 融合)
    │   └─ 返回: chunk 内容 + 来源 + 相关性分数
    │
    ├─ Tool 2: ext_tool_object (结构化对象查询)
    │   └─ 搜索产品/项目/文件/笔记 → 返回完整结构化记录
    │
    ├─ Tool 3: ext_tool_traverse (图谱漫游)
    │   ├─ next/prev: 翻页
    │   ├─ related: 关联知识
    │   └─ references: 引用实体
    │
    └─ Tool 4: ext_tool_act (执行操作)
        └─ 创建待办、更新项目状态
    
    ▼
LLM 生成最终回答 (Markdown)
    │
    ▼
返回: { answer, tool_log, context_docs, workspace }
```

### 5.2 混合检索策略

```
用户 query
    │
    ├─ FTS5 全文检索 (kk_chunks_fts)
    │   └─ 关键词匹配 + BM25 排序
    │
    ├─ LanceDB 向量检索
    │   └─ query → embedding → cosine similarity
    │   └─ 可选: tag 过滤 (通过 rag_interceptor 翻译)
    │
    └─ RRF (Reciprocal Rank Fusion) 融合
        └─ score = Σ 1/(k + rank_i) 合并两路结果
```

### 5.3 RAG Interceptor (`rag_interceptor.rs`)

LLM 搜索请求携带业务过滤时：

```
输入: domain, tag_names, project_ids, product_ids
    │
    ├─ 标签翻译: tag_mappings + tags → file_id[]
    ├─ 项目翻译: file_relations(entity_type='project') → file_id[]
    ├─ 产品翻译: file_product_mapping → file_id[]
    └─ 领域翻译: domain='library' → library_file_locations → file_id[]
    
输出: file_id[] → 传入 FTS5/LanceDB WHERE 过滤
```

---

## 六、会话记忆机制

### 6.1 Agent Session Memory

```rust
struct SessionMemory {
    active_objects: Vec<ActiveObject>,   // LRU, max 20
    visited_chunks: HashSet<String>,     // 已访问的 chunk_id
    visited_sources: HashSet<String>,    // 已访问的 source_id
    focus: Focus,                        // 当前焦点
    observations: Vec<String>,           // 观察记录
    hypotheses: Vec<String>,             // 假设
    current_goal: String,                // 当前目标
    scratchpad: String,                  // 工作笔记 (max 500 chars)
}
```

### 6.2 前端会话持久化

```
KnowledgeBase.vue
    ├─ kbSaveSessions(sessions) → Tauri IPC → SQLite 存储
    ├─ kbLoadSessions()         → Tauri IPC → 恢复会话
    └─ 每次消息变更自动调用 saveToStorage()
```

---

## 七、删除链路

### 7.1 统一删除函数

```rust
pub fn cleanup_kb(source_type: &str, source_id: &str) {
    // 1. 收集 chunk_ids
    // 2. DELETE source_data → 触发器 → knowledge_chunks + FTS5
    // 3. DELETE vector_cache WHERE chunk_id IN (...)
    // 4. LanceDB delete_chunks_by_rowids(...)
}
```

### 7.2 业务删除路径

| 实体 | 删除路径 | cleanup_kb 调用 |
|------|----------|----------------|
| Note | notes_permanent_delete → cleanup_kb("NOTE", ...) | ✅ |
| Product | product_delete → cleanup_kb("PRODUCT", ...) | ✅ |
| Project | project_delete → cleanup_kb("PROJECT", ...) | ✅ |
| Todo | todos_delete → cleanup_kb("TODO", ...) | ✅ |
| File | file_delete → cleanup_kb_for_file → cleanup_kb("FILE", ...) | ✅ |

---

## 八、配置体系

### 8.1 AI 配置 (system_config.ai_config)

```json
{
  "providers": [{ "id", "name", "apiKey", "baseUrl", "modelName" }],
  "activeProviderId": "...",
  "visionProviders": [{ "id", "apiKey", "baseUrl", "modelName", "apiFormat", "prompt" }],
  "activeVisionId": "...",
  "embedding": { "baseUrl", "apiKey", "modelName" }
}
```

- API Key 使用 AES-256-GCM 加密存储（机器码绑定）
- 嵌入模型变更时自动清空向量库并重新向量化

### 8.2 技术依赖

| 组件 | 技术 | 用途 |
|------|------|------|
| 向量数据库 | LanceDB (本地) | 向量存储与检索 |
| 嵌入模型 | OpenAI-compatible API | 文本向量化 |
| LLM | OpenAI-compatible API | 对话生成 |
| 视觉模型 | GPT-4o / Qwen-VL / DeepSeek-OCR | 文档 OCR |
| 全文检索 | SQLite FTS5 | 关键词检索 |
| 文本分块 | text-splitter crate | 智能分块 |

---

## 九、数据流全景图

```
┌─────────────────────────────────────────────────────────┐
│                        用户交互层                        │
│  上传文件 / 创建笔记 / 编辑产品 / 更新项目 / 管理待办     │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                     数据采集层                           │
│  SQL 触发器 → ai_sync_queue                             │
│  source_doc_sync → source_data                          │
│  file_parser → source_data + file_assets                │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                     知识管线层                           │
│  knowledge_pipeline (8步)                               │
│  ├─ ai_tag: 全文 → LLM NER → 标签/实体                 │
│  ├─ smart_chunker: 全文 → parent/child 双层块           │
│  ├─ embedding_client: 文本 → 1024维向量                 │
│  └─ lancedb_client: 向量 → LanceDB 存储                │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                     检索消费层                           │
│  Agent ReAct Loop                                       │
│  ├─ ext_tool_search: FTS5 + LanceDB RRF 混合检索        │
│  ├─ ext_tool_object: 结构化查询                         │
│  ├─ ext_tool_traverse: 图谱漫游                         │
│  └─ ext_tool_act: 操作执行                              │
│                                                         │
│  RAG Interceptor: 标签/领域/项目/产品 → file_id 过滤    │
│  Synapse: 命中 → base_weight 演化 (1.0 → 5.0)          │
│  Session Memory: 会话级工作记忆 (LRU + scratchpad)       │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                     输出层                              │
│  LLM 生成 Markdown 回答                                │
│  前端: 打字机渲染 + 工具调用卡片 + 上下文面板 + 会话持久化│
└─────────────────────────────────────────────────────────┘
```

---

## 十、设计亮点

1. **双层块架构**: parent/child 分离，检索命中 child 后自动关联 parent 获取完整上下文
2. **混合检索**: FTS5 关键词 + LanceDB 向量 + RRF 融合，关键词和语义互补
3. **突触演化**: `base_weight` 动态调整，高频知识自动提升排名
4. **Agent 工具化**: 4 个工具覆盖搜索/查询/图谱/操作，ReAct 循环自主决策
5. **会话记忆**: LRU + scratchpad，避免重复检索
6. **管线闭环**: 从数据采集到向量化到检索，8 步自动化
7. **统一删除**: `cleanup_kb()` 函数确保所有知识实体删除时完整清理源数据、知识块、向量缓存和 LanceDB
8. **API Key 加密**: AES-256-GCM + 机器码绑定，模型变更自动清空重建向量库
