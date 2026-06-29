# ArkMemory 独立化方案

## 目标

将知识库/AI 记忆体从 ArkMind 中完全分离，成为独立项目 ArkMemory，可以被任何应用接入。

---

## 当前架构

```
ArkMind（单体）
├── 业务模块（项目/产品/笔记/待办/文件）
├── 知识模块（source_data / chunks / vector / synapse / agent）
└── 共享数据库 arkmind.db（40+ 张表）
```

### 耦合分析

| 层 | 耦合程度 | 说明 |
|---|---------|------|
| 数据库 | **强** | 单一 SQLite，业务表和知识表混在一起 |
| source_data | **强** | 既是业务实体视图，又是知识库入口 |
| 同步机制 | **已解耦** | `knowledge_lifecycle.rs` 已有 `notify_changed` / `get_changed_since` 接口 |
| Agent | **强** | `agent_workflow.rs` 同时查询业务表和知识表 |
| 前端 | **中等** | `kb_commands.rs` 和业务 commands 分开，共享 Tauri invoke |

### 好消息

`knowledge_lifecycle.rs` 已经是一个干净的接口层：

```rust
// 业务层 → 知识库
notify_changed(source_type, source_id)   // 通知变更
notify_deleted(source_type, source_id)   // 通知删除

// 知识库 ← 业务数据
get_changed_since(since, limit)          // 拉取变更列表
```

分离的"契约"已经存在。

---

## 目标架构

```
ArkMind（业务）                    ArkMemory（知识/AI 记忆体）
├── 项目/产品/笔记/待办/文件管理     ├── source_data / knowledge_chunks / vector_cache
├── 业务 CRUD                      ├── object_links / synapse（注意力系统）
├── 调用 ArkMemory HTTP API         ├── embedding / FTS5 / LanceDB
└── 不再直接访问知识表               ├── agent_workflow（AI 对话）
                                    ├── knowledge_pipeline（同步管线）
                                    ├── MEMORY / POLICY / PERSON 等认知对象
                                    └── 暴露 HTTP API
```

---

## 实现路径：本地 HTTP 服务

ArkMemory 作为独立进程运行，暴露 HTTP API。ArkMind 通过 HTTP client 调用。

```
ArkMind ──HTTP──▶ ArkMemory (localhost:9800)
                    ├── 独立 SQLite
                    └── 独立 LanceDB
```

### 为什么选 HTTP 而不是嵌入式 SDK

| 方案 | 优点 | 缺点 |
|------|------|------|
| 嵌入式 SDK | 零网络开销 | 同一进程，不够独立 |
| **HTTP 服务** | **完全独立、语言无关、可被其他应用接入** | **localhost 开销可忽略** |
| WASM 插件 | 安全沙箱 | SQLite/LanceDB 在 WASM 中有限制 |

### 核心优势

1. **ArkMemory 可以成为独立产品** — 任何应用都能接入自己的"AI 记忆体"
2. **接口天然适合 HTTP 化** — `notify_changed` → POST，`get_changed_since` → GET
3. **Agent 对话、知识检索、向量化都在 ArkMemory 内部完成**
4. **未来可加认证、多用户、云端同步**

---

## ArkMemory API 设计

### 记忆接口（业务层调用）

```
POST /api/notify/object-changed
  Body: { source_type, source_id }
  Response: { success: true }

POST /api/notify/object-deleted
  Body: { source_type, source_id }
  Response: { success: true }
```

### 查询接口（检索 + 获取）

```
GET /api/query/search?q=关键词&limit=10&scope=PROJECT-xxx
  Response: { results: [{ source_type, source_id, title, chunk_text, score }] }

GET /api/query/object?type=PRODUCT&id=xxx
  Response: { source_type, source_id, title, content, links: [...] }

GET /api/query/object-links?source_id=PRODUCT-xxx
  Response: { links: [{ target_id, edge_type, weight }] }
```

### Agent 接口（AI 对话）

```
POST /api/agent/chat
  Body: { query: "介绍18500", history: [...], max_iterations: 8 }
  Response: { success: true, answer: "...", tool_log: [...], workspace: "..." }

POST /api/agent/chat/stream
  Body: { query: "...", history: [...] }
  Response: SSE stream (agent:token, agent:thought, agent:progress)
```

### 记忆接口（学习 + 溯源）

```
POST /api/memory/learn
  Body: { title: "5310主要用于PACS", content: "...", mentions: ["PRODUCT-5310"], source_chat_id: "..." }
  Response: { success: true, memory_id: "MEMORY-xxx" }

GET /api/memory/trace?source_id=MEMORY-xxx
  Response: { memory: {...}, links: [{ edge_type: "LEARN_FROM", target_id: "CHAT-xxx" }] }

GET /api/memory/stats
  Response: { total_objects: 1247, total_chunks: 8934, memory_count: 56, ... }
```

### 管线接口

```
POST /api/pipeline/sync
  Response: { synced: 12 }

POST /api/pipeline/resync
  Response: { success: true }

GET /api/pipeline/status
  Response: { running: true, last_sync: "2026-06-29T10:00:00", pending: 3 }
```

---

## ArkMemory 内部结构

```
arkmemory/
├── Cargo.toml
├── src/
│   ├── main.rs                 # HTTP Server 入口
│   ├── api/                    # HTTP 路由
│   │   ├── mod.rs
│   │   ├── notify.rs           # POST /api/notify/*
│   │   ├── query.rs            # GET /api/query/*
│   │   ├── agent.rs            # POST /api/agent/*
│   │   ├── memory.rs           # POST /api/memory/*
│   │   └── pipeline.rs         # POST /api/pipeline/*
│   ├── core/                   # 核心逻辑
│   │   ├── mod.rs
│   │   ├── source_data.rs      # source_data 表操作
│   │   ├── knowledge_chunks.rs # 切片 + FTS5
│   │   ├── vector.rs           # LanceDB 向量检索
│   │   ├── object_links.rs     # Object Link + 注意力
│   │   ├── synapse.rs          # 衰减 + 激活
│   │   └── memory_objects.rs   # MEMORY/POLICY/PERSON 创建
│   ├── agent/                  # AI Agent
│   │   ├── mod.rs
│   │   ├── workflow.rs         # ReAct 循环（从 agent_workflow.rs 迁移）
│   │   ├── tools.rs            # Tool 定义 + 执行
│   │   └── planner.rs          # Phase 1: Planner 层（未来）
│   ├── pipeline/               # 知识管线
│   │   ├── mod.rs
│   │   ├── sync.rs             # 同步循环
│   │   ├── embed.rs            # 向量化循环
│   │   └── lifecycle.rs        # notify_changed / get_changed_since
│   ├── embedding/              # Embedding 客户端
│   │   ├── mod.rs
│   │   └── client.rs
│   └── db/                     # 数据库
│       ├── mod.rs
│       ├── schema.rs           # 表定义
│       └── migrations.rs       # 迁移
├── data/                       # 运行时数据
│   ├── arkmemory.db            # 独立 SQLite
│   └── vectors/                # LanceDB 数据
└── tests/
```

### ArkMemory 独占的表

```sql
-- 记忆核心
source_data              -- 统一记忆容器
knowledge_chunks         -- 知识碎片
vector_cache             -- 向量缓存
object_links             -- 实体关系（带 edge_type）
global_search_fts        -- FTS5 全文索引

-- 认知对象
-- （都是 source_data 的 type，不需要新表）
-- MEMORY / POLICY / PERSON / SUMMARY / EXPERIENCE

-- Agent 状态
chat_sessions            -- 会话管理
chat_messages            -- 对话历史

-- 管线
ai_sync_queue            -- 同步队列
ai_tag_queue             -- 标签队列

-- 注意力
-- object_links 的 weight 字段 + synapse 衰减公式
```

### ArkMind 独占的表

```sql
projects                 -- 项目
project_dynamics         -- 项目动态
products                 -- 产品
product_series           -- 产品系列
solutions                -- 方案
solution_versions        -- 方案版本
solution_items           -- 方案条目
notes                    -- 笔记
notebooks                -- 笔记本
note_project_mapping     -- 笔记-项目映射
todos                    -- 待办
todo_project_mapping     -- 待办-项目映射
file_assets              -- 文件资产
file_relations           -- 文件关联
library_folders          -- 文库文件夹
library_file_locations   -- 文件位置
channel_partners         -- 渠道伙伴
channel_contacts         -- 渠道联系人
vendor_partners          -- 供应商
vendor_contacts          -- 供应商联系人
project_product_mapping  -- 项目-产品映射
tags                     -- 标签（业务标签）
tag_mappings             -- 标签映射
users                    -- 用户
license                  -- 许可
system_config            -- 系统配置
system_notifications     -- 系统通知
system_settings          -- 系统设置
user_policies            -- 用户策略
event_store              -- 事件存储
```

---

## 迁移步骤

### Phase 1：抽取独立 crate（3-5 天）

```
1. 创建 arkmemory/ 目录结构
2. 将以下文件迁移到 arkmemory/src/core/：
   - source_data 操作（从 database.rs 中抽取）
   - knowledge_chunks 操作
   - vector 操作（embedding_client.rs, lancedb_client.rs）
   - object_links + synapse.rs
   - smart_chunker.rs
   - search_assets.rs（RRF fusion）
3. 将 agent_workflow.rs 迁移到 arkmemory/src/agent/
4. 将 knowledge_pipeline.rs 迁移到 arkmemory/src/pipeline/
5. 将 knowledge_lifecycle.rs 作为内部模块
6. 确保 arkmemory 可以独立编译
```

### Phase 2：加 HTTP Server（2-3 天）

```
1. 用 axum 框架搭建 HTTP server
2. 实现 /api/notify/* 接口
3. 实现 /api/query/* 接口
4. 实现 /api/agent/chat 接口（含 SSE 流式）
5. 实现 /api/memory/* 接口
6. 实现 /api/pipeline/* 接口
7. 默认端口 localhost:9800
```

### Phase 3：ArkMind 迁移（2-3 天）

```
1. ArkMind 新增 ArkMemory HTTP client
2. kb_commands.rs 改为调用 HTTP API
3. 前端 agent chat 改为调用 ArkMemory stream API
4. 移除 ArkMind 中的知识库相关 service 引用
5. 业务层通过 notify_changed/notity_deleted 通知 ArkMemory
```

### Phase 4：清理（1-2 天）

```
1. 删除 ArkMind 中的知识库代码
2. 删除 arkmind.db 中的知识库表（保留业务表）
3. 更新前端，移除知识库相关页面（或重定向到 ArkMemory）
4. 测试完整流程
```

---

## 工作量估计

| 阶段 | 工作量 | 产出 |
|------|--------|------|
| Phase 1：抽取 crate | 3-5 天 | arkmemory-core 可编译 |
| Phase 2：HTTP Server | 2-3 天 | API 可用 |
| Phase 3：ArkMind 迁移 | 2-3 天 | ArkMind 通过 HTTP 调用 |
| Phase 4：清理 | 1-2 天 | 代码干净 |
| **总计** | **8-13 天** | **ArkMemory 独立运行** |

---

## 独立后的价值

### 作为独立产品

- **ArkMemory**：本地 AI 记忆体，开源
- 任何应用都能接入自己的"长期记忆"
- 支持 MEMORY / POLICY / PERSON 等认知对象
- 内置 Agent + 知识管线 + 注意力系统

### 可接入的应用

- Notion 插件
- Obsidian 插件
- 飞书/钉钉机器人
- Web 应用
- 移动端 App
- 其他桌面应用

### 独立迭代能力

- 知识管线优化（加 MCP、加 Web Search）
- Agent 能力升级（加 Planner、加 Worker）
- 向量检索优化
- 多用户支持
- 云端同步

---

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| HTTP 延迟影响体验 | localhost 延迟 <1ms，可忽略 |
| 两个进程管理 | 用系统服务或 ArkMind 自动拉起 ArkMemory |
| 数据库迁移复杂 | Phase 1 可以共享数据库，Phase 4 再分离 |
| 前端改动量大 | kb_commands 已经是独立模块，改动可控 |
