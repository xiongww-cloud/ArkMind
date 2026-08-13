# ArkMind 架构收敛计划（下一个大版本）

> 状态：**定稿，待最终评审后实施**（已经过两轮架构评审，全部修订意见已并入）
> 制定日期：2026-08-12
> 前置文档：`FEATURE_REPORT.md`（基于代码实现的功能报告）
>
> 已确认决策：
> 1. 双 ID 收敛**包含在本版本内**，采用两阶段迁移（不做局部修补）。
> 2. LanceDB 向量回收**自动触发**，首版带 dry-run 保险开关。
> 3. Stage 0 迁移演练**双轨并行**：真实库副本演练 + 自动化夹具测试。
> 4. 回退采用"备份恢复 + 装回旧版"路径，**不做双写兼容层**（见 Rollback Design）。
> 5. 本计划评审通过后再进入实现。

## 0. 目标与原则

本版本不加新功能。目标是把现有能力变得**稳定、可解释、可靠**：

- 消除数据一致性事故源（双 ID、fire-and-forget）。
- 消除存储隐患（LanceDB 孤儿向量无限增长）。
- 补齐生命线测试。
- 删除死代码与冗余抽象，降低认知负担。

原则：**减少机制，而不是增加机制**。

### 冻结清单（本版明确不做）

- reranker / Cross-Encoder
- 新 ranking 算法
- synapse 数学模型调整（衰减率、激活公式冻结；仅加回归测试锁定"实现=公式"）
- 新 Memory 类型、新图谱边类型
- GraphRAG 化文本知识
- Context/Memory 新机制

---

## Stage 0 — 迁移演练（双轨，先于一切实施）

P0-1 是本版本风险最大的操作，正式实现前必须完成演练。

### 轨 A：真实库演练（一次性，本机执行）

- 形态：`src-tauri` 增加 `#[cfg(debug_assertions)]` 调试命令 `debug_rehearse_id_migration(source_db_path)`（复用真实迁移代码路径，避免逻辑分叉）。
- 流程：复制真实 `arkmind.db` 到临时目录 → 以临时库执行完整迁移逻辑 → 输出演练报告：
  - 项目总数 / 需迁移数 / 冲突清单
  - 10 个引用位置逐一的 old-ID 残留计数
  - 迁移耗时、备份校验结果
- 产出物：演练报告文本，人工核对通过后才允许进入阶段 1。

### 轨 B：自动化夹具测试（永久进入测试套件）

`cargo test` + tempfile 构造仿真库（含旧 backendId 的全套关联数据），断言四组用例：

1. **完整性**：迁移后全部 10 个引用位置 `WHERE = old_backend_id` 计数为 0（冲突项除外）。
2. **幂等**：跑两次迁移，DB 快照逐字节相等。
3. **中断回滚**：注入中途失败（坏行/锁冲突），数据完整恢复迁移前状态。
4. **冲突处理**：localId 已被占用 → 该项目跳过，状态 = `completed_with_conflicts`，其余项目正常迁移。

---

## 阶段 1 — 数据安全

## P0-1 双 ID 收敛：两阶段 canonical ID 迁移

### 0. 现状（代码事实）

- 前端项目本地 ID（UUID / `PRO-xxx`）与后端 `projects.id` 分离，靠 `_backendId` 字段 + localStorage `pms_backend_project_ids` 映射维系。
- 后端 `project_create`（project_commands.rs:7-23）**已支持**用 `displayId` 作为 `projects.id` 落库；仅当 displayId/id 均空时才生成 `uuid::Uuid::new_v4()`。
- 即：经 `syncProjectToBackend` 创建的项目，后端 id 已等于前端本地 id。**真正产生分歧的只有存量数据**：早期未传 displayId 的项目、以及仅存在于 localStorage 映射中的记录。
- 前端复杂度集中点：`stores/project.js`（591 行，21 处相关逻辑）、`ProjectHub.vue` 的 `syncFromBackend()` 匹配合并。

### 1. 目标形态

- **canonical ID = 前端本地 UUID**，创建时作为 `projects.id` 直传后端（沿用 displayId 通道，协议不变）。
- 前端删除：`_backendId` 字段、`loadBackendIdMap`/`saveBackendIdMap`、`syncProjectToBackend` 的双重逻辑。
- `project.js` 预计从 591 行收敛到约 300 行。

### 2. 阶段 A：基础设施（`run_migrations` 内，纯状态准备）

**不在此处执行任何数据改写**（关键修正：run_migrations 在 setup() 内先于前端执行，此时无法获得前端身份映射，直接迁移存在启动时序死结）。

仅确保迁移状态字段存在（存 `system_settings`）：

| 字段 | 取值 |
|---|---|
| `project_id_migration_state` | `pending` / `running` / `completed` / `completed_with_conflicts` / `failed` |
| `project_id_migration_version` | 迁移方案版本号（当前 = 1） |
| `project_id_migration_at` | 完成时间 |
| `project_id_migration_count` | 成功迁移项目数 |
| `project_id_migration_conflicts` | 冲突数 |

### 3. 阶段 B：身份映射驱动迁移（前端启动后触发）

```
旧版本数据启动（新版客户端）
    ↓
前端 Project Store 加载本地项目
    ↓
生成完整 identity map（主来源 = 当前项目数据本身）：
  [{ localId: project.id, backendId: project._backendId }, ...]
  localStorage pms_backend_project_ids 仅作补充比对，不作唯一来源
    ↓
migration_submit_project_id_map(map)   ← 新 Tauri 命令
    ↓
后端：state pending → running
    ↓
迁移前自动 backup_all → backups/pre_id_migration_<ts>.zip
  并校验备份完整性（可打开、项目行数一致）；校验失败则不启动迁移
    ↓
关闭 FK enforcement（在事务外完成——SQLite 中 PRAGMA foreign_keys
  在事务内切换无效）
    ↓
BEGIN
  逐对 UPDATE projects.id + 全部 10 个引用位置
  冲突对（localId 已存在且 ≠ backendId）→ 跳过并记录
    ↓
统一 integrity check（所有引用位置 old-ID 计数必须为 0，冲突项除外）
    ↓
COMMIT → 恢复 FK
    ↓
逐项目写审计表 project_id_migration_log(old_id, new_id, status, migrated_at)
state = completed 或 completed_with_conflicts（含 count/conflicts 统计）
    ↓
返回结果 → 前端标记 localStorage 映射 migrated:true（保留不删除，
  见 Rollback L3）→ 进入 canonical 模式
```

### 4. 迁移改写的引用位置（当前确认 10 个，以 integrity check SQL 为准）

1. `project_dynamics.project_id`
2. `solutions.project_id`（solution_versions/items 经 solution_id 间接关联，无需改）
3. `note_project_mapping.project_id`
4. `todo_project_mapping.project_id`
5. `project_product_mapping.project_id`
6. `partner_project_mapping.project_id`
7. `contact_project_mapping.project_id`
8. `file_relations.entity_id WHERE entity_type='PROJECT'`（先 SELECT DISTINCT 确认实际取值，含小写变体）
9. `business_relations.source_id WHERE source_type='PROJECT'` **及** `target_id WHERE target_type='PROJECT'`（两个方向）
10. `source_data.source_id WHERE source_type='PROJECT'`（source_data_id 不变，chunks/FTS/向量不受影响）

不改写：`chat_sessions.session_state` 内 JSON 中的历史项目 id 引用（active_objects 等工作区记忆）——允许自然衰减淘汰，迁移日志中说明。

### 5. 硬性规则

- **禁止名称+创建时间启发式匹配**。ID 迁移宁可留 conflict，不猜错。
- 冲突项目保留旧 ID 作为兼容态存在；整体状态如实标记 `completed_with_conflicts`，**不声称完成**。
- 无映射可提交时前端仍调 `migration_submit_project_id_map({})`；后端检测无 backendId-only 项目则直接标记 `completed`。

### 6. 前端改造

| 位置 | 改动 |
|---|---|
| `stores/project.js` | 删 `_backendId`/映射/syncProjectToBackend 双逻辑；所有操作直接用 `project.id`；启动时检测旧数据 → 生成 identity map → 调 `migration_submit_project_id_map` → 成功后标记 localStorage `migrated:true`（**不删除**，保留一个版本周期） |
| `ProjectHub.vue` | `syncFromBackend()` 简化为按 id 直接合并（去掉名称/displayId 模糊匹配） |
| `useDashboardData` / Header / selection store | 检查 `_backendId` 寻址残留，统一为 id |
| `api/db-client.ts` | `createProject` 的 displayId 参数保留（协议不变），注释标注为 canonical id 通道 |

### 7. 验收标准

- Stage 0 双轨全部通过。
- 全局 grep `_backendId` 无残留引用。
- 老用户升级后：项目详情六 Tab（笔记/待办/文件/方案/日志/合作伙伴）数据完整。
- 新建项目刷新后不丢失、不"复活"。
- `get_logs` / 诊断可读到迁移状态行：state / projects / migrated / conflicts。

---

## P0-2 写路径确认（消除 fire-and-forget）

### 原则

**本地 store 是后端持久化状态的可信投影，不是乐观缓存。**
写入成功 = 后端写成功 → 才提交 UI 状态。

### 关键路径（必须 await + 失败 toast + 不回填本地状态）

| 操作 | 现状 | 改后 |
|---|---|---|
| 创建项目 | fire-and-forget（project.js:95） | await 后端返回后才入本地列表 |
| 删除项目 | warn 后仍删本地（project.js:494-497） | await 成功才移除；失败 → toast + 保留本地 |
| 状态变更（含 Won/归档） | fire-and-forget | await + 失败回滚本地状态 |
| 方案定稿 finalize | 待核 | await + 失败提示 |
| 方案版本保存 | 已 await | 补失败 toast |

非关键路径（动态日志追加、touch 活跃时间）保持 fire-and-forget，但统一加 `.catch` 记 warn 日志（供 `get_logs` 排查）。

### 验收

- 注入 API 失败（mock throw）：UI 出错误 toast 且本地状态未变更。

---

## 阶段 2 — 存储健康

## P1 LanceDB 孤儿向量自动回收

### 现状（代码事实）

- `hybrid_knowledge::embed_batch` 纯追加；删 chunk 只清 `vector_cache`，LanceDB 内向量残留，查询时 JOIN 过滤。
- 已有基础设施：`scan_all_rowids()`、`delete_chunks_by_rowids(&[i64])`。
- LanceDB delete 为全表重写 → 必须批量、低频。

### 设计（`services/vector_vacuum.rs`）

1. **触发点**：knowledge_pipeline 内，每 24 小时检查一次。
2. **双时间戳**（语义分离：检查 ≠ 清理）：
   - `last_vector_vacuum_check`：每次检查后更新
   - `last_vector_vacuum`：仅实际执行清理后更新
3. **流程**：`scan_all_rowids()` 得集合 A；`SELECT rowid FROM knowledge_chunks` 得有效集合 B；孤儿 = A − B。
4. **触发条件**：`|孤儿| ≥ 500 且 orphan_ratio ≥ 20%`，否则跳过。
5. **dry-run 保险开关**：`vector_vacuum_dry_run` 配置项，开发期默认 `true`（只扫描、记日志、不删除）；**Stage 0 + 轨 B 测试验证孤儿判定无误后，发布前翻为 `false`**。开关长期保留作保险丝。
6. **删除前二次校验**：执行 delete 前再次断言孤儿集合与 knowledge_chunks 无交集。
7. **互斥**：与 embed_loop 同 runtime 顺序执行，天然互斥，无需加锁；失败记日志下周期重试。
8. **观测**：`pipeline_debug_status` 增加 `last_vector_vacuum_check` / `last_vector_vacuum` / `orphan_vectors`（设置页只读展示，不加按钮）。

### 已知限制（本版不处理）

- 超大规模（百万级以上）时单次全表重写较重，届时再评估空闲窗口调度；当前数据规模下维持本方案。

### 验收

- 单测：临时 LanceDB 插入向量 → 删 chunks → vacuum → 孤儿清除、活向量保留。
- dry-run 模式：日志有扫描报告、库无变化。
- 集成：上传→删除文件循环 N 次后日志出现 vacuum 记录，检索结果不变。

---

## P2 生命线测试补齐

原则：Rust 侧优先（`cargo test` + tempfile，不依赖 WebView）。前端只补纯逻辑。

### Rust 测试清单（按优先级）

| # | 目标 | 覆盖点 | 位置 |
|---|---|---|---|
| 0 | **ID 迁移（轨 B）** | 完整性/幂等/中断回滚/冲突 四组用例（见 Stage 0） | database/db_migrations.rs |
| 1 | db_migrations 幂等 | 空库 init → 再跑不报错；两次 init schema 一致 | database/ |
| 2 | smart_chunker | 标题层级分块、HTML 表格 rowspan/colspan、KV 模式断言、超长父块拆分、短尾合并 | services/smart_chunker.rs |
| 3 | ai_sync 增量物化 | content_hash 未变跳过、变更后单事务删旧插新、子块 base_weight=5.0 | services/ai_sync.rs |
| 4 | synapse 锁定 | 30 天衰减恰好减半、激活渐近 MAX=5.0 不越界、rrf_multiplier(5.0)≈1.9；**另加 activation 被真实调用的回归测试**（针对历史"只衰减不回升"bug）。目的：锁定实现=公式，不推动模型变更 | services/synapse.rs + agent_workflow 调用点 |
| 5 | business_relations | upsert 幂等、delete_by_source 级联、backfill 正确建边 | services/business_relations.rs |
| 6 | vector_vacuum | 见 P1 验收（含 dry-run） | services/vector_vacuum.rs |
| 7 | 备份/恢复往返 | `backup_all` → `restore_all` 到临时目录 → 行数与关键字段一致（显式覆盖，支撑 Rollback L3） | commands/export_commands.rs |
| 8 | import/export 往返 | projects 域导出 zip → 导入临时库 → 一致 | commands/export_commands.rs |
| 9 | file_parser 清洗 | strip_page_numbers / merge_pdf_broken_lines / has_real_text 边界 | services/file_parser.rs |

### 前端测试清单

| # | 目标 | 覆盖点 |
|---|---|---|
| 1 | utils/excelRaw.ts | guessMapping 命中、合并单元格 mergeMap |
| 2 | utils/solutionExcel.ts | buildItemsFromMapping 设备/子网行分类 |
| 3 | stores/project.js（重构后） | 创建/删除 await 失败时本地状态不变（mock api 层） |

### 验收

- `cargo test`、`npm run test`、`npm run lint`、`vue-tsc` 全绿。

---

## 阶段 3 — 减负

## P3.1 UI 概念收敛

| 项 | 改动 |
|---|---|
| `KnowledgeGraph.vue` + Sidebar + 路由 meta | 更名「知识观测台」（路径 `/knowledge-graph` 保留避免破坏书签，仅改文案），弱化"图谱"表述 |
| 设置页管线区 | 展示 P1 新增 vacuum 状态字段（只读） |

## P3.2 死代码清理（放在最后：前面改动落定后 grep 结果才可信）

| 文件/模块 | 说明 | 处理 |
|---|---|---|
| `services/object_changed.rs` | 薄封装，trigger_sync 已是 no-op；约 10+ 处调用点 | 删模块 + 机械删除所有调用点（每处为单行独立语句，逐处 review diff，cargo test 兜底） |
| `services/reranker_client.rs` | 0 字节空文件 | 删文件 + mod 声明 |
| `services/prompts/OLD_MEMORY_PROMPT.md` | 无 include_str! 引用的归档 | 删（git 历史可查） |
| `src-tauri/src/oss.rs` | 一行注释桩 | 删文件 + mod 声明；settings_commands 中 oss_* 桩命令**保留**（前端仍调用），加 deprecated 注释 |
| `src/renderer/src/stores/file.ts` | mock store | grep 确认无 import 后删 |
| `src/components/common/PaneToggle.vue` | Vite root 外孤儿文件 | 删 |
| `src/main/`（4 个 stub） | Electron 迁移残留 | 删目录 |
| `ConfiguratorPlaceholder.vue`、`ProductCatalogManager.vue` | 疑似未用 | grep 确认后删 |
| `prompts/systemPrompt.js` + `useAssetManager.ts` | PPT/资产管理在途素材 | **保留原位** + `TODO(incubating)` 注释，不移目录（避免制造新的迁移成本） |

---

## Rollback Design（严重 Bug 回退机制）

### 四层回退

**L0 — 迁移过程内回退**
单事务 + 事务外 FK 开关：中途任何失败 → 完整回滚，DB 零变化，state = `failed`。integrity check 不过则不 COMMIT。

**L1 — 迁移前快照**
迁移执行前自动 `backup_all` 到 `backups/pre_id_migration_<ts>.zip`，并**校验备份完整性**（可打开、项目行数一致）；校验失败则迁移不启动。该备份永不自动清理。

**L2 — 迁移审计表**
`project_id_migration_log(old_id, new_id, status, migrated_at)` 逐项目记录。用途：事后精确追查 ID 变迁；为极端情况下的反向迁移工具保留可能。成本：一张表 + 每项目一行。

**L3 — 发布后回退路径**

必须正视的事实：**迁移完成后旧版本客户端与已迁移 DB 不兼容**（旧前端 localStorage 映射指向已不存在的 backendId）。完整回退路径：

```
发现严重 Bug
    ↓
退出新版本
    ↓
restore_all(backups/pre_id_migration_<ts>.zip)   ← 恢复到迁移前数据
    ↓
重装旧版本客户端
    ↓
localStorage 旧映射仍在（新版只标记 migrated:true，未删除）→ 旧版正常工作
```

配套措施：
1. 验收含 backup/restore 往返测试（P2 #7）。
2. 附回退 Runbook（上述流程 + 每步验证点）。
3. localStorage 旧映射在迁移成功后**保留一个版本周期**，只标记 `migrated:true`——这是 L3 能工作的关键。
4. 明确**不做双写兼容层**（等于保留双 ID，违背本版本目标，代价远大于收益）。

### 非迁移类变更的回退

| 变更 | 回退方式 | 说明 |
|---|---|---|
| P0-2 写路径 | git revert + 重发版 | 无数据格式变化 |
| P1 vacuum | dry-run 开关 + git revert | 删除不可逆，故首版 dry-run 验证后放行 + 删除前二次校验 |
| P3 死代码 | git revert | 无 |

---

## 实施顺序与依赖

```
Stage 0（迁移演练，双轨）
  ├─ 轨 A：真实库副本演练 → 人工核对报告
  └─ 轨 B：自动化夹具测试（ID 迁移四组用例）

阶段 1（数据安全）
  ├─ P2#1 db_migrations 幂等测试（先行打底）
  ├─ P0-1 两阶段 canonical ID 迁移（后端 + 前端）
  └─ P0-2 写路径确认

阶段 2（存储健康）
  ├─ P1 向量 vacuum（dry-run 起步）
  └─ P2 其余 Rust 测试（#2-6, #8-9）

阶段 3（减负）
  ├─ P3.2 死代码清理（最后做）
  ├─ P3.1 UI 更名
  └─ 前端测试（P2 前端 #1-3）
```

每个阶段完成后：`cargo test` + `npm run test` + `npm run lint` + `vue-tsc` 全绿，再进入下一阶段。

## 风险登记

| 风险 | 等级 | 缓解 |
|---|---|---|
| ID 迁移改写引用位置遗漏 | 高 | 统一 integrity check SQL 覆盖全部 10 个位置；Stage 0 真实库演练；L1 备份 + L2 审计表 |
| localStorage 映射丢失导致无法迁移 | 中 | 主来源改为前端当前项目数据本身；localStorage 仅补充；匹配不上留 conflict（completed_with_conflicts），不猜 |
| 迁移误改（猜错 ID） | 高 | 硬性规则：禁止启发式匹配，宁可冲突 |
| vacuum 误删活向量 | 中 | dry-run 首版验证 + 删除前二次校验 + 阈值门槛 + 轨 B 测试 |
| vacuum 全表重写阻塞检索 | 低 | 24h 周期 + 双阈值；与 embed_loop 天然互斥；大规模场景记为已知限制 |
| 发布后严重 Bug 需回退 | 中 | Rollback Design L0-L3；备份校验；localStorage 保留一个版本 |
| 删 object_changed 调用点误删业务逻辑 | 低 | 单行独立语句逐处 review diff；cargo test 兜底 |

## 评审确认清单

- [x] P0-1 两阶段迁移时序（阶段 A 状态准备 / 阶段 B 前端触发）
- [x] identity map 主来源 = 前端当前项目数据；localStorage 仅补充
- [x] 禁止启发式匹配；冲突状态 = completed_with_conflicts
- [x] chat session_state 不改写（自然衰减）
- [x] P1 vacuum 双时间戳 + dry-run 开关（验证后放行）
- [x] Stage 0 双轨演练
- [x] Rollback Design L0-L3（含不做双写层）
- [x] PPT 素材保留原位 + TODO(incubating)
- [x] 冻结清单
- [ ] **最终评审：本文件确认后进入实现**
