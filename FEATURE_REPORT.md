# ArkMind（方舟智库）功能报告

> 本报告完全基于代码库实际实现生成（分析时间：2026-08-12）。
> 技术栈：**Tauri 2（Rust 后端）+ Vue 3 + TypeScript + Pinia + Vue Router + Tailwind CSS + TipTap 2 + SQLite（rusqlite）+ LanceDB 向量库**。
> 前端版本 2.0.0（package.json），后端版本 13.2.0（src-tauri/Cargo.toml）。

---

# 第一部分：总体架构

## 1.1 应用形态

ArkMind 是一款**本地优先的桌面端个人/商务知识管理与项目管理系统**，核心定位：以项目管理为主线，围绕项目组织笔记、待办、文件、产品、合作伙伴，并通过本地 RAG（检索增强生成）+ Agent 工作流提供 AI 问答与主动洞察。

- 全部数据存储于本地：SQLite 数据库 `arkmind.db` + 内容寻址（CAS）文件存储 + LanceDB 向量库。
- AI 能力通过用户自配置的 OpenAI 兼容 API（LLM / 视觉模型 / Embedding）实现，API Key 经 AES-256-GCM 加密存储（密钥绑定设备机器码）。
- 许可证系统（Ed25519 验签 + 机器码绑定）控制 Premium 功能。

## 1.2 多窗口架构

应用由三个独立 Vite 入口构成三个窗口：

| 窗口 | 入口 | 尺寸/特性 | 功能 |
|---|---|---|---|
| 主窗口 | `index.html` → `main.ts` → `App.vue` | 自定义无边框标题栏 | 完整应用（路由 + Pinia） |
| 聊天浮窗 | `chat.html` → `chat.ts` → `ChatWindow.vue` | 620×700 透明浮窗，禁用后台节流 | 快捷 AI 对话（固定会话 fab-main-session） |
| 桌面精灵 | `sprite.html` → `sprite.ts` → `SpriteWindow.vue` | 48×74 透明、无边框、置顶、跳过任务栏 | 桌宠吉祥物，点击唤起聊天窗 |

窗口行为：
- 主窗口关闭时**隐藏而非退出**（CloseRequested → prevent_close），托盘菜单提供「显示/隐藏精灵」「退出」。
- 单实例插件：二次启动时聚焦已有主窗口。
- 聊天浮窗在打开原生文件对话框时进入 dialog mode 防止失焦自动隐藏。
- macOS 专用：通过 objc2 设置 NSWindow collectionBehavior（Auxiliary/Stationary/FullScreenAuxiliary），支持 Dock 图标 Reopen 重开主窗口。
- 精灵位置持久化（`save_sprite_position`，物理/逻辑像素换算），开关状态存配置，启动时自动恢复。

## 1.3 自定义协议 `pms://`

后端注册异步 URI Scheme，前端 WebView 内通过 `pms://<host>/<id>` 预览文件：
- id 仅允许字母数字与短横线（长度 ≤100），防路径穿越。
- 解析链：`file_relations.id → file_assets.content_hash → flat_disk_path(hash)`，限制 100MB，按扩展名返回正确 MIME（pdf/docx/xlsx/png/jpg/gif/webp）。

## 1.4 数据目录

- macOS：`~/Arkmind`；Windows：`%APPDATA%/com.arkmind.app`。
- 文件 CAS 存储默认在 `%LOCALAPPDATA%/com.arkmind.app/files`（可在设置中改 localPath，变更后 `invalidate_storage_base`）。

---

# 第二部分：前端功能（src/renderer）

## 2.1 路由与页面

Hash 路由（`createWebHashHistory`），全部懒加载：

| 路径 | 页面 | 标题 |
|---|---|---|
| `/dashboard` | Dashboard.vue | 总览 |
| `/projects` | ProjectHub.vue | 项目库 |
| `/projects/:id` | ProjectDetail.vue | 项目详情 |
| `/products` | ProductCenter.vue | 产品库 |
| `/assets` | AssetLibrary.vue | 资料库 |
| `/notes` | NotesPage.vue | 笔记库 |
| `/todos` | TodoPage.vue | 待办 |
| `/contacts` | ContactsPage.vue | 通讯录 |
| `/knowledge-graph` | KnowledgeGraph.vue | 知识库（监控） |
| `/knowledge/brain` | AgentChat.vue | 智能问答 |
| `/settings` | Settings.vue | 系统设置 |

主窗口骨架（App.vue）：左侧可折叠图标导航栏（Sidebar）+ 自定义 Header + `<router-view>`（KeepAlive 保活）+ 全局浮层（确认框、Toast、上传进度、激活弹窗、新手引导）。启动时加载用户、检查许可证（未激活/过期强制弹激活框，每 5 分钟轮询）、检查新手引导（`guide_seen`）。

## 2.2 总览仪表盘（Dashboard）

数据由 `useDashboardData` 并发聚合 12 类数据源（`Promise.allSettled`）：

- 问候语 + 业务域计数胶囊（快捷跳转）。
- 5 张 KPI 卡：活跃项目、管道金额、本月中标、预计下单 30 天、紧急事项（KpiCard 支持 delta 涨跌与紧急脉冲动画）。
- Pipeline 阶段漏斗（PipelineFunnel，点击跳项目库）+ 阶段金额环形图（StageDonut）。
- 近 6 个月新增/中标趋势（TrendChart）+ 客户 Top5（TopCustomers）。
- 今日待办 + 关键截止（投标/预计下单）+ 最近笔记。
- 知识库概览：碎片数、关联对象、冷热分布条 + 知识增长趋势图（KnowledgeGrowth）。

图表基于 vue3-apexcharts，含 ActivityHeatmap 等活动热力图组件。

## 2.3 项目库（ProjectHub，917 行）

- 项目表格列表：搜索防抖、阶段多选/客户/预算/时间过滤、排序。
- **双 ID 体系**：本地 UUID/PRO-xxx 与后端 CUID 通过 `_backendId` + localStorage（`pms_backend_project_ids`）映射，`syncProjectToBackend` 保证后端记录存在；`syncFromBackend()` 拉取后端项目与本地按 `_backendId`/名称匹配合并。
- 行内编辑；多选模式 + 批量导出 `.ark` 项目包；导入 `.ark` 包。
- 新建/编辑项目弹窗：客户下拉、预计下单日历（自绘 CalendarPicker）。
- 底部汇总状态栏。

## 2.4 项目详情（ProjectDetail，775 行）

六个 Tab：

1. **详情**：基本信息、预算/投标时间行内编辑、客户管理（ProjectDetailCustomer）、渠道管理（ProjectChannels）、供应商管理（ProjectVendors）。
2. **文件**：FileManager（文件夹树 + 文件列表、带进度上传、重命名、归档项目只读）。
3. **笔记**：ProjectNotes（TipTap 编辑 + 项目文件面板 + `@` 选择器，自动保存）。
4. **待办**：ProjectTodos（复用 TodoDetail 编辑器）。
5. **方案**：SolutionConfigurator（见 2.5）。
6. **日志**：Timeline（项目动态流，按类型过滤，note/ai 类型可行内编辑与删除）。

顶部：九阶段切换下拉（Lead → SolutionDesign → … → Won/Lost/Archived，变更自动写 system 动态，进入 Bidding 自动弹投标日历）；更多菜单：导出 `.ark`、删除项目。支持 embedded 模式被笔记/待办页内嵌预览。

## 2.5 方案配置器（SolutionConfigurator，981 行，最大组件）

项目报价/BOM 方案管理：

- 多方案（plan）× 多版本管理；版本有草稿（draft）/定稿（final）状态；版本快照历史。
- 设备清单表格：设备行 + 子网行（rowType: device/subnet）。
- 型号内联补全：输入型号自动匹配产品库并带出技术参数。
- **Excel 导入**：经通用三步向导 ExcelImportWizard（Sheet/表头行选择 → 列映射 `guessMapping` 自动猜列 → 行审查），行分类 `classifySolutionRow`：合计行自动忽略、合并单元格识别子网行、问题行人工决策。
- **Excel 导出**：`exportToPMSExcel`（ExcelJS）→ 后端 `solution_download_excel` 写盘。

## 2.6 产品库（ProductCenter，741 行）

三栏布局（目录树 / 产品列表 / 产品详情）：

- 目录树多级（product_series 树形结构），「未分类」置底且自动创建保护；拖拽移动目录与产品（自实现 mouse 拖拽，300ms 长按触发）；同级重名校验。
- 产品详情：名称/技术参数编辑（防抖保存）、标签。
- **物料管理**：产品附件上传（`file_upload entityType:'product'`）、预览、导出、删除、**跨产品关联**（`updateMaterialAssociations`，三态勾选树 none/partial/all）。
- Excel 导入（ExcelImportWizard）与导出（名称+技术参数两列）。

## 2.7 资料库（AssetLibrary，531 行）

全局文件库（孤文件/l library 域）：

- 虚拟文件夹树：创建/重命名/移动/排序/删除。
- 文件类型过滤（word/excel/ppt/pdf/图片等）、上传、重命名、备注编辑、拖拽移动到文件夹、预览/导出/删除。

## 2.8 笔记库（NotesPage，995 行，最复杂页面之一）

四视图：

1. **闪记（daily）**：每日笔记自动创建（is_daily=1）。
2. **笔记本**：多级树结构，笔记归属笔记本。
3. **孤儿笔记**：无项目非日记的笔记。
4. **回收站**：软删除（deleted_at），可恢复到指定笔记本或永久删除；后端 30 天自动清理。

编辑器能力：

- TipTap 富文本（标题 H1-H3、加粗/斜体/下划线/删除线、对齐、列表、任务列表、引用、代码块、表格含行列操作、占位提示）。
- `@` 唤起 AtMentionSelector：插入**项目 mention** 或**文件 mention** 节点（自定义 inline atom 节点，HTML 序列化为 `span[data-type="project-link"|"file-link"]`；项目节点点击弹项目预览，文件节点点击预览文件）。
- 防抖自动保存（600ms，切换前强制保存）。
- 附件面板 AttachmentPanel：上传（对话框多选/拖拽）、预览、插入编辑器、重命名、解除关联。
- 自实现拖拽排序/移动笔记本与笔记（绕开 WebView HTML5 拖拽 bug）。
- 内嵌项目详情预览（useLinkModals）。

## 2.9 待办（TodoPage，630 行）

三栏布局：待办列表（列表/日历双模式，TodoCalendar 月历按截止日期落点、优先级色点、点日期直接创建）→ TodoDetail 编辑器（TipTap 正文、优先级 P1/P2/P3、截止时间 CalendarPicker、重复周期、所属项目）→ 第三栏（todo 附件 + 按文件夹分组的项目文件树，可插入编辑器）。

**循环待办**：后端 `todos_list` 时自动处理——已完成且 repeat 到期者克隆下一周期新待办（复制项目映射），前端快捷待办面板也会把到期重复待办重置为未完成。

## 2.10 通讯录（ContactsPage，476 行）

三 Tab：渠道（CHANNEL）/ 供应商（VENDOR）/ 客户（CUSTOMER）——底层是统一的 partners 模型。实体列表 + 联系人多级结构；行内编辑、新建实体/联系人弹窗、删除确认。

## 2.11 知识库监控（KnowledgeGraph，138 行）

调用 `kb_memory_dashboard_stats`：

- 4 张统计卡：知识对象 / 碎片 / 热点 / 冷碎片。
- 对象分布柱状图、知识分布 donut、**遗忘曲线**（突触权重分布 area 图）、知识增长趋势（7天~一年切换）。
- 每 5 秒自动刷新（保存/恢复滚动位置）。
- 管线调试区：push_queue 待推送/处理中/失败计数。

## 2.12 智能问答（AgentChat，747 行）

主窗口内完整 AI 助手：

- 会话管理（ChatSidebar）：新建/重命名/删除/选择，持久化 `kb_save_sessions`/`kb_load_sessions`，消息游标分页（滚动加载更早）。
- 附件：最多 5 个、20MB，文件对话框或粘贴（macOS 剪贴板文件走后端 `chat_read_macos_clipboard_files`；粘贴图片经 `chat_save_temp_image` 落盘）。
- 消息流：Markdown 渲染（marked + DOMPurify 消毒）、打字机效果（token 队列 15ms 节流消费 + 200ms 渲染节流）、工具调用卡片（ToolCallCard 显示第 N 轮·N 个工具、thought、args、result）。
- 消息动作：复制答案、复制完整 trace、润色（`kb_chat`）、保存到笔记本（弹笔记本选择器）。
- **对象卡片**：解析 AI 回答中的 `<refs>` 标签，内嵌渲染 ProjectCard / TodoCard（可勾选）/ ObjectList。

### 浮动聊天窗（ChatWindow，548 行）

由桌宠唤起的快捷对话窗：收起态为输入条（可拖拽移动窗口），发送/加附件自动展开（`expand_chat_window` 通知 Rust 调整窗口位置，delta=564）；固定会话 `fab-main-session`；历史分页加载；消息上限 20 条。

### 桌面精灵（SpriteWindow）

SVG 机器人吉祥物（MascotSprite），区分点击（唤起聊天窗）与拖拽（8px 阈值）；监听 `notification:new`、`agent:status`、`agent:done/error` 事件切换 idle/thinking/speaking/notifying 四种动画状态。

## 2.13 系统设置（Settings，1758 行，最大视图）

- **许可证**：状态/到期时间/机器码复制/激活密钥输入。
- **数据管理**：分域导出 `.ark`（监听 `export:progress` 事件显示进度条）、全局导入、一键备份/恢复（整库 + files 目录打 zip）。
- **AI 配置**（三 Tab）：
  - LLM：多供应商列表（增删改、预设模型一键填入、测试连接 `ai_test_provider`）。
  - 视觉模型：多供应商（用于 PDF OCR），同上。
  - 向量引擎：单一嵌入模型配置 + `ai_test_embedding`。
- **智能通知**：后台 LLM 分析的开关与间隔配置。
- **桌面精灵**：开关（即时创建/显示/隐藏精灵窗口）。
- **知识库管线管理**：状态查看（运行状态、chunks 父子块数、向量化有效/积压、文件解析进度、last_embed_error）、清理知识/向量/全部、重建向量、重新物化、孤儿数据清理。
- **存储配置**：本地路径修改、OSS（遗留桩）、存储迁移（轮询进度 + MigrationToast）。
- **运行日志**：内存环形日志查看/清空。
- 版本号（`__APP_VERSION__`，从 Cargo.toml 注入）、重新打开使用指南。

## 2.14 Header 全局功能（756 行）

- 自定义窗口控制按钮（min/max/close）。
- 页标题（项目详情页显示「项目 / 名称」）。
- **全局搜索**：防抖 300ms，合并 `search_business`（业务表）+ `search_source_data`（FTS 全文）结果，`lookup_object_names` 补名，按 FILE/NOTE/PRODUCT/PROJECT/TODO 过滤徽章，点击跳转（selectionStore.jumpTo + 路由）或直接打开文件。
- **快捷待办面板**：未完成待办，可勾选/删除，自动重置到期重复待办。
- **闪记**：快捷笔记弹层，自动定位/创建当日 daily note，以带时间戳的 blockquote 前置插入。
- **通知中心**：未读数 + NotificationPanel（Markdown 渲染 + DOMPurify 消毒 + 去 UUID、标记已读/删除、点击跳实体）。

## 2.15 Pinia Stores

| Store | 职责 |
|---|---|
| `auth.ts` | 当前用户（默认本地管理员/OWNER）、头像 |
| `license.ts` | 许可证层级 tier（basic/advanced/enhanced） |
| `selection.ts` | 跨页跳转选择信号（type/id/seq/extra，各页面 watch seq 定位） |
| `project.js`（550 行） | 项目列表、方案/版本、项目动态（独立 dynamicsMap）；双 ID 体系与后端同步；操作 fire-and-forget 写动态日志 |
| `product.js` | 产品目录树（seriesTree 计算属性）、产品列表、搜索/选中/排序、物料关联 |
| `file.ts` | 遗留 mock store（内存假数据，未使用） |

## 2.16 前端基础设施

- **api/db-client.ts**（436 行）：全部 Tauri 命令的类型化封装（用户/许可证/项目/产品/文件/物料/设置/笔记/合作伙伴/方案/知识库/AI/管线/会话/精灵/记忆探索器/资产/存储/导入导出/待办），含大量 TS 接口定义。
- **api/notification-client.ts**：通知域 API（含 LoopStatus）。
- **composables**：useAgentChat（AI 对话引擎核心）、useDashboardData、useTiptapConfig、useTiptapToolbar、useConfirm（Promise 化确认框）、useToast、useUploadProgress、useNavigation、useProjectStatus（九阶段常量）、useResizablePanels、useMultiSelect（Shift 范围多选）、useStorageMigration、useLinkModals、useTooltip。
- **utils**：time.ts（时区统一体系：SQLite 存本地时间字符串，禁用 toISOString）、format.ts、sanitize.ts（DOMPurify 懒加载）、excelRaw.ts（SheetJS 通用 Excel 解析 + 合并单元格 + 列名猜测）、solutionExcel.ts / productExcel.ts（ExcelJS 导出）。
- **测试**：仅 AppIcon 与 time 两个 vitest 用例。

## 2.17 遗留/在途模块

- `src/main/`：Electron → Tauri 迁移后的占位 stub（4 个文件均为一行注释）。
- `prompts/systemPrompt.js`：PPT AI 生成模块提示词（无对应对视图，在途功能）。
- `composables/useAssetManager.ts`：演示文稿模块遗留。
- `ProductCatalogManager.vue`、`ConfiguratorPlaceholder.vue`：疑似旧版遗留。

---

# 第三部分：后端功能（src-tauri/src）

## 3.1 启动序列（lib.rs）

1. 强制初始化防抖物化调度器（需 tokio runtime）。
2. 非 macOS 移除原生标题栏（自定义 Header）。
3. 初始化内存日志收集器（1000 条环形缓冲）。
4. 初始化 SQLite（`arkmind.db`）→ 业务关联表 init/重建/回填 → 清理聊天消息（每会话最多 50 条 compressed）→ 首次启动注入使用指南到 memories 表 → 清理过期许可证。
5. 窗口事件监听（最大化事件、关闭隐藏）。
6. 数据修复（is_daily 损坏、临时预览链接清理）；Premium 用户升级 notes→content_fts FTS5 触发器。
7. 精灵窗口恢复；系统托盘（显示/隐藏精灵、退出）。
8. 后台线程（错开 1~3 秒启动）：
   - **知识管线**（sync_loop + embed_loop）
   - **数据中台** file_ingestion（先孤儿清理）
   - **通知循环** notification_loop

## 3.2 许可证系统（license.rs / license_commands.rs）

- 格式：`Arkmind-{base64_json_payload}.{base64_signature}`；payload 含 product/machine_code/expires_at/issued_at。
- **Ed25519 验签**（内嵌公钥）+ 机器码绑定 + 过期检查。
- 机器码：Windows 主板 UUID（隐藏 PowerShell 避免窗口闪烁）/ macOS IOPlatformUUID / Linux machine-id，回退 hostname；SHA256 前 8 字节 hex 作为 machine_hash。
- 激活流程：验签 → 旧 active 置 inactive → 落库 → 启动知识管线。
- 命令：`license_status`（过期自动降级）、`machine_code`、`license_activate`；启动时 `cleanup_expired_licenses`。
- Premium 门控：FTS5 触发器升级、AI sync、知识管线、Agent RAG、LanceDB。

## 3.3 项目管理（project_commands.rs，~1170 行，最大命令模块）

- 项目 CRUD：列表（LEFT JOIN 取客户名）、创建（支持 displayId 自定义编号）、更新（状态变更检测、Won 时写 won_at）、删除。
- `touch_project`：刷新项目"最后活跃时间"（笔记/待办等操作都会触发）。
- **项目动态**：增/删/改/列（仅 type∈note/ai 可删改）；状态变更自动插 system 动态；`project_archive` 归档。
- **导入导出**：`.ark` 项目包导入/导出、多项目批量打包 zip。
- **方案子系统**：
  - 方案 CRUD（solution_list/create/rename/delete）。
  - 版本：列表（聚合 items 为 JSON、SUM 金额）、创建、元数据更新、定稿（finalize）、草稿保存、删除。
  - `solution_download_excel`：前端生成 buffer 落盘。
- **图谱同步**：
  - `sync_project_products`：收集项目全部方案用到的产品，重写 project_product_mapping 与 business_relations 的 PROJECT→PRODUCT 边。
  - `sync_solution_graph`：幂等重建方案图谱边 `PROJECT -has_solution→ SOLUTION -has_version→ SOLUTION_VERSION -uses→ SOLUTION_ITEM -instance_of→ PRODUCT`，边 metadata 带金额/设备数/型号/数量/单价。

## 3.4 产品管理（product_commands.rs）

产品系列（树形）与产品 CRUD、排序（update_sort_order(s)）；所有写操作伴随知识库变更通知（`notify_changed("PRODUCT")`）。

## 3.5 文件系统（file_commands.rs，~757 行）

**CAS 内容寻址存储**：

- SHA-256 流式哈希 → `flat_disk_path(hash)` 扁平存储（hash 校验防路径穿越）。
- `file_assets`（content_hash UNIQUE，去重复用：同 hash 只加 relation）+ `file_relations`（多态 entity_type/entity_id 关联到项目/产品/笔记/待办/资料库）。
- 上传后自动入知识解析管线（`notify_file_upload`）。

命令群（40+ 个）：上传（拒绝相对路径与 `..` 穿越）、多域列表（global/all/tree/by_product/by_note/by_todo）、文件夹 CRUD（library_folders：创建/重命名/删除/移动/排序）、文件移动/跨域移动、标签映射管理、删除（整体/按域）、打开/下载副本/读字节/预览/导出、元数据更新、产品关联管理（unlink/get/update associations）。

聊天附件支持：`chat_save_temp_image`、`chat_read_macos_clipboard_files`（macOS 剪贴板文件，percent_decode + NSFilenamesPboardType 双实现）。

## 3.6 笔记系统（notes_commands.rs）

- 列表族：全部（GROUP_CONCAT 项目 IDs）、闪记（daily）、按项目、孤儿、回收站、归档、按笔记本、按笔记本计数。
- 写操作：创建（事务插 notes + note_project_mapping，写业务关联 + touch_project + 知识通知 + object_changed）、更新、软删/恢复/永久删除。
- 笔记本 CRUD。

## 3.7 合作伙伴（partner_commands.rs）

统一模型：`partners.type ∈ CHANNEL/VENDOR/CUSTOMER` + 独立联系人实体 `partner_contacts` + 多对多映射（partner_project_mapping / contact_project_mapping / partner_contact_mapping）。

- 统一命令：partner_list（按项目或全局，附带联系人）、create/update/delete、link/unlink_project（写 business_relations）、备注更新。
- 联系人：create/update/delete、link/unlink_project、全局列表/计数。
- **兼容别名三组**：`channel_*` / `vendor_*` / `customer_*`（各含 list/create/update/delete/contact CRUD/link/unlink/remark），转发到统一实现。

## 3.8 待办（todo_commands.rs）

- `todos_list`：先处理**循环待办克隆**（已完成且 repeat 到期 → 克隆下一周期，复制项目映射，更新旧待办 due_date 防重复克隆），再 LEFT JOIN 项目排序返回。
- 创建（事务 + touch_project + 知识通知）、更新（完成检测刷新项目活跃）、删除、toggle、get。

## 3.9 设置与配置（settings_commands.rs）

- `settings_get/update`：system_config（单行 tenant_id='local-user'），camelCase 映射；localPath 变更使存储缓存失效。
- **AI 配置**：`get_ai_config`（返回前解密 API Key）、`update_ai_config`（合并式部分更新 + 加密落库 + 使 embedding/chat 配置缓存失效）、`ai_test_provider` / `ai_test_embedding`（连通性测试）。
- 用户资料、存储配置（OSS 系列多为占位桩）、精灵位置/开关、通用 KV（system_settings）、日志查看/清空。

## 3.10 导入导出与备份（export_commands.rs，~930 行）

- `export_data(domains, save_path)`：按域导出 JSON + 复制附件二进制，发 `export:progress` 事件，最终打 zip。域：projects（含 solutions/versions/items/dynamics/partners/contacts/business_relations）、products、notes、todos、files、contacts。
- `backup_all` / `restore_all`：整库备份（DB + files 目录打 zip）/ 恢复（先 `close_db` 释放句柄）。
- `import_data` + 各域导入（projects/products/notes/todos/files/contacts）。
- `export_write_buffer`：前端 buffer 落盘（Excel 导出等）。

## 3.11 窗口管理（window_commands.rs）

主窗口控件、精灵窗口（创建/切换/关闭/定位）、聊天浮窗（创建/切换/展开/关闭/dialog mode）、macOS 专用 NSWindow 行为设置。

## 3.12 通知系统（notification_commands.rs）

- 通知 CRUD：列表（可仅未读）、创建（正则剥除 UUID 防泄露）、标记已读/全部已读、删除。
- `notification_stats`：未读数 + 今日 LLM 调用/Token 统计（跨天自动归零）。
- 配置（enabled/interval_minutes/startup_only）+ 循环状态（running/last/next check）。

## 3.13 AI 与管线管理命令（ai_commands.rs / asset_commands.rs）

- 状态：`ai_sync_status`（running/chunks/sources）、`knowledge_pipeline_status`、`ai_sync_diagnose`（各业务表行数）。
- **管线调试**：`pipeline_debug_status`（父子块数、向量化有效/积压 embed_pending、last_embed_error、source_data 各类型计数、文件解析 pending/processing/failed/unsupported、当前解析页进度）。
- 维护操作：清理知识（reset_pipeline 重新物化）、清向量/重建向量（vector_cache + LanceDB）、清全部（连 source_data 重入队全部文件）、resync（从业务表重建 source_data）、failed→pending 重试、孤儿数据清理。
- 资产：`assets_search` / `assets_search_fts5`（FTS5 chunk 搜索）、资产列表/诊断（fileAssets/ftsIndexed/vectorCached）、获取最新版本/解析文本、缺 source_data 文件重新解析回填（上限 200）。

## 3.14 知识库命令（commands/kb/）

**会话持久化**：
- `kb_save_sessions` / `kb_load_sessions`（每会话附最近 10 条消息）/ `kb_load_session_messages`（before_id 游标分页）/ `kb_delete_session`。
- FAB 精灵固定会话：`kb_fab_init_session` / `kb_fab_load_history` / `kb_fab_save_message`。
- 启动清理：每会话最多保留 50 条 compressed 消息。

**Agent 入口**：
- `kb_agent_chat`：spawn `agent_chat_workflow`，完成后清洗答案（`clean_tool_markers`）存 chat_messages，tool_log 存 session.tool_metadata，发 `agent:done` / `agent:error` 事件。
- `kb_chat`（无会话简版）、`agent_cancel`。

**检索与诊断**：
- `kb_search`（FTS5）、`kb_delete`（完整清理链）、`kb_list_documents`、`kb_import_file`（走统一上传管线）、`kb_save_note`。
- 诊断族：kb_diagnose / kb_search_debug / kb_data_browse / kb_browse_parents / kb_browse_vectors（LanceDB scan）/ kb_inspect_fts5 / kb_get_table_chunks / kb_test_embedding / kb_test_chat。

**Agent 外部工具（Tauri 侧）**：
- `ext_tool_search_knowledge_base`：embedding→LanceDB top30 + FTS5 top30 → RRF 融合（k=60）→ 取父块 top5（带来源元数据）。
- `ext_tool_get_document_page`、`ext_tool_get_project_context`（项目属性 + final 方案 BOM）。
- `ext_tool_translate_filter` / `ext_tool_search_filtered`：领域（library/project/product）→ file_id 过滤 + FTS5/向量双路。

**记忆探索器（kb_memory_commands.rs）**：
- `kb_memory_objects`：对象搜索（补 FILE 标题与 NOTE/TODO 项目关联，过滤已完成待办）。
- `search_business`：五表 LIKE 目录检索；`search_source_data`：FTS 全文；`lookup_object_names`：批量查名。
- `kb_memory_parents`：父块列表含**突触权重**（`effective = base × e^(−γΔt)`，γ=ln2/30天，星级 1-5）。
- `kb_memory_heatmap`：热块/冷块 top10（90 天未访问为冷）。
- `kb_memory_dashboard_stats`：类型分布、权重分布 21 桶、365 天增长、30 天活跃热力、健康度（hot/cold 比例）。

---

# 第四部分：AI 知识管线（核心架构）

## 4.1 端到端数据流

```
业务 CRUD
  └─ knowledge_lifecycle::notify_changed / notify_deleted / notify_file_upload
       └─ push_pipeline::enqueue_push → knowledge_push_queue（同实体去重，5s 防抖）
            └─ file_ingestion 数据中台（并发 5，RAII TaskGuard，重试 3 次）
                 ├─ change      → source_doc_sync::sync_entity → source_data（+FTS 触发器）
                 ├─ file_upload → file_parser 智能解析 → source_data
                 └─ delete      → 级联清理 chunks/vector_cache/LanceDB/source_data/FTS

knowledge_pipeline::sync_loop（10s，复合游标 updated_at|rowid 增量扫 source_data）
  └─ debounced_sync::SCHEDULER（5s 静默窗口，并发 4，退避重试 5 次）
       └─ ai_sync::materialize_and_store
            └─ smart_chunker 结构感知父子分块 + semantic_builder 表格→KV
                 → knowledge_chunks（parent/child）→ kk_chunks_fts（触发器同步）

knowledge_pipeline::embed_loop（每轮最多 10 批 × 5 块，指数退避 1s→60s）
  └─ hybrid_knowledge::vectorize_new_chunks（毒丸隔离）
       → embedding_client（OpenAI 兼容，LRU 缓存）→ LanceDB（纯追加）→ vector_cache
```

## 4.2 数据中台（file_ingestion.rs，~1130 行）

- 从 knowledge_push_queue 取任务，最大并发 5（ACTIVE_TASKS + RAII TaskGuard 防 panic 卡死）。
- **冷启动自愈**：processing→pending 崩溃恢复、failed→pending 自动重试、清理孤立 file_parse_cache；卡 5 分钟以上的 processing 自动重置。
- **PDF 流式处理**（process_pdf_streaming）：逐页渲染 + 视觉模型 OCR、file_parse_cache 页级断点续传、progress_page/total 心跳进度、空白页跳过；无视觉模型时走纯文本提取。
- 错误语义：`__UNSUPPORTED__`（格式白名单外 → unsupported 终态）、`__NO_RETRY__`（直接 failed）、其余 3 次重试。
- 轮询补漏：历史 notes/products/projects/todos 兜底；启动时孤儿大清理（chunks/vector_cache/source_data/business_relations 四类）。

## 4.3 智能文件解析（file_parser.rs，~1660 行）

- `parse_file_smart`：前 4KB magic number 嗅探（infer crate）→ 命中直接路由；否则后缀路由；最后轮询兜底。
- **PDF 三档回退**：视觉模型 OCR（pdfium 逐页渲染 → base64 → Vision API）→ pdfium 纯文本 → pdf-extract。
- 视觉模型适配器模式：VisionAdapter trait（OpenaiAdapter / DashscopeAdapter）+ qwen3 特判；统一提取 prompt（表格/单位/双栏严格规则）；180s 超时。
- 平台渲染回退链：pdfium → pdftoppm → macOS sips。
- OCR 三态结果（Ok/Blank/Failed）+ 错误分类 + `has_real_text` 防缓存投毒。
- Office 解析：自实现 DOCX XML→Markdown（含编号/表格）、XLSX（calamine）、PPTX。
- 两级清洗：控制字符/BOM 清理、HTML→Markdown、页码剥离、乱码行剥离、PDF 断行合并、孤儿表格包裹。

## 4.4 语义物化与分块（ai_sync.rs / smart_chunker.rs / semantic_builder.rs）

- **source_doc_sync**：业务实体 → 格式化 Markdown（NOTE/TODO 走 html_to_markdown；PRODUCT 含技术参数节；PROJECT 含阶段/负责人/金额/投标时间元信息行）→ UPSERT source_data（内容未变不刷 updated_at）。
- **smart_chunker v2 结构感知分块**：
  - Block 解析：Heading/Paragraph/Table（Markdown + HTML）/List/Code/Quote/HorizontalRule。
  - 按标题层级建 Section 树 → 自适应累积父块（COMPACT_THRESHOLD=350、MAX_PARENT=1800）。
  - 表格块经 semantic_builder 转 KV 文本并入；短尾合并。
  - 父块 ≤350 字符 → 1 个 compact 子块，否则按行组切 FOCUSED_TARGET=350 的子块。
- **semantic_builder**：HTML/Markdown 表格 → 密集 KV 语义文本（处理 rowspan/colspan/br/HTML 实体；表头模式 `列名=值；` 或记录模式 `记录N：...`），供嵌入与检索。
- **增量物化**：per-parent content_hash 对比实现"内容未变跳过"；删旧块+插新块在**单事务**（含 FTS 与 vector_cache 清理）；子块 base_weight 初始 5.0。
- **防抖调度器**（debounced_sync）：解决 TipTap 600ms 自动保存引发的重复物化——DashMap 状态机（Pending/Processing{dirty}）、5 秒静默窗口、单 worker + Semaphore 并发 4、30s 退避重试最多 5 次。

## 4.5 向量化（hybrid_knowledge.rs / lancedb_client.rs / embedding_client.rs）

- **待向量块选取**：knowledge_chunks 中不在 vector_cache 的非空 child 块（id 升序确定性）。
- **毒丸隔离**：批量失败时网络/超时类错误直接抛（交 embed_loop 退避），否则逐条重试，部分成功则把失败块写入 vector_cache 标记跳过（防永久重试）。
- **LanceDB**：惰性全局单例（`<data_dir>/lancedb`，表 knowledge_vectors）；schema：`rowid, vector:FixedSizeList<Float32>, parent_id, doc_id, content_hash, tags`；**纯追加插入**（避免 delete 全表重写，孤儿向量靠查询时 JOIN 过滤）；余弦近似 `sim = max(0, 1 − d²/2)`；支持 tags/file_ids 预过滤。
- **Embedding 客户端**：配置优先级 system_config.ai_config → kb_config 旧路径；30 秒配置缓存；智能 URL 拼接；64 条 LRU 文本→向量缓存；30s 超时共享 client。

## 4.6 突触权重系统（synapse.rs）—— 类脑记忆机制

仅 child chunk 参与，模拟神经突触：

- **惰性衰减** O(1)：`effective = base_weight × exp(−γΔt)`，γ=ln2/(30天)（**30 天半衰期**）。
- **激活强化**：父块被采纳入 LLM 上下文时，`new_base = decayed + (5.0 − decayed) × 0.3`（渐近回升至 MAX=5.0），access_count+1。
- **RRF 乘子**：`1 + ln(w+1)×0.5`（w=5 时约 ×1.9）——高权重知识在检索中排名提升。

## 4.7 检索融合（search_assets.rs / rag_interceptor.rs）

- **RRF 融合**：`rrf_fusion(fts5, vector, k=60)`，标准 Reciprocal Rank Fusion `Σ 1/(k+rank)`。
- **领域过滤"翻译官"**：把 library/project/product 领域翻译为 file_id 数组（查 file_relations），供 FTS5/LanceDB 做 `WHERE file_id IN (...)` 过滤。

## 4.8 Agent 工作流（agent_workflow.rs，~4950 行，核心）

**系统提示词**：决策辅助 Agent 身份、事实优先级（Memory 是背景非事实源）、工具调用协议、工具选择规则、业务对象模型（PROJECT→SOLUTION→VERSION→ITEM→PRODUCT 图谱）、Policy/Memory 纪律。

**11 个工具**：
1. `ext_tool_search`：知识检索（关键词 + 时间范围）。
2. `ext_tool_object`：业务对象查询（mode: detail/search/filter/count/group/aggregate；field_filters 支持 =/!=/>/</LIKE/IN/IS_NULL；覆盖全部业务实体含 PROJECT_DYNAMIC）。
3. `ext_tool_chunk_navigate`：chunk 翻页（next/prev）。
4. `ext_tool_relations`：业务图谱双向导航（带边 metadata）。
5. `ext_tool_manage_todo` / 6. `ext_tool_manage_project`（含动态 add/delete_dynamic）/ 7. `ext_tool_manage_partner` / 8. `ext_tool_manage_contact`：业务 CRUD。
9. `ext_tool_memory`：记忆 CRUD + 搜索（支持批量）。
10. `ext_tool_manage_policy`：用户策略管理。
11. `ext_tool_manage_notification`：创建通知。

**SessionMemory（会话记忆，持久化于 chat_sessions.session_state）**：
- active_objects：注意力分数（每轮 ×0.95 衰减、<0.1 且 10 轮未访问淘汰）。
- visited_chunks/sources：防重复展示（跨轮差集保存）。
- Focus、observations/hypotheses、tool_history、token 统计。
- `compute_scope_from_session`：top5 高分对象 + 新对象 → rowid 工作区。
- `generate_scratchpad`：≤4000 字工作记忆注入。

**主循环**：
- 加载 session_state + DB active 历史（带时间戳）+ 压缩摘要 + 工具数据；注入 policies（默认输出规则 + 用户策略）。
- 每轮注入 `[DYNAMIC_CONTEXT]`：scratchpad + 当前时间 + **预算系统**（150 分总预算，首搜 8 分逐轮 ×1.2 递增 + 上下文 token 估算）。
- **SSE 流式**：content 逐 token 推前端（`agent:token` 事件），累积 tool_calls delta，usage 统计 cached_tokens 命中率。
- 工具调用三级解析：原生 tool_calls → XML 解析（兼容 DSML 标记）→ HTML 解析兜底。
- `AGENT_CANCEL` 取消；上下文超限触发 context_compressor 压缩重载。

**检索执行（execute_search_knowledge）**：
1. **停用词过滤**（"项目/产品/文件"等类型词不污染检索）。
2. 对象名 LIKE 精确匹配（命中源 RRF×1.5 🔥 加权）。
3. 双路：embedding→LanceDB top30 + FTS5（`"kw" OR ...`）top30 → RRF(k=60)。
4. **scope/discovery 双路径**：工作区内原分；区外 ×0.3 且每源最多 2 块。
5. 乘突触权重乘子 → 时间过滤 → visited 去重 → content_hash 去重。
6. top3 父块输出（带来源、父块 x/N、ChunkID 短 ID、工作区标记）。
7. 被采纳的父块触发突触激活。

**上下文压缩（context_compressor.rs）**：压缩 prompt（3000-5000 字 JSON 摘要：summary/key_points/decisions/unfinished）替换最后一条用户消息，复用会话上下文调 LLM（命中 KV cache）；active 消息标记 compressed，摘要写 chat_sessions.context_metadata。

## 4.9 后台 LLM 洞察（notification_loop.rs）

- 独立线程 + 2-worker runtime；配置驱动（enabled / interval_minutes 默认 60 / startup_only / last_check_at / 今日调用统计）。
- `run_analysis`：收集增量业务数据（活跃项目、未完成待办、近期中标、新增文件/笔记/产品/合作伙伴、文件解析失败积压），**无变化则跳过 LLM**（省 token）。
- 构造后台分析 prompt 调 `agent_chat_workflow(max_iter=6, silent=true)`，LLM 通过 `ext_tool_manage_notification` 创建通知 → 前端通知中心 + 桌宠 notifying 状态。

## 4.10 内置使用指南（user_guides.rs）

内置 6+ 篇使用指南（AI 模型配置、沟通技巧、Policy 系统、AI 工具能力等），按版本标记注入/更新到 memories 表，供 AI 检索回答使用问题。

---

# 第五部分：数据库设计（database/）

技术：rusqlite + SQLite（WAL、foreign_keys、busy_timeout=5000、recursive_triggers），全局单连接 + `with_db/transaction/all/get/run` 辅助 + 30s 节流 WAL checkpoint。

## 5.1 核心表

| 领域 | 表 |
|---|---|
| 账户/许可 | users, license |
| 项目 | projects（status 九阶段、budget、bid_time、expected_order_date、won_at）, project_dynamics |
| 产品 | product_series（树形 parent_id）, products |
| 方案 BOM | solutions → solution_versions（draft/final）→ solution_items（device/subnet） |
| 配置 | system_config（ai_config/kb_config/storage/sprite，单行）, system_settings（KV，含 last_sync_time 游标） |
| 笔记 | notebooks, notes（is_daily、deleted_at 回收站、hit_count、neural_energy）, note_project_mapping |
| 文件 CAS | file_assets（content_hash UNIQUE）, library_folders, library_file_locations, file_relations（多态关联） |
| 合作伙伴 | partners（CHANNEL/VENDOR/CUSTOMER）, partner_contacts, partner_contact_mapping, partner_project_mapping, contact_project_mapping, project_product_mapping（旧 channel/vendor 映射表保留兼容） |
| 待办 | todos（priority/due_date/repeat）, todo_project_mapping |
| AI 管线 | event_store（事件溯源）, knowledge_push_queue（推送队列：pending/processing/failed/unsupported、debounce_until、progress_page/total）, source_data（统一清洗后全文，UNIQUE(source_type,source_id)）, knowledge_chunks（父子双层：chunk_type/parent_id/content_hash/base_weight/last_access_at/access_count）, vector_cache, file_parse_cache（PDF 页级 OCR 断点） |
| FTS5 搜索 | kk_chunks_fts（doc_title, content；unicode61；触发器同步，仅 child）, global_search_fts（external content on source_data）, content_fts（notes，premium） |
| 会话 | chat_sessions（tool_metadata/context_metadata/session_state）, chat_messages（status active/compressed） |
| 记忆/策略 | memories, policies（含 default-output-rules） |
| 通知 | notifications, notification_config（默认 enabled=1, interval=60min）, reminder_ack |
| 图谱 | business_relations（五元组 + UNIQUE 索引） |

## 5.2 业务关联图谱（business_relations.rs）

统一五元组边表（source_type/source_id → target_type/target_id + relation_type + metadata）：

- 启动时检测旧 INTEGER id 坏表 → 重建为 TEXT → 全量 backfill（file_relations has_file、NOTE→PROJECT authored_by、TODO/PRODUCT/PARTNER/CONTACT→PROJECT belongs_to）。
- 清理方案图谱孤儿边。
- 支撑 Agent 的 `ext_tool_relations` 图谱双向导航。

## 5.3 FTS5 检索（db_fts.rs）

- `fts5_search_chunks_slim`（bm25(5.0,1.0) 轻量命中）/ `fts5_search_chunks`（带 snippet `<mark>` 高亮）/ `fts5_search_content`（global_search_fts 大一统全文）。
- 触发器自动同步：kk_chunks_fts（INSERT+UPDATE，仅 child 块，启动时回填）、global_search_fts 镜像 source_data。

## 5.4 迁移系统（db_migrations.rs，~1100 行）

30+ 项幂等迁移：Won 项目 won_at 回填、partners company→address、映射表迁移、base_weight 异常修复、knowledge_chunks 父子重构、todos 映射迁移、MEMORY/POLICY 独立表、notifications 建表、source_data TEXT id 迁移、CHAT 去重 + UNIQUE 约束、过期队列清理等。

---

# 第六部分：安全与加密

## 6.1 敏感数据加密（crypto.rs）

- **AES-256-GCM** 加密 API Key；密钥派生：`SHA256(machine_hash || "arkmind-ai-config-v1")`（**绑定设备**，密文无法跨机解密）。
- 密文格式：`enc:<base64(nonce‖ciphertext)>`；`is_encrypted` 识别；批量处理 ai_config 的 providers/visionProviders/embedding 中的 apiKey；解密失败原样返回（容错）。

## 6.2 其他安全措施

- pms:// 协议 id 白名单校验 + 100MB 上限。
- 文件上传拒绝相对路径与 `..` 穿越；CAS hash 路径校验。
- 通知文本正则剥除 UUID 防内部 ID 泄露。
- 前端所有 AI/通知内容经 DOMPurify 消毒。
- SQL 全部参数化；LIKE 转义辅助（escape_like）。

---

# 第七部分：关键设计观察

1. **三层防抖/队列**（push_queue 5s 窗口、debounced_sync 5s 静默、sync_loop 10s 游标轮询）保证富文本高频保存下 CPU 不被榨干且变更不丢。
2. **双写一致性策略**：单事务删插 chunks + 触发器同步 FTS + LanceDB 纯追加（孤儿靠查询过滤），刻意避免 LanceDB delete 的全表重写开销。
3. **类脑记忆机制**：突触权重（30 天半衰期衰减 + 使用激活回升 + RRF 乘子加权）让"常用知识越用越强、冷知识自然遗忘"，并可视化为遗忘曲线/冷热分布。
4. **双 ID 体系**是前端最大复杂度来源：本地 UUID/PRO-xxx 与后端 CUID 通过 `_backendId` + localStorage 映射维系。
5. **许可证门控**：Premium 功能（FTS5 升级、知识管线、Agent RAG）在激活时启动，但核心检索基础设施始终初始化。
6. **OSS 云存储已废弃**：oss.rs 与相关命令均为占位桩，存储为纯本地 CAS。
7. **reranker_client.rs 为空文件**：重排序功能以 RRF + 突触权重替代。
8. 遗留模块：Electron 主进程 stub、PPT 生成提示词（在途）、file.ts mock store 等。
