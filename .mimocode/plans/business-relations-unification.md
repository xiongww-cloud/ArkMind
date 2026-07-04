# Business Relations 统一执行计划

## 目标

将散落在多张表中的业务关联统一到 `business_relations` 表，知识层只监听一张表感知所有变化。同时将 `object_links` 从知识层剥离，归入 Memory 层（仅保留 Hebbian 经验关联）。

## 架构原则

```
ArkKnowledge                         ArkMemory
├─ Objects                           ├─ Experiences
├─ business_relations (新)            ├─ object_links (原, 仅 Hebbian)
├─ Chunks                            ├─ Recall / Decay
├─ FTS                               └─ Attention
└─ Vector
        ↑                    ↑
        └── Memory 通过 provider_id 引用 Object ──┘
```

- **Business Relation** = 事实：belongs_to / has_file / authored_by。Create→Delete，无 weight，无 decay
- **Memory Relation** = 认知：cooccurrence。Recall→Strength++，Decay→Forget

---

## Phase 0: 当前修复收尾（进行中）

**目标**：稳定当前产品物料管理功能

- [x] 修复 `storage_base()` 路径问题（已回退到 exe 同级）
- [x] 修复 `onMaterialDrop` 拖拽路径（使用 Tauri 原生 file drop）
- [x] 修复 `file_update_product_associations` 参数传递（fileId 独立参数）
- [x] 修复 `file_get_product_associations` 返回字段（entity_id vs id）
- [x] 修复关联时保留 custom_name 和 description
- [x] 修复 `file_unlink_from_product` 委托 `file_delete`
- [x] 修复 `file_unlink_from_product` 引用计数清理

---

## Phase 1: 创建 business_relations 表 + change notification

**目标**：新建统一关系表，知识层可通过它感知所有业务关系变化

### 1.1 新建表

```sql
CREATE TABLE IF NOT EXISTS business_relations (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,      -- 'file', 'product', 'project', 'note', 'todo', 'series'
    source_id TEXT NOT NULL,
    target_type TEXT NOT NULL,      -- 'product', 'project', 'series', 'note', 'todo'
    target_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,    -- 'belongs_to', 'has_file', 'authored_by', 'tagged_with'
    metadata TEXT DEFAULT '{}',     -- JSON: 类型特有字段 (custom_name, description, folder_id 等)
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_br_source ON business_relations(source_type, source_id);
CREATE INDEX IF NOT EXISTS idx_br_target ON business_relations(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_br_type ON business_relations(relation_type);
CREATE INDEX IF NOT EXISTS idx_br_updated ON business_relations(updated_at);
```

### 1.2 change detection 函数

```rust
// business_relations.rs (新模块)
pub fn get_changed_relations(since: &str, limit: usize) -> Vec<Value> {
    // SELECT * FROM business_relations WHERE updated_at > ? ORDER BY updated_at ASC LIMIT ?
}

pub fn notify_relation_changed(source_type, source_id, target_type, target_id, relation_type, metadata) {
    // INSERT OR REPLACE INTO business_relations ...
}

pub fn notify_relation_deleted(source_type, source_id, target_type, target_id, relation_type) {
    // DELETE FROM business_relations WHERE source_type=? AND source_id=? AND target_type=? AND target_id=? AND relation_type=?
}
```

### 1.3 注册为新模块

- `src-tauri/src/services/business_relations.rs`
- 在 `mod.rs` 注册
- 在 `lib.rs` 中 `business_relations::init()` 调用建表

**涉及文件**：
- `src-tauri/src/services/business_relations.rs`（新建）
- `src-tauri/src/services/mod.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/database.rs`（建表或 migration）

---

## Phase 2: 业务层写入 business_relations

**目标**：所有业务关联操作同时写入 business_relations，双写过渡期

### 写入映射表

| 业务操作 | 当前写入表 | 同步写入 business_relations |
|---|---|---|
| 上传物料关联产品 | `file_relations` (entity_type='product') | `has_file: FILE→PRODUCT` |
| 删除物料关联 | `file_relations` DELETE | `has_file` DELETE |
| 笔记关联项目 | `note_project_mapping` | `authored_by: NOTE→PROJECT` |
| 待办关联项目 | `todo_project_mapping` | `belongs_to: TODO→PROJECT` |
| 项目关联产品 | `project_product_mapping` | `belongs_to: PRODUCT→PROJECT` |
| 项目关联客户 | `project_customer_mapping` | `associated: PROJECT→CUSTOMER` |
| 标签关联 | `tag_mappings` | `tagged_with: ENTITY→TAG` |

### 实现方式

在每个业务命令的 INSERT/DELETE 之后，加一行调用：

```rust
// 例: file_commands.rs - add_relation_with_desc 之后
crate::services::business_relations::notify_relation_changed(
    "FILE", file_id, "PRODUCT", entity_id, "has_file",
    &json!({"custom_name": custom_name, "description": description})
);
```

**涉及文件**（按优先级排序）：
1. `file_commands.rs` — file_relations 写入点最多（~15 处）
2. `notes_commands.rs` — note_project_mapping（~6 处）
3. `todo_commands.rs` — todo_project_mapping（~5 处）
4. `project_commands.rs` — project_product_mapping（~2 处）
5. `partner_commands.rs` — channel/vendor/customer（~14 处）
6. `tag_mappings` — 分散在多个 commands 中

---

## Phase 3: 知识层读取 business_relations

**目标**：知识管线和 Agent 通过 business_relations 理解业务关系

### 3.1 knowledge_lifecycle 扩展

```rust
pub fn get_relations_for_entity(entity_type: &str, entity_id: &str) -> Vec<Value> {
    // SELECT * FROM business_relations WHERE source_type=? AND source_id=?
    //    UNION
    // SELECT * FROM business_relations WHERE target_type=? AND target_id=?
}
```

### 3.2 source_doc_sync 扩展

`sync_entity` 在同步实体内容时，额外读取 business_relations 构建关系摘要写入 source_data.content：

```
[产品] OceanStor 5310
[关系] belongs_to → 项目A (PACS解决方案)
[关系] has_file ← 彩页.pdf, 白皮书.pdf
[关系] tagged_with ← 存储, 医疗
```

### 3.3 Agent 检索扩展

`ext_tool_object` 的 search/recall 模式增加关系上下文：
- search 结果附带相关业务关系
- recall 模式展示实体的完整关系网络

**涉及文件**：
- `knowledge_lifecycle.rs`
- `source_doc_sync.rs`
- `agent_workflow.rs`（ext_tool_object / ext_tool_search）

---

## Phase 4: object_links 归入 Memory 层

**目标**：object_links 只保留 Hebbian 经验关联，彻底与业务关系分离

### 4.1 清理 object_links 中的业务关联残留

当前 object_links 中如果混入了业务关联（如 FILE→PRODUCT 的 cooccurrence），需要：
- 确认 `update_object_links` 只被知识管线调用（共现关系），不被业务层调用
- 检查现有数据，删除非 cooccurrence 的行

### 4.2 将 object_links 迁移到 Memory 模块

```
memory/
├── object_links.rs     (从 synapse.rs 提取，仅 Hebbian)
├── recall.rs
├── decay.rs
└── attention.rs
```

**涉及文件**：
- `synapse.rs` → 拆分为 `memory/object_links.rs` + 知识层工具函数
- `agent_workflow.rs` 中 `expand_object_links` 调用改为 `memory::expand_links`

---

## Phase 5: 删除旧的 mapping 表（最终态）

**目标**：废弃旧表，所有业务关联统一走 business_relations

### 迁移策略

1. 新建 `migrate_to_business_relations` migration，将现有 mapping 表数据导入 business_relations
2. 业务层写入改为只写 business_relations（去掉旧表写入）
3. 读取改为从 business_relations 查询
4. 验证无报错后，DROP 旧表

### 涉及的旧表

| 旧表 | 迁移后 |
|---|---|
| `file_relations` | `business_relations` (relation_type='has_file') |
| `note_project_mapping` | `business_relations` (relation_type='authored_by') |
| `todo_project_mapping` | `business_relations` (relation_type='belongs_to') |
| `project_product_mapping` | `business_relations` (relation_type='belongs_to') |
| `project_customer_mapping` | `business_relations` (relation_type='associated') |
| `tag_mappings` | `business_relations` (relation_type='tagged_with') |

### 注意事项

- `file_relations` 有 `custom_name`、`description` 等类型特有字段 → 存入 `metadata` JSON
- `file_list_by_product` 等查询需要改为查 `business_relations` + `file_assets` JOIN
- 前端所有 `file_relations` 相关的查询/更新 API 需要同步修改

---

## 执行顺序建议

```
Phase 0 (当前) ──→ Phase 1 ──→ Phase 2 ──→ Phase 3 ──→ Phase 4 ──→ Phase 5
  修 bug          建表+通知    双写过渡     知识层读取   拆 object_links  删旧表
```

- Phase 0-1：可以立即执行，风险低
- Phase 2：双写过渡，不影响现有功能
- Phase 3：依赖 Phase 2 数据
- Phase 4-5：业务层稳定后再做，改动面大
