<script setup lang="ts">
import { reactive, computed } from 'vue'
import type { ClarificationData, ClarificationQuestion } from '@/composables/useAgentChat'
import { buildClarificationReply, mergeClarificationAnswer } from '@/composables/useAgentChat'

const props = defineProps<{
  clarification: ClarificationData
  /** 非会话最后一条消息 → 已回答态：折叠选择器只留提示（防刷新后重复提交） */
  answered?: boolean
}>()
const emit = defineEmits<{ submit: [text: string] }>()

const answers = reactive<Record<string, string | string[]>>({})
/** 每道题的自定义答案文本（选择题可选补充；单选提交时覆盖点选，多选作为附加项） */
const custom = reactive<Record<string, string>>({})
for (const q of props.clarification.questions) {
  answers[q.question_id] = q.type === 'multiple_choice' ? [] : ''
  custom[q.question_id] = ''
}

const disabled = computed(() => !!props.answered)

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
  }
}
function textOf(q: ClarificationQuestion): string {
  const a = answers[q.question_id]
  return typeof a === 'string' ? a : ''
}
function setText(q: ClarificationQuestion, v: string) {
  if (!disabled.value) answers[q.question_id] = v
}

const canSubmit = computed(() => props.clarification.questions.some((q) => {
  const a = merged(q)
  return Array.isArray(a) ? a.length > 0 : a.trim().length > 0
}))

function merged(q: ClarificationQuestion): string | string[] {
  return mergeClarificationAnswer(q, answers[q.question_id], custom[q.question_id])
}

function submit() {
  if (disabled.value || !canSubmit.value) return
  const finalAnswers: Record<string, string | string[]> = {}
  for (const q of props.clarification.questions) finalAnswers[q.question_id] = merged(q)
  emit('submit', buildClarificationReply(props.clarification, finalAnswers))
}
function skipWithDefaults() {
  if (disabled.value) return
  emit('submit', buildClarificationReply(props.clarification, {}))
}
</script>

<template>
  <div
    class="rounded-xl border bg-white shadow-sm overflow-hidden w-full max-w-[420px]"
    :class="answered ? 'border-[#e5e5ea] opacity-90' : 'border-brand-300'"
  >
    <div class="px-3 py-2 bg-brand-50/70 border-b" :class="answered ? 'border-[#e5e5ea]' : 'border-brand-100'">
      <span class="text-[12px] font-medium text-brand-800">需要补充关键信息</span>
    </div>
    <div class="px-3 py-2.5 space-y-2.5">
      <p v-if="clarification.reason" class="text-[11px] text-surface-500 leading-relaxed">{{ clarification.reason }}</p>
      <div v-for="(q, i) in clarification.questions" :key="q.question_id">
        <p class="text-[12px] font-medium text-surface-700 mb-1.5 leading-snug">{{ i + 1 }}. {{ q.prompt }}</p>
        <div v-if="q.type !== 'text'">
          <div
            class="flex flex-wrap gap-1.5 transition-opacity"
            :class="q.type === 'single_choice' && custom[q.question_id].trim() ? 'opacity-40' : ''"
          >
            <button
              v-for="opt in q.options" :key="opt"
              type="button"
              @click="toggle(q, opt)"
              :disabled="disabled"
              class="px-2.5 py-1 rounded-lg text-[11px] border transition-colors"
              :class="isPicked(q, opt)
                ? 'border-brand-500 bg-brand-600 text-white'
                : 'border-surface-200 bg-white text-surface-600' + (disabled ? '' : ' hover:border-brand-300 hover:bg-brand-50/40')"
            >{{ opt }}</button>
          </div>
          <input
            v-model="custom[q.question_id]"
            :disabled="disabled"
            type="text"
            class="mt-1.5 w-full border border-surface-200 rounded-lg px-2.5 py-1 text-[11px] outline-none focus:border-brand-400 disabled:bg-surface-50"
            :placeholder="q.type === 'multiple_choice' ? '补充其他答案（将并入已选项）...' : '或输入自定义答案（将覆盖上方选择）...'"
          />
        </div>
        <input
          v-else
          :value="textOf(q)"
          :disabled="disabled"
          type="text"
          class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400 disabled:bg-surface-50"
          placeholder="输入答复..."
          @input="(e: any) => setText(q, e.target.value)"
        />
      </div>
      <div v-if="!answered" class="flex items-center justify-end gap-2.5 pt-0.5">
        <button type="button" class="text-[11px] text-surface-400 hover:text-surface-600 transition-colors" @click="skipWithDefaults">按默认假设继续</button>
        <button
          type="button"
          :disabled="!canSubmit"
          class="px-3 py-1.5 rounded-lg text-[11px] font-medium transition-colors"
          :class="canSubmit ? 'bg-brand-600 text-white hover:bg-brand-700 shadow-sm' : 'bg-surface-100 text-surface-400 cursor-not-allowed'"
          @click="submit"
        >确认并继续</button>
      </div>
      <p v-else class="text-[11px] text-surface-400">已答复，结果见下方对话</p>
    </div>
  </div>
</template>
