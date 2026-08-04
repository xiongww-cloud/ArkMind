<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import AppIcon from '@/components/common/AppIcon.vue'
import { setSystemSetting } from '@/api/db-client'

const router = useRouter()

const show = ref(false)
const step = ref(0)

interface GuideAction { label: string; path: string; icon: string; primary?: boolean }
interface GuideStep {
  icon: string
  iconBg: string
  iconColor: string
  title: string
  content: string
  tips?: string[]
  actions?: GuideAction[]
}

const steps: GuideStep[] = [
  {
    icon: 'home', iconBg: 'bg-brand-50', iconColor: 'text-brand-600',
    title: '欢迎来到方舟智库',
    content: `欢迎使用 ArkMind 方舟智库。这是一款帮你「攒知识、用 AI」的桌面软件。\n\n你平时在软件里记录的项目、客户、产品、笔记、文件、待办……这些内容，都会自动沉淀成你自己的知识积累，越用越有价值。`,
  },
  {
    icon: 'sparkles', iconBg: 'bg-purple-50', iconColor: 'text-purple-600',
    title: 'AI 能帮你做什么',
    content: `配好 AI 之后，它能做三件大事：\n\n① 精准回答 — 基于你软件里的真实数据回答，不瞎猜。比如「这个客户去年签了多少钱」。\n\n② 帮你操作软件 — 直接说需求，AI 帮你动手完成。比如「新建一个待办」「把 XX 项目状态改成商机」「添加一个客户联系方式」。它只会在你授权的范围内操作，重要改动都会先跟你确认。\n\n③ 整理与总结 — 帮你查资料、归纳要点、写总结，像一位熟悉你全部业务的助手。`,
  },
  {
    icon: 'settings', iconBg: 'bg-blue-50', iconColor: 'text-blue-600',
    title: '配置 AI 需要 3 样东西',
    content: `在「系统设置 → AI 模型配置」里配置 3 类模型：\n\n① 语言模型（必配）— AI 的大脑，负责理解和回答。推荐：小米 MiMo V2.5\n② 视觉模型（可选）— 让它看懂图片、扫描件和 PDF 文档里的内容。推荐：小米 MiMo V2.5\n③ 向量模型（必配）— 把内容建索引，让 AI 快速找到你的资料。推荐：硅基流动 bge-m3\n\n每类只需填三项：接口地址、密钥、模型名称。有推荐预设一键填入，还能先测试连接再保存。`,
  },
  {
    icon: 'archive', iconBg: 'bg-emerald-50', iconColor: 'text-emerald-600',
    title: '知识是怎么沉淀的',
    content: `软件会把你录入的项目、笔记、文件等资料，自动整理成「知识碎片」，存入你的个人知识库。\n\nAI 回答问题时，会先从你的知识库里检索相关内容，再结合它们给你精准答案。内容越多、越完整，AI 就越懂你。\n\n提示：知识库有「衰减机制」，长期不用的内容会逐渐淡化。重要信息 AI 会帮你记住（见下一步）。`,
  },
  {
    icon: 'lightbulb', iconBg: 'bg-amber-50', iconColor: 'text-amber-600',
    title: '把它当成一位新同事',
    content: `和 AI 相处，就像和一位新同事相处——它会记住你，也会听你的规矩。\n\n它会主动记住：不用刻意教它。对话中你提到的设备知识、客户偏好、怎么称呼你……它觉得重要就会默默记下来，下次聊天自然用上。\n\n它会守你的规矩：你也可以交代它工作习惯，它会一直照做。比如「以后回答时先列要点，再展开说明」「我是做安防集成的，推荐设备时优先考虑性价比」。\n\n它记住的东西、立下的规矩，随时都能调整，直接开口就行：\n· 「你记住了哪些内容？」\n· 「你现在有哪些规矩？」\n· 「把 XX 那条改成……」「忘掉 XX 那条」\n\n相处越久，它越懂你。`,
  },
  {
    icon: 'bell', iconBg: 'bg-rose-50', iconColor: 'text-rose-600',
    title: '智能通知 —— AI 替你盯着业务',
    content: `AI 会定期扫描你的数据，发现值得关注的事情时主动提醒你（点右上角铃铛查看）：\n\n· 待办到期：快到截止日期的任务\n· 投标/签约节点：临近的关键时间点\n· 项目停滞：长时间没进展的项目\n· 重要洞察：AI 分析出的业务要点\n\n你可以在「系统设置 → 智能通知」里开关提醒、设置检查频率（每 30 分钟到 24 小时）。也可以直接对 AI 说「帮我创建一条通知」或「查看通知」。`,
  },
  {
    icon: 'play', iconBg: 'bg-cyan-50', iconColor: 'text-cyan-600',
    title: '开始使用',
    content: '一切就绪！从这里可以快速到达任何功能，开始你的知识积累之旅。',
    actions: [
      { label: '去配置 AI', path: '/settings', icon: 'settings', primary: true },
      { label: '新建项目', path: '/projects', icon: 'folder-open' },
      { label: '新建笔记', path: '/notes', icon: 'file-doc' },
      { label: '新建待办', path: '/todos', icon: 'list' },
      { label: '添加联系人', path: '/contacts', icon: 'users' },
      { label: '上传文件', path: '/assets', icon: 'archive' },
      { label: '添加产品', path: '/products', icon: 'cube' },
      { label: '智能问答', path: '/knowledge/brain', icon: 'chat' },
      { label: '知识库', path: '/knowledge-graph', icon: 'network' },
    ],
    tips: [
      '使用中遇到任何不懂的地方，都可以直接问 AI 助手，它随时待命。',
      '这份指南以后可以随时重新打开：系统设置 → 右上角「使用指南」。',
    ],
  },
]

const totalSteps = steps.length
const isFirst = computed(() => step.value === 0)
const isLast = computed(() => step.value === totalSteps - 1)
const currentStep = computed(() => steps[step.value])

function open() { step.value = 0; show.value = true }
function close() { show.value = false }
function next() { if (!isLast.value) step.value++ }
function prev() { if (!isFirst.value) step.value-- }

async function markSeen() {
  try { await setSystemSetting('guide_seen', '1') } catch (e) { console.error('[GuideModal] markSeen failed:', e) }
}
async function finish() { await markSeen(); close() }
async function skip() { await markSeen(); close() }

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

        <div class="relative bg-white rounded-2xl shadow-2xl border border-surface-200 w-full max-w-[520px] mx-4 overflow-hidden z-10">
          <!-- Header -->
          <div class="flex items-center justify-between px-6 pt-5 pb-1">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg overflow-hidden">
                <img src="/icon.png" class="w-8 h-8 object-cover" alt="ArkMind" />
              </div>
              <span class="text-[13px] font-semibold text-surface-700">ArkMind 使用指南</span>
            </div>
            <button @click="skip" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition-colors" title="关闭">
              <AppIcon name="x" size="14" />
            </button>
          </div>

          <!-- Step dots -->
          <div class="flex items-center justify-center gap-1.5 pt-3 pb-1">
            <span v-for="(s, i) in steps" :key="i" class="rounded-full transition-all duration-200"
              :class="i === step ? 'w-4 h-1.5 bg-brand-500' : (i < step ? 'w-1.5 h-1.5 bg-brand-300' : 'w-1.5 h-1.5 bg-surface-200')" />
          </div>

          <!-- Content -->
          <div class="px-6 py-5 min-h-[280px]">
            <div class="flex items-center gap-3 mb-4">
              <div class="w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0" :class="currentStep.iconBg">
                <AppIcon :name="currentStep.icon" size="20" :class="currentStep.iconColor" />
              </div>
              <h3 class="text-[16px] font-bold text-surface-800">{{ currentStep.title }}</h3>
            </div>
            <p class="text-[13px] text-surface-600 leading-relaxed whitespace-pre-line">{{ currentStep.content }}</p>

            <!-- Action buttons grid (action hub page) -->
            <div v-if="currentStep.actions?.length" class="mt-4 grid grid-cols-3 gap-2">
              <button v-for="a in currentStep.actions" :key="a.path"
                @click="goTo(a.path)"
                class="flex flex-col items-center justify-center gap-1.5 py-2.5 px-2 rounded-xl border transition-colors"
                :class="a.primary ? 'border-brand-300 bg-brand-50 text-brand-700 hover:bg-brand-100' : 'border-surface-200 text-surface-600 hover:border-brand-300 hover:bg-surface-50 hover:text-brand-600'">
                <AppIcon :name="a.icon" size="16" />
                <span class="text-[11px] font-medium">{{ a.label }}</span>
              </button>
            </div>

            <!-- Tips -->
            <div v-if="currentStep.tips?.length" class="mt-4 space-y-1.5">
              <p v-for="(t, i) in currentStep.tips" :key="i" class="text-[11px] text-surface-400 leading-relaxed flex items-start gap-1.5">
                <AppIcon name="lightbulb" size="11" class="flex-shrink-0 mt-0.5 text-amber-400" />
                {{ t }}
              </p>
            </div>
          </div>

          <!-- Footer -->
          <div class="flex items-center justify-between px-6 pb-5 pt-1">
            <button @click="skip" class="text-[12px] text-surface-400 hover:text-surface-600 transition-colors">跳过</button>
            <div class="flex items-center gap-2">
              <button v-if="!isFirst" @click="prev"
                class="px-3 py-1.5 rounded-lg text-[12px] text-surface-500 hover:bg-surface-100 transition-colors">
                上一步
              </button>
              <button v-if="!isLast" @click="next"
                class="px-4 py-1.5 rounded-lg text-[12px] font-medium bg-brand-600 text-white hover:bg-brand-700 transition-colors">
                下一步
              </button>
              <button v-else @click="finish"
                class="px-4 py-1.5 rounded-lg text-[12px] font-medium bg-brand-600 text-white hover:bg-brand-700 transition-colors">
                完成
              </button>
            </div>
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
