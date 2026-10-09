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
