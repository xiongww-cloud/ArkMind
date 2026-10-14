<script setup lang="ts">
import { ref } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { setSystemSetting } from '@/api/db-client'

const show = ref(false)

interface ChangelogEntry {
  version: string
  date: string
  icon: string
  iconBg: string
  iconColor: string
  title: string
  items: string[]
}

const changelog: ChangelogEntry[] = [
  {
    version: '14.4.1',
    date: '2026-10-13',
    icon: 'sparkles',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: '答复版式与复制体验',
    items: [
      '桌面浮窗的 AI 答复新增「复制」按钮，一键带走回答原文',
      'AI 助手（主窗口）答复去掉气泡卡片、正文平铺展示，与浮窗版式统一，长文更好读；复制/修订/Workspace 等操作不变',
      '修复断网/弱网时启动可能白屏：界面字体改为内置，页面渲染不再等待外部字体服务器',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'folder',
    iconBg: 'bg-teal-50',
    iconColor: 'text-teal-600',
    title: '目录上传与智能判重',
    items: [
      '支持整个目录上传：选中文件夹先给扫描清单（文件数/总大小/跳过原因：隐藏项、系统文件、链接、超 200MB 大文件），确认后再批量入库，防手滑',
      '目录结构镜像：本地文件夹层级原样落到你当前所在的目录里；已有同名子文件夹自动复用，大小写/首尾空格不同也视为同一个，不会并排建重复目录',
      '上传入口统一：资料库与项目里点上传区即可选「选择文件」或「选择目录」（系统文件框本身不支持两者混选），不再分设按钮',
      '智能判重：同一项目/笔记里传同名且内容相同的文件直接跳过，只提示「跳过重复文件 N 个」；同名但内容不同（比如报价改过版）自动加「(2)(3)」序号保留，不用手动改名',
      '改名同口径：把文件改成已有的「同名同内容」会被拒绝——列表重复从上传和改名两个源头都堵死',
      '粘贴、邮件附件、聊天转存等所有入库通道执行同一判重标准；内容相同但名字不同的文件仍放行，底层存储依旧只存一份',
      '文件类型判定修正：压缩包（zip/rar/7z，包括改了后缀的）明确归为「不支持」，不再误报「解析失败」反复重试；损坏的 docx/pdf 仍按真失败处理，修复后可重跑',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'envelope',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: '邮件保留期与邮箱修复',
    items: [
      '每个邮箱可单独设置邮件保留期：永久 / 1 / 3 / 6 / 12 个月；超期邮件自动清理本地库（含收件箱/已发送/历史软删残留），每 24 小时巡检一次',
      '清理语义透明：邮件正文知识随邮件收敛；附件已关联项目/资料的保留，仅被该邮件单独引用的随附清理；IMAP 服务器上的原件不受影响，同步游标不乱序',
      '修复：首次开启保留期、过期邮件成千封时清理会中途报错停住（数据库参数上限），现按每批 900 封分批执行',
      '修复：编辑邮箱账户会静默丢失服务器地址/端口/邮箱地址——旧版改完保存就再也收不了件的存量 bug，现在正常更新',
      '保留期下拉改为应用统一的下拉风格（此前是原生样式，与其他设置项不搭）',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'bolt',
    iconBg: 'bg-indigo-50',
    iconColor: 'text-indigo-600',
    title: '任务面板（计划清单）换新',
    items: [
      '重开窗口后自动恢复当前会话的任务进度与完整历史步骤并滚动到最新，不再需要重新发一条消息才显示',
      '切换会话不再串进度：每个窗口只跟随自己会话的任务（修复浮窗被主窗口进度顶掉、主窗看不到自己进度的双向问题）',
      '失败步骤显示红叉，不再与「已完成」的绿色勾混淆',
      '面板右上角新增 × 可手动清除旧计划（仅空闲时可用，任务运行中不显示，防误删断上下文）；折叠箭头方向修正为「向右=收起、向下=展开」',
      '计划未跑完自动续跑兜底：AI 提前收尾但计划尚有未完成事项时，系统自动提示其核对进度继续执行（每轮至多 2 次；若核对后确认都已完成，AI 会直接关闭计划收尾作答，不为跑而跑）',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'network',
    iconBg: 'bg-purple-50',
    iconColor: 'text-purple-600',
    title: 'ID 体系与对象关联',
    items: [
      '项目/伙伴/联系人/笔记等对外 ID 统一为新格式：PROJECT-xxxxx / VENDOR-xxxxx / PERSON-xxxxx，AI 报告里一眼能分辨对象类型；旧会话里的短称呼（P1、L-12、纯数字）继续兼容可用',
      '修复：AI 用新格式 ID 关联项目与伙伴时，伙伴挂上了、项目侧却看不到，且关系图谱出现双重前缀的脏数据（存量坏数据启动时自动修复）',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'sparkles',
    iconBg: 'bg-amber-50',
    iconColor: 'text-amber-600',
    title: '界面细节一批',
    items: [
      '「智能问答」更名「AI 助手」——早已不只是问答：委派任务、制定计划、文件操作、写文档都在这一个页面',
      '通知上的〔继续跟进〕一点即办结：加入观察池的同时自动结案离开待处理列表，不用再点第二次',
      '桌面浮窗历史消息改右侧蓝色气泡，与主窗口完全一致，不再是「我：」纯文本',
      '工作面板工具卡片等高对齐，有徽章/没徽章不再参差不齐',
      '洞察设置「观察池」从行内展开改为居中弹窗，独立滚动不再撑长设置页',
      '项目文件树与资料库侧栏逐像素对齐：行几何统一、新增筛选框（命中自动展开）、首次加载默认全折叠、「未分类」独立分隔区',
      '洞察分析师新增「档案对账」：邮件签名/正文出现你的职位、公司等身份信息且与档案不符或缺失时，会提议一键写入「我的档案」（有据才提，你可确认或忽略）',
      '修复：部分自绘下拉框「第一次点不开」（分析频率、邮件保留期等），现首次点击即响应、外部自动收起',
      '修复：macOS 上聊天附件选择按钮因平台判断遗留被隐藏的问题',
    ],
  },
  {
    version: '14.4.0',
    date: '2026-10-13',
    icon: 'settings',
    iconBg: 'bg-slate-100',
    iconColor: 'text-slate-600',
    title: '安全加固',
    items: [
      'SSRF 防护收紧：带方括号的 IPv6 地址（如 [::1]）可绕过内网拦截的漏洞已修复，联网抓取前统一做地址级校验',
    ],
  },
  {
    version: '14.3.0',
    date: '2026-10-09',
    icon: 'bell',
    iconBg: 'bg-indigo-50',
    iconColor: 'text-indigo-600',
    title: '通知处置体系与关注管理',
    items: [
      '通知每条新增〔办结〕〔不再跟进〕〔继续跟进〕：办结=我处理过了，结案；不再跟进=这事与我无关，AI 90 天内不再提起；继续跟进=把这件事钉入 AI 持续关注清单',
      '新增「关注管理」：对话里直接说"帮我盯着XX"，分析师每期主动跟进、有结果就通知你；设置 →「AI 持续关注清单」可查看、删除，你指定关注的条目永不自动过期',
      'AI 质量自愈：你的"不再跟进"率偏高时自动收紧通知门槛；分析师每轮还会从往来邮件沉淀你的稳定画像（需你确认）',
      '需要答复的问题统一生成确认卡片，不再藏在正文里',
      '待办详情支持 Markdown 渲染；笔记正文首行重复标题自动清洗；编辑器「链接/表格」等弹层不再被附件栏遮挡',
      'PDF 摄入大幅提速：同文档内多路并行 OCR + 空白页预检跳过 + 失败自动重试；笔记新增「未分类」虚拟笔记本（拖入即移出）',
    ],
  },
  {
    version: '14.2.1',
    date: '2026-10-08',
    icon: 'bolt',
    iconBg: 'bg-rose-50',
    iconColor: 'text-rose-600',
    title: '浮窗链接与通知排序修复',
    items: [
      '修复：桌面快速唤起的聊天浮窗中点击链接会在浮窗内打开——现在统一交给系统默认浏览器',
      '通知页排序优化：未办结/未忽略的通知始终置顶；处置操作后列表即时重排，无需刷新',
      '通知页移除「全部已读」：有办结/忽略就够了，已读不再是用户操作',
    ],
  },
  {
    version: '14.2.0',
    date: '2026-10-08',
    icon: 'bell',
    iconBg: 'bg-indigo-50',
    iconColor: 'text-indigo-600',
    title: '通知处置与方案导出升级',
    items: [
      '通知支持「办结/忽略」：告诉分析师你的处置结果，忽略的话题不再重复打扰',
      '通知页四类筛选「全部/待处理/已办结/已忽略」，已处置可一键回到待处理',
      '铃铛弹窗只待处置的通知才消失：一键处理、已读不算办结，通知会保留直到你明确办结或忽略',
      '多选批量操作：批量办结、批量删除',
      '方案报价单 Excel 导出重做：行高按内容自适应，小计/合计改用公式（改数量单价自动重算），合计行标签合并至 G 列排版更规范',
      '方案清单勾选「更新产品参数」保存后产品库即时生效',
    ],
  },
  {
    version: '14.1.2',
    date: '2026-10-08',
    icon: 'settings',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: '待办到期时间修复',
    items: [
      '待办到期时间正确显示时分：AI 创建、日历选择、通知一键创建写入的时间统一规范化，未带时间的默认当日 09:00（存量待办启动时自动补齐）',
      '修复月度重复任务在月末日期（如每月 31 日）卡住不滚动的老问题',
    ],
  },
  {
    version: '14.1.1',
    date: '2026-10-08',
    icon: 'file-doc',
    iconBg: 'bg-slate-100',
    iconColor: 'text-slate-600',
    title: '使用手册同步更新',
    items: [
      '邮件章节对齐新版附件展示与收件人折叠、编辑器补超链接说明、智能问答补 AI 生成文档卡片说明',
      '编译零警告维护',
    ],
  },
  {
    version: '14.1.0',
    date: '2026-10-08',
    icon: 'file-text',
    iconBg: 'bg-green-50',
    iconColor: 'text-green-600',
    title: '排版与编辑细节打磨',
    items: [
      'AI 生成 Word 文档符合中文排版习惯：正文与列表段落自动首行缩进 2 字符（居中题注、右对齐落款自动豁免）',
      'AI 创建的笔记/待办富文本正常呈现，Markdown 不再显示为源码',
      '富文本编辑器新增超链接：选中文字即可插入/移除链接（笔记、待办）',
      '邮件多收件人默认只显示 3 个，点击展开查看全部',
    ],
  },
  {
    version: '14.0.0',
    date: '2026-10-07',
    icon: 'chat-ai',
    iconBg: 'bg-purple-50',
    iconColor: 'text-purple-600',
    title: '多 Agent 架构（全新）',
    items: [
      '全新多 Agent 协作：主 Agent 按任务类型自动委托给专业助手',
      '设置页可创建自定义专属 Agent，内置业务/文档/方案/招投标助手均可手动启停',
      '方案助手支持多轮对话后逐章或整篇生成解决方案；招投标助手整理参数对比与投标响应（持续打磨中）',
      '子 Agent 共享完整会话历史，输出质量更有保障',
    ],
  },
  {
    version: '14.0.0',
    date: '2026-10-07',
    icon: 'lightbulb',
    iconBg: 'bg-amber-50',
    iconColor: 'text-amber-600',
    title: '智能通知 → 智能洞察',
    items: [
      '独立分析师 Agent 解读业务数据，洞察更贴近你的关注点',
      '新增观察池：新事件先入池沉淀，反复出现或形成关联后才通知，不再事事打扰',
      '支持添加个性化洞察规则',
      '对象关系发现：定期挖掘对象间潜在关联，一键确认关联',
    ],
  },
  {
    version: '14.0.0',
    date: '2026-10-07',
    icon: 'sparkles',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: 'AI 能力升级',
    items: [
      '「询问用户」：AI 拿不准时弹窗请你拍板，大幅减少瞎猜',
      '联网搜索：设置页配置博查 API Key 后，问答与洞察可检索互联网公开信息',
      'AI 可直接生成 Word / Excel 文件交付',
      '产品与笔记管理增强：AI 可查看/新建目录，并按目录归类产品',
    ],
  },
  {
    version: '14.0.0',
    date: '2026-10-07',
    icon: 'network',
    iconBg: 'bg-indigo-50',
    iconColor: 'text-indigo-600',
    title: '知识库与图谱',
    items: [
      '知识观测台全新升级：关系图谱结构化展示对象关联，点节点弹详情卡、聚焦邻域',
      '项目笔记/文件同步出现在笔记与资料库（按项目目录归类），不必再进项目详情页',
      '文件解析格式判定增强：按文件头内容识别，后缀错误不再导致解析失败',
      '知识管线新增「回收孤儿文件资产」，手动清理业务已删除的磁盘残留，后台亦定期自动回收',
      '修复华为 SCT 报价单部分产品型号解析遗漏',
    ],
  },
  {
    version: '14.0.0',
    date: '2026-10-07',
    icon: 'settings',
    iconBg: 'bg-green-50',
    iconColor: 'text-green-600',
    title: '交互与效率优化',
    items: [
      '桌面精灵升级为快捷键聊天窗：随手唤起对话，会话自动沉淀到智能问答页',
      '未选择会话时直接发消息，自动创建新会话',
      '项目详情选择客户/渠道/供应商时，可直接新建公司和联系人',
      '富文本编辑器「插入时间」支持完整年月日',
      '邮件附件展示升级，对齐专业邮件客户端',
      '修复项目笔记/待办看不到项目文件、无法插入的问题',
    ],
  },
  {
    version: '13.5.2',
    date: '2026-09-13',
    icon: 'file-text',
    iconBg: 'bg-green-50',
    iconColor: 'text-green-600',
    title: '文件解析优化',
    items: [
      '文件解析支持 .doc 格式',
      '优化 Word 文件解析质量，格式还原更准确',
    ],
  },
  {
    version: '13.5.1',
    date: '2026-09-12',
    icon: 'envelope',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: '邮件系统（全新）',
    items: [
      '接入 IMAP 邮箱，支持收件箱/已发送同步',
      '三栏布局：账户树 + 邮件列表 + 详情预览',
      '附件自动存入资料库，按内容 hash 去重',
      '内嵌图片自动渲染，大邮件懒加载',
      '邮件内容自动索引到知识库，AI 可检索',
    ],
  },
  {
    version: '13.5.1',
    date: '2026-09-12',
    icon: 'chat-ai',
    iconBg: 'bg-purple-50',
    iconColor: 'text-purple-600',
    title: '智能问答增强',
    items: [
      'Agent 支持按发件人、主题等筛选查询邮件',
      '通知支持操作按钮（查看邮件/项目/待办、创建待办等）',
    ],
  },
  {
    version: '13.5.1',
    date: '2026-09-12',
    icon: 'settings',
    iconBg: 'bg-slate-100',
    iconColor: 'text-slate-600',
    title: 'UI 改进',
    items: [
      '修复标题栏弹窗文字无法选中的问题',
      '通知面板展示结构化操作建议',
    ],
  },
]

function open() {
  show.value = true
}

async function close() {
  try {
    await setSystemSetting('last_seen_version', __APP_VERSION__)
  } catch (e) {
    console.error('[WhatsNewModal] save version failed:', e)
  }
  show.value = false
}

defineExpose({ open })
</script>

<template>
  <Teleport to="body">
    <Transition name="whatsnew-fade">
      <div v-if="show" class="fixed inset-0 z-[9998] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/30 backdrop-blur-sm" />

        <div class="relative bg-white rounded-2xl shadow-2xl border border-surface-200 w-full max-w-[520px] mx-4 overflow-hidden z-10 flex flex-col">
          <!-- Header -->
          <div class="flex items-center justify-between px-6 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg overflow-hidden">
                <img src="/icon.png" class="w-8 h-8 object-cover" alt="ArkMind" />
              </div>
              <div>
                <span class="text-[13px] font-semibold text-surface-700">新版本亮点</span>
                <span class="ml-2 text-[11px] text-surface-400">v{{ __APP_VERSION__ }}</span>
              </div>
            </div>
            <button @click="close" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition-colors" v-tooltip="'关闭'">
              <AppIcon name="x" size="14" />
            </button>
          </div>

          <!-- Body -->
          <div class="flex-1 overflow-y-auto px-6 py-4 max-h-[420px]">
            <div v-for="(section, i) in changelog" :key="i" class="mb-5 last:mb-0">
              <div class="flex items-center gap-2.5 mb-2.5">
                <div class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0" :class="section.iconBg">
                  <AppIcon :name="section.icon" size="16" :class="section.iconColor" />
                </div>
                <div class="flex items-center gap-2">
                  <h3 class="text-[14px] font-semibold text-surface-800">{{ section.title }}</h3>
                  <span class="text-[10px] text-surface-400">v{{ section.version }}</span>
                  <span class="text-[10px] text-surface-300">·</span>
                  <span class="text-[10px] text-surface-400">{{ section.date }}</span>
                </div>
              </div>
              <ul class="ml-10 space-y-1.5">
                <li v-for="(item, j) in section.items" :key="j" class="text-[12px] text-surface-600 leading-relaxed flex items-start gap-2">
                  <span class="w-1 h-1 rounded-full bg-surface-300 mt-[7px] shrink-0" />
                  <span>{{ item }}</span>
                </li>
              </ul>
            </div>
          </div>

          <!-- Footer -->
          <div class="flex items-center justify-end px-6 py-3 border-t border-surface-100 shrink-0">
            <button @click="close"
              class="px-5 py-1.5 rounded-lg text-[12px] font-medium bg-brand-600 text-white hover:bg-brand-700 transition-colors">
              知道了
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.whatsnew-fade-enter-active,
.whatsnew-fade-leave-active {
  transition: opacity 0.2s ease;
}
.whatsnew-fade-enter-from,
.whatsnew-fade-leave-to {
  opacity: 0;
}
</style>
