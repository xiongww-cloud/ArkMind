<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useAgentRunStore } from '@/stores/agentRun'
import AppIcon from '@/components/common/AppIcon.vue'

/** 执行计划 Dock：常驻输入框上方，滚动永不离场。
 *  纯快照投影：agent:plan 有值即显示、null 即消失——生命周期完全由后端 ext_tool_plan 管理（create/update/delete）。
 *  手动关闭仅在 run 空闲时提供（执行中不出现 ×，防误点中断 AI 的计划上下文）；清空走 plan_clear 落 session_state，跨重启不复活。
 *  展开偏好跨轮记忆（localStorage）。 */
const props = defineProps<{ running?: boolean }>()
const agentRun = useAgentRunStore()
const expanded = ref(localStorage.getItem('planDockExpanded') !== '0')

function toggle() {
  expanded.value = !expanded.value
  localStorage.setItem('planDockExpanded', expanded.value ? '1' : '0')
}
async function clearPlan() {
  const sid = agentRun.planSid || agentRun.runSid
  if (!sid || props.running) return
  try { await invoke('plan_clear', { sid }) } catch { /* 会话已删等竞态：本地兜底消失 */ agentRun.clearPlan() }
}
const doneCount = () => agentRun.plan ? agentRun.plan.steps.filter(s => s.state === 'done').length : 0
const activeStep = () => agentRun.plan?.steps.find(s => s.state === 'active') || null
const allDone = () => !!agentRun.plan && agentRun.plan.steps.length > 0 && doneCount() === agentRun.plan.steps.length
</script>

<template>
  <div v-if="agentRun.plan" class="annotate-skip mb-2 rounded-xl border border-surface-200 bg-white shadow-sm overflow-hidden">
      <div type="button" role="button" class="w-full flex items-center gap-2 px-3 py-1.5 cursor-pointer select-none" @click="toggle">
        <span v-if="allDone()" class="w-4 h-4 rounded-full bg-emerald-100 text-emerald-600 flex items-center justify-center flex-shrink-0">
          <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
        </span>
        <span v-else class="relative flex w-2.5 h-2.5 items-center justify-center flex-shrink-0">
          <span class="absolute w-2.5 h-2.5 rounded-full bg-brand-500/30 animate-ping"></span>
          <span class="w-1.5 h-1.5 rounded-full bg-brand-600"></span>
        </span>
        <span class="text-[11px] font-semibold text-surface-700 tabular-nums flex-shrink-0">{{ doneCount() }}/{{ agentRun.plan.steps.length }}</span>
        <span class="text-[11px] text-surface-500 truncate flex-1 text-left">{{ activeStep()?.title || (allDone() ? '全部步骤完成' : agentRun.plan.title) }}</span>
        <span v-if="activeStep()?.agent" class="text-[9px] px-1.5 py-[1px] rounded-full bg-brand-50 text-brand-600 flex-shrink-0 max-w-[90px] truncate">{{ activeStep()!.agent }}</span>
        <button v-if="!running" type="button" @click.stop="clearPlan" class="w-4 h-4 flex items-center justify-center rounded text-surface-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0" v-tooltip="'关闭执行计划'">
          <AppIcon name="x" size="10" />
        </button>
        <AppIcon name="chevron-right" class="w-3 h-3 text-surface-400 transition-transform flex-shrink-0" :class="expanded ? 'rotate-90' : ''" />
      </div>
      <div v-if="expanded" class="px-3 pb-2 space-y-1 border-t border-surface-100 pt-1.5">
        <div v-for="(st, i) in agentRun.plan.steps" :key="i" class="flex items-center gap-2 text-[11px] leading-snug">
          <span v-if="st.state === 'pending'" class="w-3.5 h-3.5 flex items-center justify-center flex-shrink-0"><span class="w-2 h-2 rounded-full border border-surface-300"></span></span>
          <span v-else-if="st.state === 'active'" class="w-3.5 h-3.5 flex items-center justify-center flex-shrink-0"><span class="w-2.5 h-2.5 rounded-full bg-brand-500/25 border-2 border-brand-600 animate-pulse"></span></span>
          <span v-else class="w-3.5 h-3.5 rounded-full bg-emerald-100 text-emerald-600 flex items-center justify-center flex-shrink-0">
            <svg class="w-2 h-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
          </span>
          <span class="min-w-0 truncate" :class="st.state === 'done' ? 'text-surface-400 line-through decoration-surface-300' : st.state === 'active' ? 'text-surface-800 font-medium' : 'text-surface-600'">{{ st.title }}</span>
          <span v-if="st.agent" class="text-[9px] px-1.5 py-[1px] rounded-full bg-surface-100 text-surface-500 flex-shrink-0 ml-auto max-w-[90px] truncate">{{ st.agent }}</span>
        </div>
      </div>
  </div>
</template>
