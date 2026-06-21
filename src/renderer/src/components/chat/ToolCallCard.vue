<script setup lang="ts">
import { ref } from 'vue'

const props = defineProps<{
  round: any
}>()

const expanded = ref(false)
const expandedResults = ref<Record<number, boolean>>({})
</script>

<template>
<div class="bg-white rounded-xl border border-[#e5e5ea] overflow-hidden shadow-sm">
  <!-- Collapsed bar -->
  <div @click="expanded = !expanded" class="flex items-center gap-2.5 px-3.5 py-2.5 cursor-pointer select-none hover:bg-[#f8f8fa] transition-colors">
    <div class="w-1.5 h-1.5 rounded-full bg-[#6b4eff] flex-shrink-0"></div>
    <span class="text-[11px] font-medium text-[#3a3a3c]">第 {{ round.round }} 轮 · {{ round.tools?.length || 0 }} 个工具</span>
    <div class="flex items-center gap-1.5 ml-auto">
      <span v-for="(tc, tci) in round.tools?.slice(0, 3)" :key="tci" class="text-[9px] px-1.5 py-0.5 rounded-full bg-[#f0f0f5] text-[#86868b] truncate max-w-[80px]">{{ tc.tool?.replace('ext_tool_', '') }}</span>
      <svg class="w-3 h-3 text-[#c7c7cc] transition-transform duration-200" :class="expanded ? 'rotate-180' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
    </div>
  </div>

  <!-- Expanded content -->
  <div v-if="expanded" class="border-t border-[#e5e5ea] divide-y divide-[#f0f0f5]">
    <!-- Thought -->
    <div v-if="round.thought" class="px-3.5 py-3 text-[12px] text-[#6e6e73] leading-relaxed bg-[#faf8f0]">
      <span class="font-medium text-[#8b7355]">🧠 思考</span><br>{{ round.thought }}
    </div>

    <!-- Tool results -->
    <div v-for="(tc, tci) in round.tools" :key="tci" class="px-3.5 py-2.5">
      <div class="flex items-center gap-1.5 mb-1">
        <span class="w-1 h-1 rounded-full bg-[#34c759] flex-shrink-0"></span>
        <span class="text-[11px] font-mono font-medium text-[#3a3a3c]">{{ tc.tool?.replace('ext_tool_', '') }}</span>
      </div>
      <div class="bg-[#f8f8fa] rounded-lg p-2.5 mb-1.5 font-mono text-[10px] text-[#6e6e73] overflow-x-auto leading-relaxed whitespace-pre-wrap">{{ JSON.stringify(tc.args, null, 1) }}</div>
      <div v-if="tc.result" @click.stop="expandedResults[tci] = !expandedResults[tci]" class="cursor-pointer select-none">
        <div class="flex items-center gap-1 text-[9px] text-[#86868b] hover:text-[#6b4eff] transition-colors">
          <svg class="w-3 h-3 transition-transform" :class="expandedResults[tci] ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 5l7 7-7 7"/></svg>
          结果 ({{ tc.result_len || tc.result.length }} 字符)
        </div>
        <div v-if="expandedResults[tci]" class="mt-1 bg-[#f8f8fa] rounded-lg p-2.5 font-mono text-[10px] text-[#6e6e73] overflow-x-auto max-h-60 overflow-y-auto leading-relaxed whitespace-pre-wrap">{{ tc.result }}</div>
      </div>
    </div>
  </div>
</div>
</template>
