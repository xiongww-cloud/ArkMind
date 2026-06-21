# ArkMind 方舟智库 — 功能与代码审计报告

**审计日期：** 2026-06-16  
**审计范围：** 全栈代码（Rust 后端 + Vue 3 前端 + Tauri 桌面框架）  
**项目版本：** v10.0.1  

---

## 一、项目概述

**ArkMind 方舟智库**是一款基于 Tauri 2 框架的桌面级项目管理与知识管理系统，面向销售团队和产品团队，提供项目全生命周期管理、产品资料管理、AI 驱动的知识问答等功能。

### 技术栈

| 层级 | 技术 |
|------|------|
| 桌面框架 | Tauri 2.0 (Rust) |
| 前端 | Vue 3 + TypeScript + Pinia + Tiptap 编辑器 |
| 数据库 | SQLite (rusqlite) + LanceDB (向量数据库) |
| AI/RAG | 自研 embedding + 向量检索 + 重排序 + 知识管道 |
| 文件解析 | calamine (Excel) + pdf-extract + mammoth (DOCX) + text-splitter |
| 加密 | ed25519-dalek + AES-GCM |
| UI | Tailwind CSS + Heroicons |

---

## 二、功能模块清单

### 2.1 核心业务模块

| 模块 | 路由 | 后端命令数 | 状态 |
|------|------|-----------|------|
| **总览仪表盘** | `/dashboard` | — | ✅ 完整 |
| **项目库** | `/projects` | 12 | ✅ 完整 |
| **产品中心** | `/products` | 14 | ✅ 完整 |
| **资料库** | `/assets` | 8 | ✅ 完整 |
| **笔记库** | `/notes` | 25 | ✅ 完整 |
| **待办** | `/todos` | 6 | ✅ 完整 |
| **知识库问答** | `/knowledge/brain` | 35+ | ✅ 完整 |
| **知识图谱** | `/knowledge-graph` | — | ⚠️ 占位 |
| **索引检查** | `/inspector` | 6 | ⚠️ 调试用 |
| **系统设置** | `/settings` | 12 | ✅ 完整 |

### 2.2 功能详情

#### 仪表盘 (Dashboard.vue)
- ✅ 时间问候语 + 日期显示
- ✅ AI 搜索框（跳转到知识库）
- ✅ Pipeline 漏斗统计（线索挖掘/测试验证/商务报价）
- ✅ 推进中项目 Top 3（含进度条）
- ✅ 关键死线提醒（紧急/中等/正常三级颜色）
- ✅ 知识库雷达（最近笔记 + 文档）
- ✅ AI 效能统计（本月知识资产 + 节省时间）

#### 项目库 (ProjectHub.vue + ProjectDetail.vue)
- ✅ 项目 CRUD（名称/客户/金额/阶段/描述/投标时间）
- ✅ 9 种项目状态（Lead → SolutionDesign → PoC → Quotation → Bidding → Won/Ordered/Lost/OnHold）
- ✅ 多种视图（网格/列表）
- ✅ 搜索 + 状态筛选 + 时间范围筛选
- ✅ 归档功能
- ✅ 项目导入（.pms 格式）
- ✅ 批量导出（多选）
- ✅ 后端数据同步（增量 + 去重）
- ✅ 项目动态记录
- ✅ 方案配置器（设备清单 + 报价）
- ✅ 文件管理（上传/预览/下载/移动/文件夹树）
- ✅ 时间线

#### 产品中心 (ProductCenter.vue)
- ✅ 产品系列树（CRUD + 拖拽排序 + 层级嵌套）
- ✅ 产品 CRUD（名称/描述/技术参数）
- ✅ 物料管理（上传/预览/下载/重命名/关联产品）
- ✅ 物料类型分类（彩页/白皮书/检测报告/其他）
- ✅ 三栏可调布局（侧边栏 + 列表 + 详情）
- ✅ 拖拽移动（目录到目录、产品到目录）
- ✅ 防抖自动保存

#### 笔记库 (NotesPage.vue)
- ✅ 闪记（每日自动生成）
- ✅ 笔记本树（CRUD + 拖拽排序 + 层级嵌套）
- ✅ Tiptap 富文本编辑器（加粗/斜体/下划线/删除线/标题/列表/任务清单/表格/对齐/代码块/引用/分隔线）
- ✅ @提及插入项目/文件链接
- ✅ 笔记附件（上传/预览/下载/重命名/删除）
- ✅ 回收站（软删除 + 恢复 + 永久删除）
- ✅ 自动保存（600ms 防抖）
- ✅ 三栏可调布局
- ✅ 拖拽移动笔记到笔记本

#### 待办 (TodoPage.vue)
- ✅ 待办 CRUD
- ✅ 完成/取消完成切换
- ✅ 关联项目

#### 知识库问答 (KnowledgeBase.vue)
- ✅ AI 聊天界面
- ✅ RAG 检索增强生成
- ✅ 多轮对话
- ✅ 会话保存/加载
- ✅ 文档管理（导入/搜索/删除）
- ✅ 向量索引诊断

#### 系统设置 (Settings.vue)
- ✅ 用户档案
- ✅ AI 配置（Provider/模型/API Key）
- ✅ 存储配置（本地/OSS）
- ✅ 许可证激活

---

## 三、后端架构分析

### 3.1 模块划分

```
src-tauri/src/
├── lib.rs              # Tauri 应用入口，注册 200+ 命令
├── main.rs             # 程序入口
├── database.rs         # SQLite 连接池 + Schema 初始化
├── license.rs          # ed25519 许可证验证
├── oss.rs              # 阿里云 OSS 客户端
├── time_utils.rs       # 时区工具
├── commands/           # 15 个命令模块
│   ├── mod.rs
│   ├── ai_commands.rs
│   ├── asset_commands.rs
│   ├── db_commands.rs
│   ├── export_commands.rs
│   ├── file_commands.rs
│   ├── kb_commands.rs
│   ├── license_commands.rs
│   ├── notes_commands.rs
│   ├── partner_commands.rs
│   ├── product_commands.rs
│   ├── project_commands.rs
│   ├── settings_commands.rs
│   ├── todo_commands.rs
│   └── window_commands.rs
└── services/           # 20 个服务模块
    ├── agent_workflow.rs
    ├── ai_sync.rs
    ├── ai_tag.rs
    ├── chat_api.rs
    ├── data_pipeline.rs
    ├── debounced_sync.rs
    ├── embedding_client.rs
    ├── file_ingestion.rs
    ├── file_parser.rs
    ├── hybrid_knowledge.rs
    ├── knowledge_pipeline.rs
    ├── lancedb_client.rs
    ├── rag_interceptor.rs
    ├── reranker_client.rs
    ├── search_assets.rs
    ├── semantic_builder.rs
    ├── smart_chunker.rs
    ├── source_doc_sync.rs
    └── synapse.rs
```

### 3.2 后台服务

应用启动时自动启动 3 个后台工作线程：

| 服务 | 启动延迟 | 功能 |
|------|---------|------|
| KnowledgePipeline | 2s | 知识管道：文件解析 → 分块 → 向量化 → 入库 |
| AI Tag | 3s | AI 标签：自动为文档生成标签 |
| File Ingestion | 4s | 文件摄入：智能路由 + 清洗 + 落盘 + 状态广播 |

### 3.3 注册命令统计

| 类别 | 命令数 |
|------|--------|
| 数据库原始操作 | 3 |
| 许可证 | 3 |
| 项目 | 12 |
| 产品 | 14 |
| 文件 | 35+ |
| 笔记 | 25+ |
| 笔记本 | 4 |
| 合作伙伴 | 12 |
| 方案 | 12 |
| 存储 | 8 |
| 知识库 | 35+ |
| AI 同步 | 12 |
| 资产 | 8 |
| 标签 | 3 |
| 设置 | 8 |
| 窗口 | 3 |
| 导出 | 12 |
| 待办 | 6 |
| OSS | 1 |
| **合计** | **200+** |

---

## 四、代码质量评估

### 4.1 优点

| 方面 | 评价 |
|------|------|
| **模块化** | 前后端分离清晰，命令模块按业务域划分 |
| **UI/UX** | 界面精美，交互流畅，拖拽操作完善 |
| **富文本** | Tiptap 编辑器集成度高，功能丰富 |
| **RAG 架构** | 知识管道设计合理：解析→分块→向量化→检索→重排序 |
| **类型安全** | 前端使用 TypeScript，后端使用 Rust |
| **错误处理** | 大部分 API 调用有 try/catch 包裹 |
| **自动保存** | 笔记、产品描述均支持防抖自动保存 |

### 4.2 问题与风险

#### 🔴 致命缺陷 (P0)

| # | 问题 | 文件 | 影响 |
|---|------|------|------|
| 1 | `file_update_meta` 写已删除的 `file_assets.description` 列 | `file_commands.rs:186-188` | 前端更新文件描述时 SQL 报错 |
| 2 | `product_update_sort_orders` 是空操作（`Ok(())`） | `product_commands.rs:27` | 拖拽排序产品列表后静默丢弃变更 |
| 3 | `cleanup_preview_links` 潜在死锁 | `file_commands.rs:40` | Mutex 不可重入导致 panic |

#### 🟡 高危问题 (P1)

| # | 问题 | 文件 | 影响 |
|---|------|------|------|
| 4 | 6 个项目导出/导入命令全部为空壳 | `project_commands.rs:41-57` | 用户点击导出无反应 |
| 5 | `solution_version_save_draft` 忽略传入数据 | `project_commands.rs:55` | 方案配置器保存草稿时数据丢失 |
| 6 | `solution_version_finalize` 只设 status 不存 content | `project_commands.rs:54` | 最终版方案无设备清单 |
| 7 | `file_has_oss_files` 和 `file_migrate_files` 硬编码返回 | `file_commands.rs:182-183` | 存储配置 OSS 切换不可用 |

#### 🟠 中等缺陷 (P2)

| # | 问题 | 文件 | 影响 |
|---|------|------|------|
| 8 | `file_delete_version` 不删除物理文件 | `file_commands.rs:168` | 磁盘空间缓慢泄漏 |
| 9 | `product_series_update` 所有 DB 操作用 `.ok()` 静默吞错 | `product_commands.rs:6` | 用户以为操作成功实际未生效 |
| 10 | `file_delete` 同文件在 library + product 域同时存在时误删物理文件 | `file_commands.rs:150` | 数据丢失 |
| 11 | `file_move` 同域移动丢失 `creator` 字段 | `file_commands.rs:466-468` | 元数据丢失 |
| 12 | `file_update_meta` 显示名更新遗漏 `file_library_mapping` | `file_commands.rs:190-194` | 资料库修改文件名不生效 |

#### 🔵 低风险 (P3)

| # | 问题 | 影响 |
|---|------|------|
| 13 | `.ok()` 静默吞错共计 35+ 处 | 排查问题困难 |
| 14 | `product_series_delete` 不级联删除子节点 | 孤儿节点残留 |
| 15 | CSP 设置为 null | 安全风险 |

---

## 五、安全审计

### 5.1 安全措施

| 措施 | 状态 |
|------|------|
| 许可证验证 (ed25519) | ✅ 已实现 |
| AES-GCM 加密 | ✅ 已引入 |
| CSP (Content Security Policy) | ⚠️ 设置为 null |
| 输入验证 | ⚠️ 部分缺失 |

### 5.2 安全风险

1. **CSP 为 null** (`tauri.conf.json:26`) — 未启用内容安全策略，存在 XSS 风险
2. **SQL 注入** — 大部分查询使用参数化查询，但 `db_run` 命令允许执行任意 SQL
3. **文件路径遍历** — `pms://` URI scheme 中未充分验证路径
4. **API Key 存储** — AI 配置中的 API Key 明文存储在 SQLite

---

## 六、性能评估

### 6.1 正面

- ✅ 前端使用 `computed` 缓存计算结果
- ✅ 笔记编辑使用 600ms 防抖自动保存
- ✅ 文件上传有进度提示
- ✅ 向量检索使用 LanceDB（高性能）

### 6.2 待优化

- ⚠️ `Dashboard.vue` 每次激活都重新加载全部数据
- ⚠️ 笔记列表未实现分页（大量笔记时可能卡顿）
- ⚠️ 知识库搜索未实现结果缓存

---

## 七、建议修复优先级

### 立即修复 (P0)
1. 修复 `file_update_meta` 的 SQL（移除对已删除列的写入）
2. 实现 `product_update_sort_orders` 的实际排序逻辑
3. 修复 `cleanup_preview_links` 的死锁问题

### 尽快修复 (P1)
4. 实现项目导出/导入命令的实际逻辑
5. 修复 `solution_version_save_draft` 保存实际数据
6. 实现 OSS 存储迁移功能

### 计划修复 (P2)
7. `file_delete_version` 增加物理文件删除
8. 将 `.ok()` 替换为正确的错误处理
9. 修复 `file_delete` 的跨域文件引用检查

### 改进建议
10. 启用 CSP 安全策略
11. 实现笔记列表分页
12. 添加 API Key 加密存储

---

## 八、总结

**项目成熟度：** 中等偏上（功能完整，部分核心功能为空壳实现）

**代码质量：** 良好（模块化清晰，类型安全，但错误处理不足）

**主要风险：** 6 个后端命令为空实现，影响项目导出、方案保存、OSS 迁移等核心功能

**建议：** 优先修复 P0/P1 级别问题，特别是空壳命令和死锁问题，这些直接影响用户体验和数据安全。

---

*报告生成时间：2026-06-16*  
*审计工具：MiMo Code Agent*
