<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import AppIcon from '@/components/common/AppIcon.vue'
import { setSystemSetting } from '@/api/db-client'

const router = useRouter()

const show = ref(false)
const current = ref(0)

interface GuideAction { label: string; path: string; icon: string; primary?: boolean }
interface GuideChapter {
  group: string
  icon: string
  iconBg: string
  iconColor: string
  title: string
  content: string
  tips?: string[]
  actions?: GuideAction[]
}

const chapters: GuideChapter[] = [
  // ─── 入门 ────────────────────────────────────────────────────────────
  {
    group: '入门',
    icon: 'home', iconBg: 'bg-brand-50', iconColor: 'text-brand-600',
    title: '快速上手',
    content: `欢迎使用 ArkMind 方舟智库——一款帮你「攒知识、用 AI」的桌面软件。三步上手：

① 配置 AI（必做）— 打开「系统设置 → AI 模型配置」，填好三类模型的接口地址、密钥、模型名称：
· 语言模型：AI 的大脑，负责理解和回答。推荐：小米 MiMo V2.5
· 向量模型：给内容建索引，让 AI 快速找到你的资料。推荐：硅基流动 bge-m3
· 视觉模型（可选）：让 AI 看懂图片、扫描件和 PDF。推荐：小米 MiMo V2.5
每类模型都有推荐预设可一键填入，可先「测试连接」再保存。

② 录入业务数据 — 项目、客户、产品、笔记、文件、待办……平时怎么用就怎么录，内容会自动沉淀成你的知识库。

③ 开始使用 AI — 在「智能问答」里直接问：「这个客户去年签了多少钱」「帮我把 XX 项目状态改成商机」。它基于你的真实数据回答，也能帮你动手操作软件。

左侧目录可以随时跳到任意功能的详细说明。`,
    actions: [
      { label: '去配置 AI', path: '/settings', icon: 'settings', primary: true },
      { label: '新建项目', path: '/projects', icon: 'folder-open' },
      { label: '新建笔记', path: '/notes', icon: 'file-doc' },
      { label: '新建待办', path: '/todos', icon: 'list' },
      { label: '添加联系人', path: '/contacts', icon: 'users' },
      { label: '上传文件', path: '/assets', icon: 'archive' },
      { label: '添加产品', path: '/products', icon: 'cube' },
      { label: '智能问答', path: '/knowledge/brain', icon: 'chat' },
      { label: '知识观测台', path: '/knowledge-graph', icon: 'network' },
    ],
    tips: [
      '使用中遇到任何不懂的地方，都可以直接问 AI 助手，它随时待命。',
      '这份指南以后可以随时重新打开：系统设置 → 右上角「使用指南」。',
    ],
  },
  // ─── 功能模块 ────────────────────────────────────────────────────────
  {
    group: '功能模块',
    icon: 'dashboard', iconBg: 'bg-blue-50', iconColor: 'text-blue-600',
    title: '总览',
    content: `打开软件第一眼看到的驾驶舱，一屏掌握业务全貌。

① 顶部统计条 — 项目、联系人、客户、产品、笔记、文件、知识碎片的总数，点任意一项直接跳转对应页面。

② 五张核心指标卡 — 活跃项目、管道金额（含本月新增和环比）、本月中标（个数/金额/胜率）、未来 30 天预计下单、紧急事项。

③ 图表区 — 项目阶段漏斗、阶段金额分布、近 6 个月新增与中标趋势、客户金额 Top5。

④ 右侧三栏 — 待办事项（点进去直达）、关键截止日期（投标/下单，红色脉冲表示紧急）、最近笔记。

⑤ 知识库概览 — 知识碎片冷热分布和增长趋势。

逻辑说明：
· 「紧急事项」= 已逾期的待办 + 2 天内要投标的项目。
· 「预计下单」只统计填了预计下单日期、且未中标/已下单/未丢单/未归档的项目。
· 所有数据实时计算，不需要手动刷新。`,
  },
  {
    group: '功能模块',
    icon: 'folder-open', iconBg: 'bg-emerald-50', iconColor: 'text-emerald-600',
    title: '项目库',
    content: `所有项目的一张大表：筛选、排序、就地编辑，都在这里完成。

基本用法：
· 新建项目：填名称、金额、客户、阶段；阶段选「投标」时会要求填投标时间；重名会提示。
· 筛选：搜索框（名称/描述/客户/负责人），可按阶段、金额区间、更新时间、预计下单季度组合过滤。
· 排序：点表头（名称/阶段/金额/预计下单/更新时间）切换升降序。
· 就地编辑：鼠标悬停名称旁的铅笔改名字；点阶段下拉直接换阶段；点金额、预计下单日期直接改，不用进详情页。
· 导入导出：勾选多个项目可打包导出为一个 .ark 文件；也能导入别人发你的 .ark 项目包。

逻辑说明：
· 你改状态、金额、日期时，软件会自动往该项目的「日志」里写一条动态记录。
· 列表默认只显示"你的"项目（你是负责人、或没有负责人）。
· 点某一行进入项目详情，切换页面再回来时滚动位置会保留。`,
  },
  {
    group: '功能模块',
    icon: 'template', iconBg: 'bg-indigo-50', iconColor: 'text-indigo-600',
    title: '项目详情',
    content: `点开一个项目后进入的工作台，顶部 6 个标签页管理这个项目的方方面面。

① 详情 — 名称、阶段、金额、描述、预计下单都可直接改；投标倒计时（≤3 天变红，过期变紫）。下方是客户、渠道、供应商三个面板：可以关联已有伙伴、给每个伙伴写备注、管理联系人。

② 文件 — 项目专属文件夹：建目录、上传、预览、下载、备注、打标签、拖拽移动文件到目录。

③ 笔记 — 项目专属笔记，带完整富文本编辑器，右侧面板可以直接把项目文件插进笔记里。

④ 待办 — 项目专属待办，新建时自动关联到当前项目。

⑤ 方案 — 报价方案制作（重点）：
· 一个项目可以有多个方案清单，每个清单保存多个历史版本（V1.0、V1.1……）。
· 平时编辑都在"草稿"上进行，草稿自动保存、刷新不丢。
· 产品型号列有联想输入，直接搜产品库里的型号。
· 点「保存版本」把草稿归档成正式版本，可打标签（降本减配/增项扩容等）。
· 支持导入/导出 Excel（点击方案标题栏的上传/下载按钮）。导入时弹出格式规范提示，Excel 中所有数据行完整保留，不会漏行。

⑥ 日志 — 这个项目发生的一切：系统日志（自动记录）、用户日志（手动写）、AI 日志。

逻辑说明：
· 详情页所有字段改动都会自动写日志，日志页大部分是机器生成的。
· 删除项目会级联删除它的方案、版本和文件关联，删除前有确认提示。`,
  },
  {
    group: '功能模块',
    icon: 'cube', iconBg: 'bg-amber-50', iconColor: 'text-amber-600',
    title: '产品库',
    content: `产品资料的三栏工作台：左边系列目录、中间产品列表、右边产品详情。

基本用法：
· 目录：可以无限层级建子目录、改名、删除；「未分类」是系统目录，固定在最下面。
· 拖拽：按住 300 毫秒开始拖——目录拖到目录上/下=排序，拖进目录=移动层级；产品拖到目录上=归类。
· 多选拖拽：Ctrl+点击逐个选、Shift+点击范围选，选中后拖到目录上批量移动。仅在拖拽移动和新建时禁用行内编辑。
· 产品详情：双击名字改名（全库查重）；产品描述、技术参数直接编辑，松手 0.6 秒后自动保存。
· 物料：给产品上传彩页、白皮书、检测报告等资料（无需选择类型，统一归类），上传时必须关联至少一个产品；同一个物料可以关联多个产品（勾选目录会连带选中目录下所有产品）。

Excel 导入导出：
· 导出：一键把全部产品导出为 Excel（两列：产品名称、技术参数）。
· 导入：选择 Excel 文件后会先弹出预览——共解析多少条、多少条可导入、多少条同名会跳过；选好目标目录再导入。同名产品只跳过、不覆盖。

逻辑说明：
· 物料上传是后台进行的，上传进度看右下角全局进度条。
· 从全局搜索跳过来时，会自动展开对应目录并定位到产品。`,
  },
  {
    group: '功能模块',
    icon: 'archive', iconBg: 'bg-cyan-50', iconColor: 'text-cyan-600',
    title: '资料库',
    content: `全局文件仓库：所有上传的文件在这里统一管理。

基本用法：
· 目录：左侧建多级文件夹，支持拖拽排序和移动层级；「未分类」放没归类的文件。
· 文件：按类型筛选（Word/Excel/PPT/图片等）、按名称搜索；搜索时仅过滤右侧文件列表，左侧目录树保持不变；点文件名直接预览，可以改名、写备注、下载、删除。
· 多选拖拽：Ctrl+点击逐个选、Shift+点击范围选，选中后拖到文件夹上批量移动。
· 整理：把文件拖到文件夹上就能移动过去；上传时会存进当前选中的文件夹。

Excel 导入：
· 导入 Excel 时自动解析表格内容（支持合并单元格、多 Sheet），解析结果完整保留所有数据行。
· 解析后的内容自动进入知识库构建队列，AI 问答可以直接检索 Excel 里的内容。

逻辑说明（重要）：
· 文件上传后，后台会自动解析文字内容，喂给知识库——之后 AI 问答就能搜到文件里的内容。
· PDF 按页解析，中断后下次会接着上次进度继续。
· 扫描件 PDF 和图片里的文字需要靠「视觉模型」做 OCR 识别。如果没配视觉模型，这类文件解析不出来。
· 解析进度和失败原因去「系统设置 → 知识管线管理」查看（文件解析进度条）。不支持的格式会标记为"不支持"，不会反复重试。`,
  },
  {
    group: '功能模块',
    icon: 'file-doc', iconBg: 'bg-violet-50', iconColor: 'text-violet-600',
    title: '笔记库',
    content: `三栏笔记应用：笔记本树 → 笔记列表 → 富文本编辑器。

基本用法：
· 闪记：进入「闪记」视图会自动创建今天的笔记（标题就是日期），随手记今天的事。昨天的空白闪记会被自动清理。
· 笔记本：可以建多级子笔记本；删除笔记本时里面的笔记会进回收站。
· 多选拖拽：Ctrl+点击逐个选、Shift+点击范围选，选中后拖到左侧笔记本上批量移动。
· 编辑器：标题、加粗/斜体/下划线、列表、任务清单、表格、分割线、代码块、插入时间、超链接（选中文字后点工具栏链接按钮套用，再点可移除；待办详情同款编辑器）。
· 回收站：删掉的笔记在这里，可以恢复到指定笔记本，或永久删除。

附件与 @ 链接（两个独立的东西）：
· 附件：右侧附件面板管理"附着在笔记上的文件"——上传、预览、插进正文、删除。附件会写入知识库，并随笔记一起导出。
· @ 插入链接：只是正文里的引用卡片（项目/文件），点卡片直达目标——不会产生附件、不写入知识库。

逻辑说明：
· 自动保存：停止输入 3 秒后自动保存；切换笔记/页面时也会保存。内容没变化不会重复保存。
· 回收站的笔记 30 天后自动清空。
· 点笔记里的项目引用卡片，会以弹窗形式直接打开项目详情，不用跳转页面。`,
  },
  {
    group: '功能模块',
    icon: 'list', iconBg: 'bg-rose-50', iconColor: 'text-rose-600',
    title: '待办',
    content: `任务管理：列表 + 日历两种视图，支持优先级、重复任务和附件。

基本用法：
· 视图切换：列表视图按状态分组；日历视图按月看分布，点某天可以直接在那一天建待办。
· 优先级：高/中/低（P1/P2/P3），用颜色区分；逾期的日期会标红。
· 重复任务：截止日期那里可以选每天/每周/每月重复。
· 详情：点进待办可以写富文本内容、关联项目、加附件；关联项目后还能看到项目里的文件。
· 已完成：列表底部的折叠抽屉，点勾选可以撤销完成。

逻辑说明：
· 排序规则：有日期的按截止日升序，没日期的按优先级排。
· 重复任务完成后，截止日会自动滚到下一期，不会消失。
· 内容编辑 3 秒后自动保存。
· 待办到期前，AI 会通过AI洞察提醒你。`,
  },
  {
    group: '功能模块',
    icon: 'users', iconBg: 'bg-teal-50', iconColor: 'text-teal-600',
    title: '通讯录',
    content: `客户、渠道、供应商的统一管理：三个标签页，用法完全一致。

基本用法：
· 左侧公司列表：按名称/地址/电话/联系人搜索，角标显示联系人数量。
· 右侧公司信息：名称、电话、地址、备注都可以点铅笔就地编辑，回车保存。
· 联系人：每家公司下可以加多个联系人（姓名/职位/电话/邮箱/备注），职位有常用预设（总经理/经理等）一键填入。
· 删除公司会连带删除它下面的所有联系人，删除前有确认。

逻辑说明：
· 在项目详情里把这家公司关联为渠道/客户/供应商后，通讯录里它的「项目」一栏会自动显示关联的项目名。
· 建议把伙伴信息录完整——AI 回答"这个客户去年签了多少钱"之类问题时，靠的就是这些数据。`,
  },
  {
    group: '功能模块',
    icon: 'envelope', iconBg: 'bg-blue-50', iconColor: 'text-blue-600',
    title: '邮件',
    content: `接入 IMAP 邮箱，收发同步邮件，附件自动入库，邮件内容自动索引到知识库。

基本用法：
· 添加邮箱账户：系统设置 → 邮件账户，填 IMAP 服务器/端口/用户名/密码，密码加密存储。
· 同步：后台每 1 分钟自动同步收件箱和已发送；也可手动点同步按钮。新账户自动跳过历史邮件。
· 三栏布局：左侧账户/文件夹树，中间邮件列表（每批 20 封，滚动自动加载更多），右侧邮件详情。
· 收件人：多人时默认只显示 3 个，点「查看详情」展开全部，可再收起；按主题、发件人名称/地址实时过滤。

附件：
· 邮件同步时附件自动存入资料库（按内容 hash 去重，相同文件只存一份）。
· 非图片附件在正文下方以文件卡片展示：点名称预览，悬停出现下载/复制到剪贴板按钮。
· 图片附件直接全宽渲染显示（与正文内嵌图同款加载效果）；正文已内嵌的图片（如签名图、OA 插图）不会重复出现在附件里。
· 附件不出现在资料库页面，仅在邮件详情中可见。

知识库集成：
· 所有邮件正文自动写入知识库，AI 问答可检索邮件内容。
· Agent 支持按发件人、主题、时间等筛选查询邮件。

逻辑说明：
· 密码经 AES-256 加密后存储在本地数据库。
· 删除账户会同时清除该账户下所有邮件和附件元数据。`,
  },
  // ─── AI 与系统 ───────────────────────────────────────────────────────
  {
    group: 'AI 与系统',
    icon: 'network', iconBg: 'bg-fuchsia-50', iconColor: 'text-fuchsia-600',
    title: '知识观测台',
    content: `你的知识关系网络与健康体检：看对象之间如何关联、AI 知识库运转得怎么样。

基本用法：
· 对象关系图谱：节点是有业务关联的对象（项目/文件/产品/联系人……），大小=关联数量、颜色=类型；点击节点聚焦它的邻域，浮层里可「打开该对象」直达对应页面；点空白处返回全局。
· 类型筛选：顶部 chips 可按类型隐藏/显示节点；孤立（没有任何关联）的对象不进图。
· 分布与曲线：碎片按类型分布、遗忘曲线（有效权重分布）、增长趋势（可切时间窗）。
· 管线健康：向量化覆盖率、积压、文件解析失败等；有问题可一键去「系统设置 → 知识管线管理」处理。

逻辑说明：
· 这个页面每 60 秒自动刷新一次，也可手动点「刷新」。
· 「遗忘」不是删除——内容还在，只是 AI 检索时的优先级降低了。真正重要的内容 AI 会主动记住（见智能问答章节）。
· 如果碎片数量长时间不涨，说明知识管线可能卡住了，看「管线健康」或去「系统设置 → 知识管线管理」。`,
  },
  {
    group: 'AI 与系统',
    icon: 'chat-ai', iconBg: 'bg-purple-50', iconColor: 'text-purple-600',
    title: '智能问答',
    content: `AI 助手的主战场：基于你的知识库对话。

基本用法：
· 会话：左侧可以开多个会话、改名、删除；第一句话会自动成为会话标题。
· 提问：直接输入，回车发送，Shift+回车换行。回答逐字流式输出，中途可以点「停止」。
· 每条回答下面可以一键复制答案；回答里的项目/待办引用卡片，点卡片直达对应页面。
· 让它写 Word/Excel（如「生成一份设备清单表格」）：回答下方会出现文件卡片，点名称预览，悬停可下载、复制到剪贴板；删除该会话时生成的文件一并清理。

聊天附件（重点）：
· 点输入框左侧的回形针按钮选择文件，或直接 Ctrl+V 粘贴图片/文件。
· macOS 支持从 Finder 复制文件后直接粘贴到聊天窗口。
· 支持的格式：PDF、DOCX、TXT、MD、XLSX、XLS、CSV、JSON（文档），JPG/PNG/GIF/BMP/WEBP（图片）。
· 附加 Excel 报价单后直接问「这个报价里有哪些服务器？」；附加产品彩页问「核心参数是什么？」。
· 超长文档会自动截断，保留表格结构和关键内容。
· 支持同时附加多个文件，AI 会综合分析。
· 附件芯片显示在输入框上方，点击可移除。

它会记住你：
· 对话中你提到的设备知识、客户偏好、怎么称呼你……它觉得重要就会默默记下来，下次自然用上。
· 你也可以给它立规矩：「以后回答先列要点再展开」「推荐设备优先考虑性价比」。
· 随时开口调整：「你记住了哪些内容？」「把 XX 那条改成……」「忘掉 XX 那条」。

逻辑说明：
· 会话存在本地，换电脑可以用「系统设置 → 数据导出」带走。
· 桌面精灵/浮窗的对话也会进入「智能问答」列表，与这里统一查找管理。
· 它能帮你操作软件（建待办、改状态、加联系人），重要改动会先跟你确认。`,
  },
  {
    group: 'AI 与系统',
    icon: 'bolt', iconBg: 'bg-orange-50', iconColor: 'text-orange-600',
    title: '桌面精灵',
    content: `常驻桌面的小助手：不占地方，随时开聊。

基本用法：
· 点击精灵：打开/收起聊天窗。
· 拖动精灵：挪到屏幕任意位置，位置会被记住，下次启动自动还原。
· 状态动画：思考时天线闪烁、回答时点头、有新通知时跳动提醒你。
· 聊天窗：点窗口外会自动收起，输入了一半的内容不会丢；往上滚可以加载更早的记录。

逻辑说明：
· 精灵的对话会出现在「智能问答」列表，可在那里统一查找、继续或删除。`,
  },
  {
    group: 'AI 与系统',
    icon: 'search', iconBg: 'bg-sky-50', iconColor: 'text-sky-600',
    title: '全局搜索与快捷入口',
    content: `顶部标题栏藏着几个高频工具，不用进页面就能干活。

① 全局搜索（顶部搜索框）— 搜项目、产品、笔记、待办、文件……一次搜全部，结果按类型分组，点击直接跳转到对应位置。

② 快速待办（待办图标）— 下拉看当前未完成的待办，可以直接勾选完成或删除，点 + 号快速新建。

③ 快速闪记（笔记图标）— 随手记一句话，自动写进今天的闪记笔记里（带时间戳）。写错了可以一键丢弃。

④ 通知铃铛 — AI 发现的提醒都在这里：待办到期、投标截止、项目停滞、AI 洞察。点通知可以展开详情、跳到对应项目或待办；右上角一键全部已读。

逻辑说明：
· 通知由后台AI洞察循环定期生成，频率在「系统设置 → AI洞察」里调（30 分钟到 24 小时）。
· 有新通知时桌面精灵也会跳动提醒。`,
  },
  {
    group: 'AI 与系统',
    icon: 'settings', iconBg: 'bg-slate-100', iconColor: 'text-slate-600',
    title: '系统设置',
    content: `软件的控制中心：模型、通知、数据、许可证都在这里。

① AI 模型配置 — 语言/视觉/向量三类模型。语言模型可以配多家（随时切换默认）；每家有推荐预设一键填入，先测试连接再保存。密钥只显示掩码，不会被回读。
注意：更换向量模型后旧向量不兼容，软件会提示你清空并重新向量化。

② AI洞察 — 开关 + 检查频率（仅启动时/30 分钟到 24 小时）。

③ 数据管理 —
· 导出数据：勾选范围（项目/产品/笔记/待办/文件/通讯录）导出为 .ark 包。
· 导入数据：从 .ark 包恢复。
· 一键备份：整个数据库 + 文件 + 向量库打包成 zip。
· 一键恢复：用备份 zip 覆盖当前全部数据（操作前会确认，恢复后自动重启）。
建议：定期一键备份，换电脑时用备份迁移。

④ 知识管线管理 — 三条进度条：物化、向量化、文件解析，卡住时的自救按钮（重新向量化/重置管线/重置全部数据/清理孤儿数据）。

⑤ 运行日志 — 软件运行日志，排查问题用，可以清空。

⑥ 许可证 — 机器码 + 激活码激活，显示版本（基础版/高级版/AI增强版）和有效期。

⑦ 快速唤起 — 全局快捷键，任意应用唤起/收起 AI 浮窗。`,
  },
  // ─── 版本 ────────────────────────────────────────────────────────────
  {
    group: '版本',
    icon: 'sparkles', iconBg: 'bg-amber-50', iconColor: 'text-amber-600',
    title: '版本更新日志',
    content: `v14.2.1 · 2026-10-08
· 修复：聊天浮窗中点击链接会在浮窗内打开网页，现统一交给系统默认浏览器
· 通知页未办结/未忽略的通知置顶排序，处置后列表即时重排；移除多余的「全部已读」入口

v14.2.0 · 2026-10-08
· 通知可「办结/忽略」处置：忽略的话题分析师不再重复提醒；通知页四类筛选（全部/待处理/已办结/已忽略），已处置可恢复待处理
· 铃铛弹窗只有一键处理/已读不会让通知消失，需明确办结或忽略（处置才算处理完）
· 通知多选支持批量办结、批量删除
· 方案报价单 Excel 导出重做：行高自适应，小计/合计为公式（改数量单价自动重算），合计标签合并至 G 列
· 修复方案清单「更新产品参数」保存后产品库不即时生效的问题

v14.1.2 · 2026-10-08
· 修复待办到期时间只显示年月日不显示时分：AI 创建/日历选择器/通知一键创建的时间统一规范化落库，缺时间的默认当日 09:00（存量数据自动补齐）
· 修复月度重复任务在月末（如 31 日）卡住不滚动的老 bug

v14.1.1 · 2026-10-08
· 使用手册同步更新：邮件附件卡片与图片直渲、收件人折叠展示、编辑器超链接、AI 生成文档卡片、产品物料归类说明
· 编译零警告维护修复（mac 专属编码函数按平台门控）

v14.1.0 · 2026-10-08
· AI 生成 Word 文档符合中文排版：正文与列表段落自动首行缩进 2 字符（居中/右对齐行自动豁免）
· AI 创建的笔记/待办富文本正常呈现，Markdown 不再显示为源码
· 富文本编辑器支持超链接：选中文字后插入/移除链接（笔记、待办）
· 邮件多收件人默认折叠显示 3 个，可展开查看全部

v14.0.0 · 2026-10-07
· 多 Agent 架构：主 Agent 按任务自动委托专业助手，支持自定义 Agent；内置业务/文档/方案/招投标助手均可手动启停
· 智能通知升级为智能洞察：独立分析师 Agent、观察池沉淀、个性化规则、对象关系一键发现
· AI 能力：询问用户卡片、博查联网搜索、直接生成 Word/Excel、产品按目录归类管理
· 知识观测台全新升级：对象关系图谱、节点详情卡、邻域聚焦；项目笔记/文件自动归类进笔记与资料库
· 桌面精灵改为快捷键聊天窗；未选会话发消息自动建会话；编辑器插入完整年月日；合作伙伴选择时可直接新建
· 修复：华为 SCT 报价单型号解析遗漏、项目笔记/待办无法插入项目文件、文件后缀错误导致的解析失败

v13.5.2 · 2026-09-13
· 文件解析支持 .doc 格式
· 优化 Word 文件解析质量，格式还原更准确

v13.5.1 · 2026-09-12
· 新增邮件系统（IMAP 收发同步、附件自动入库、知识库索引）
· Agent 支持邮件实体筛选查询
· 通知支持操作按钮和批量删除
· 后台分析约束为只读+通知
· 修复标题栏弹窗文字选中问题
· 新增版本升级弹窗

v13.3.0 · 2026-09-01
· 产品管理工具增强、知识库目录信息修复、移除废弃字段

v13.2.0 · 2026-08-14
· 华为 SCT 报价单自动识别导入与服务费分摊、Excel 三步导入向导、方案版本草稿持久化、新增空白版本

v13.1.0 · 2026-08-11
· 业务活跃时间语义化、中标时间标记、智能通知摘要扩展

v13.0.0 · 2026-08-09
· 聊天附件功能、多选拖拽交互、Excel 解析优化、macOS 适配

v12.1.0 · 2026-07-25
· 全面功能审计，修复多个问题

v12.0.0 · 2026-06-01
· 新增知识库功能

v11.0.0 · 2026-05-01
· 新增方案报价功能

v10.0.0 · 2026-04-01
· 新增 AI 智能问答`,
  },
]

const groups = computed(() => {
  const out: { name: string; items: { chapter: GuideChapter; index: number }[] }[] = []
  chapters.forEach((c, i) => {
    let g = out.find(x => x.name === c.group)
    if (!g) { g = { name: c.group, items: [] }; out.push(g) }
    g.items.push({ chapter: c, index: i })
  })
  return out
})

const currentChapter = computed(() => chapters[current.value])

function open() { current.value = 0; show.value = true }
function close() { show.value = false }

async function markSeen() {
  try { await setSystemSetting('guide_seen', '1') } catch (e) { console.error('[GuideModal] markSeen failed:', e) }
}
async function finish() { await markSeen(); close() }

function goTo(path: string) {
  router.push(path)
  markSeen()
  close()
}

defineExpose({ open })
</script>

<template>
  <Teleport to="body">
    <Transition name="guide-fade">
      <div v-if="show" class="fixed inset-0 z-[9998] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/30 backdrop-blur-sm" />

        <div class="relative bg-white rounded-2xl shadow-2xl border border-surface-200 w-full max-w-[760px] mx-4 overflow-hidden z-10 flex flex-col h-[580px]">
          <!-- Header -->
          <div class="flex items-center justify-between px-6 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg overflow-hidden">
                <img src="/icon.png" class="w-8 h-8 object-cover" alt="ArkMind" />
              </div>
              <span class="text-[13px] font-semibold text-surface-700">ArkMind 使用指南</span>
            </div>
            <button @click="finish" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition-colors" v-tooltip="'关闭'">
              <AppIcon name="x" size="14" />
            </button>
          </div>

          <!-- Body: TOC + content -->
          <div class="flex-1 flex min-h-0">
            <!-- Left TOC -->
            <div class="w-[168px] shrink-0 border-r border-surface-100 overflow-y-auto py-3 px-2 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
              <div v-for="g in groups" :key="g.name" class="mb-3">
                <p class="px-2 mb-1 text-[10px] font-semibold text-surface-300 uppercase tracking-wider">{{ g.name }}</p>
                <button v-for="item in g.items" :key="item.index"
                  @click="current = item.index"
                  class="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-[12px] text-left transition-colors mb-0.5"
                  :class="current === item.index ? 'bg-brand-50 text-brand-700 font-medium' : 'text-surface-500 hover:bg-surface-50 hover:text-surface-700'">
                  <AppIcon :name="item.chapter.icon" size="13" :class="current === item.index ? item.chapter.iconColor : 'text-surface-400'" />
                  <span class="truncate">{{ item.chapter.title }}</span>
                </button>
              </div>
            </div>

            <!-- Right content -->
            <div class="flex-1 overflow-y-auto px-6 py-5 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
              <div class="flex items-center gap-3 mb-4">
                <div class="w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0" :class="currentChapter.iconBg">
                  <AppIcon :name="currentChapter.icon" size="20" :class="currentChapter.iconColor" />
                </div>
                <h3 class="text-[16px] font-bold text-surface-800">{{ currentChapter.title }}</h3>
              </div>
              <p class="text-[13px] text-surface-600 leading-relaxed whitespace-pre-line">{{ currentChapter.content }}</p>

              <!-- Action buttons grid (action hub page) -->
              <div v-if="currentChapter.actions?.length" class="mt-4 grid grid-cols-3 gap-2">
                <button v-for="a in currentChapter.actions" :key="a.path"
                  @click="goTo(a.path)"
                  class="flex flex-col items-center justify-center gap-1.5 py-2.5 px-2 rounded-xl border transition-colors"
                  :class="a.primary ? 'border-brand-300 bg-brand-50 text-brand-700 hover:bg-brand-100' : 'border-surface-200 text-surface-600 hover:border-brand-300 hover:bg-surface-50 hover:text-brand-600'">
                  <AppIcon :name="a.icon" size="16" />
                  <span class="text-[11px] font-medium">{{ a.label }}</span>
                </button>
              </div>

              <!-- Tips -->
              <div v-if="currentChapter.tips?.length" class="mt-4 space-y-1.5">
                <p v-for="(t, i) in currentChapter.tips" :key="i" class="text-[11px] text-surface-400 leading-relaxed flex items-start gap-1.5">
                  <AppIcon name="lightbulb" size="11" class="flex-shrink-0 mt-0.5 text-amber-400" />
                  {{ t }}
                </p>
              </div>
            </div>
          </div>

          <!-- Footer -->
          <div class="flex items-center justify-between px-6 py-3 border-t border-surface-100 shrink-0">
            <span class="text-[11px] text-surface-300">{{ current + 1 }} / {{ chapters.length }}</span>
            <button @click="finish"
              class="btn-primary text-xs">
              完成
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.guide-fade-enter-active,
.guide-fade-leave-active {
  transition: opacity 0.2s ease;
}
.guide-fade-enter-from,
.guide-fade-leave-to {
  opacity: 0;
}
</style>
