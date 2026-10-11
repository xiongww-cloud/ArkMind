<script setup lang="ts">
import { watch } from 'vue'
import { useAgentRunStore } from '@/stores/agentRun'
import AppIcon from '@/components/common/AppIcon.vue'
import type { UiPlanStep } from '@/utils/uiBlocks'

const props = defineProps<{ title: string; steps: UiPlanStep[]; msgId: string; live?: boolean }>()
const agentRun = useAgentRunStore()

// 仅本轮 live 卡（会话最后一条消息）向 store 上报结构并显示实时进度；历史卡静态渲染
watch(() => [props.live, props.steps], () => {
  if (props.live) agentRun.setPlan(props.steps, props.msgId)
}, { immediate: true, deep: true })

const stateOf = (i: number): 'pending' | 'active' | 'done' => {
  if (!props.live) return 'pending'
  const p = agentRun.plan
  return p && p.ownerMsgId === props.msgId ? p.states[i] : 'pending'
}
const live = () => props.live && !!agentRun.plan && agentRun.plan.ownerMsgId === props.msgId
const doneCount = () => live() ? (agentRun.plan!.states.filter(s => s === 'done').length) : 0
</script>

<template>
  <div class="annotate-skip my-1.5 rounded-xl border border-[#e5e5ea] bg-[#fafafa] px-3 py-2.5">
    <div class="flex items-center gap-1.5 mb-1.5">
      <AppIcon name="list" size="12" class="text-[#2563eb]" />
      <span class="text-[11px] font-semibold text-[#3a3a3c]">{{ title }}</span>
      <span v-if="live()" class="ml-auto text-[10px] text-[#8e8e93] font-mono">{{ doneCount() }}/{{ steps.length }}</span>
    </div>
    <div class="space-y-1">
      <div v-for="(s, i) in steps" :key="i" class="flex items-center gap-2 text-[11px] leading-snug">
        <span v-if="stateOf(i) === 'pending'" class="w-3.5 h-3.5 flex items-center justify-center flex-shrink-0">
          <span class="w-2 h-2 rounded-full border border-[#d1d1d6]"></span>
        </span>
        <span v-else-if="stateOf(i) === 'active'" class="w-3.5 h-3.5 flex items-center justify-center flex-shrink-0">
          <span class="w-2.5 h-2.5 rounded-full bg-[#2563eb]/25 border-2 border-[#2563eb] animate-pulse"></span>
        </span>
        <span v-else class="w-3.5 h-3.5 rounded-full bg-emerald-100 text-emerald-600 flex items-center justify-center flex-shrink-0">
          <svg class="w-2 h-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
        </span>
        <span :class="stateOf(i) === 'done' ? 'text-[#8e8e93] line-through decoration-[#d1d1d6]' : stateOf(i) === 'active' ? 'text-[#3a3a3c] font-medium' : 'text-[#5b5b60]'">{{ s.title }}</span>
        <span v-if="s.agent" class="text-[9px] px-1.5 py-[1px] rounded-full bg-[#f2f2f7] text-[#8e8e93] flex-shrink-0">{{ s.agent }}</span>
      </div>
    </div>
  </div>
</template>
