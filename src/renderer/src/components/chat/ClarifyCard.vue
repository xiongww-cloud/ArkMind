<script setup lang="ts">
import { reactive, computed } from 'vue'
import type { ClarificationData, ClarificationQuestion } from '@/composables/useAgentChat'
import { buildClarificationReply, mergeClarificationAnswer } from '@/composables/useAgentChat'

const props = defineProps<{
  clarification: ClarificationData
  /** 非会话最后一条消息 → 已回答态：显示每题最终决定（答案从用户答复消息解析，刷新不丢） */
  answered?: boolean
  /** answered 时传入紧随其后的用户答复消息文本 */
  replyText?: string
  /** 用户选择忽略：不答复不续跑，卡片定格为「已忽略」（metadata 持久化，重启不丢） */
  ignored?: boolean
}>()
const emit = defineEmits<{ submit: [text: string], ignore: [] }>()

const SKIP_TEXT = '暂不补充这些信息，请按最合理的默认假设继续，并在结果中括注说明采用的假设。'

/** 从 buildClarificationReply 格式解析每题答案：按已知题干定位，答案=本题干冒号后到下一题干前 */
const parsedReply = computed(() => {
  if (!props.answered || !props.replyText) return null
  const t = props.replyText
  if (t === SKIP_TEXT || t.startsWith('暂不补充这些信息')) return { skipped: true, answers: {} as Record<string, string> }
  const marks: { q: ClarificationQuestion, at: number }[] = []
  for (const q of props.clarification.questions) {
    const key = `${q.prompt.replace(/[？?]\s*$/, '')}：`
    const at = t.indexOf(key)
    if (at >= 0) marks.push({ q, at })
  }
  if (!marks.length) return null
  marks.sort((a, b) => a.at - b.at)
  const answers: Record<string, string> = {}
  marks.forEach((m, i) => {
    const start = m.at + `${m.q.prompt.replace(/[？?]\s*$/, '')}：`.length
    const end = i + 1 < marks.length ? marks[i + 1].at : t.length
    answers[m.q.question_id] = t.slice(start, end).replace(/；?\s*\d+\.\s*$/, '').replace(/；\s*$/, '').replace(/。\s*请继续执行。\s*$/, '').trim()
  })
  return { skipped: false, answers }
})

const answers = reactive<Record<string, string | string[]>>({})
const custom = reactive<Record<string, string>>({})
const customOpen = reactive<Record<string, boolean>>({})
for (const q of props.clarification.questions) {
  answers[q.question_id] = q.type === 'multiple_choice' ? [] : ''
  custom[q.question_id] = ''
  customOpen[q.question_id] = false
}
const disabled = computed(() => !!props.answered || !!props.ignored)

function isPicked(q: ClarificationQuestion, opt: string): boolean {
  const a = answers[q.question_id]
  return q.type === 'multiple_choice' ? Array.isArray(a) && a.includes(opt) : a === opt
}
function toggle(q: ClarificationQuestion, opt: string) {
  if (disabled.value) return
  if (q.type === 'multiple_choice') {
    const arr = (answers[q.question_id] as string[]) || []
    const i = arr.indexOf(opt)
    if (i >= 0) arr.splice(i, 1)
    else arr.push(opt)
    answers[q.question_id] = arr
  } else {
    answers[q.question_id] = answers[q.question_id] === opt ? '' : opt
    if (answers[q.question_id]) custom[q.question_id] = ''
  }
}
function openCustom(q: ClarificationQuestion) {
  customOpen[q.question_id] = true
  if (q.type === 'single_choice') answers[q.question_id] = ''
}
function merged(q: ClarificationQuestion): string | string[] {
  return mergeClarificationAnswer(q, answers[q.question_id], custom[q.question_id])
}
const anyAnswered = computed(() => props.clarification.questions.some((q) => {
  const a = merged(q)
  return Array.isArray(a) ? a.length > 0 : String(a).trim().length > 0
}))
function submit() {
  if (disabled.value || !anyAnswered.value) return
  const finalAnswers: Record<string, string | string[]> = {}
  for (const q of props.clarification.questions) finalAnswers[q.question_id] = merged(q)
  emit('submit', buildClarificationReply(props.clarification, finalAnswers))
}
/// 忽略=这题不答：不发消息不续跑（原「先不补充，按默认假设继续」按钮并入此路径——
/// 模板话术既污染会话历史又语义冗余；AI prompt 本就允许可默认事项自行采用假设，
/// 用户之后想推进直接发消息即可）。SKIP_TEXT 解析保留：旧历史里已生成的答复仍能显示已确认态。
function ignore() {
  if (disabled.value) return
  emit('ignore')
}
</script>

<template>
  <div class="rounded-xl border bg-white overflow-hidden w-full"
    :class="answered || ignored ? 'border-surface-200' : 'border-brand-300 shadow-sm'">
    <div class="px-3.5 pt-2.5 pb-1 flex items-center gap-1.5">
      <span v-if="ignored" class="w-4 h-4 rounded-full bg-surface-100 text-surface-500 text-[10px] font-bold flex items-center justify-center flex-shrink-0">–</span>
      <span v-else-if="!answered" class="w-4 h-4 rounded-full bg-brand-100 text-brand-700 text-[10px] font-bold flex items-center justify-center flex-shrink-0">?</span>
      <span v-else class="w-4 h-4 rounded-full bg-emerald-50 text-emerald-600 text-[10px] flex items-center justify-center flex-shrink-0">✓</span>
      <span class="text-[12px] font-semibold text-surface-700">{{ ignored ? '已忽略' : answered ? '已确认' : 'AI 需要你的决定' }}</span>
    </div>
    <div class="px-3.5 pb-3 space-y-3">
      <p v-if="clarification.reason && !answered && !ignored" class="text-[11px] text-surface-500 leading-relaxed -mt-0.5">{{ clarification.reason }}</p>

      <!-- 已忽略：不答复不续跑，卡片定格；后续对话由用户主动发消息开启 -->
      <p v-if="ignored" class="text-[11px] text-surface-400 leading-relaxed -mt-0.5">已忽略此确认，未回复任何内容；想继续直接发消息即可（不补充时 AI 会按合理默认假设推进）。</p>
      <template v-else-if="answered">
        <p v-if="parsedReply?.skipped" class="text-[11px] text-surface-500">用户选择按默认假设继续（结果中已括注采用的假设）</p>
        <template v-else-if="parsedReply">
          <div v-for="(q, i) in clarification.questions" :key="q.question_id">
            <p class="text-[11px] text-surface-400 leading-snug">{{ i + 1 }}. {{ q.prompt }}</p>
            <p v-if="parsedReply.answers[q.question_id]" class="text-[12px] text-surface-800 font-medium mt-0.5">→ {{ parsedReply.answers[q.question_id] }}</p>
          </div>
        </template>
        <p v-else class="text-[11px] text-surface-400">已答复</p>
      </template>

      <!-- 作答态 -->
      <template v-else>
        <div v-for="(q, i) in clarification.questions" :key="q.question_id" class="pt-0.5">
          <p class="text-[12px] font-medium text-surface-700 mb-1.5 leading-snug">{{ i + 1 }}. {{ q.prompt }}</p>
          <input v-if="q.type === 'text'" v-model="answers[q.question_id]" type="text"
            class="w-full border border-surface-200 rounded-md px-2.5 py-1 text-[11px] outline-none focus:border-brand-400" placeholder="输入答复..." />
          <div v-else class="space-y-1">
            <button v-for="opt in q.options" :key="opt" type="button" @click="toggle(q, opt)"
              class="w-full text-left px-2.5 py-1 rounded-md border text-[11px] flex items-center gap-2 transition-colors"
              :class="isPicked(q, opt) ? 'border-brand-500 bg-brand-50 text-brand-800 font-medium' : 'border-surface-200 text-surface-600 hover:border-brand-300 hover:bg-brand-50/40'">
              <span class="w-3 h-3 rounded-full border flex items-center justify-center flex-shrink-0"
                :class="isPicked(q, opt) ? 'border-brand-600 bg-brand-600' : 'border-surface-300'">
                <svg v-if="isPicked(q, opt)" class="w-2 h-2 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
              </span>
              {{ opt }}
            </button>
            <input v-if="customOpen[q.question_id] || custom[q.question_id]" v-model="custom[q.question_id]" type="text"
              class="w-full border border-surface-200 rounded-md px-2.5 py-1 text-[11px] outline-none focus:border-brand-400"
              :placeholder="q.type === 'multiple_choice' ? '补充其他答案（并入已选项）...' : '自定义答案...'" autofocus />
            <button v-else type="button" @click="openCustom(q)" class="text-[10px] text-surface-400 hover:text-brand-600 px-1">+ 其他（自行填写）</button>
          </div>
        </div>
        <div class="flex items-center justify-end gap-2.5 pt-0.5">
          <button type="button" @click="ignore" class="text-[11px] text-surface-400 hover:text-surface-600 transition-colors">忽略</button>
          <button type="button" :disabled="!anyAnswered" @click="submit"
            :class="anyAnswered ? 'btn-primary text-xs' : 'inline-flex items-center h-6 px-2.5 rounded-md text-xs font-medium bg-surface-100 text-surface-400 cursor-not-allowed'"
          >确认并继续</button>
        </div>
      </template>
    </div>
  </div>
</template>
