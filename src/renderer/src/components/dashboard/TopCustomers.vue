<script setup lang="ts">
const props = defineProps<{
  customers: Array<{ name: string; budget: number }>
  maxBudget: number
}>()

function barWidth(budget: number): string {
  return `${Math.max((budget / props.maxBudget) * 100, 4)}%`
}

function formatBudget(v: number): string {
  if (v >= 10000) return `${(v / 10000).toFixed(1)}亿`
  if (v >= 1) return `${v.toLocaleString()}万`
  return ''
}
</script>

<template>
  <div v-if="customers.length" class="space-y-3">
    <div v-for="(c, i) in customers" :key="c.name" class="group">
      <div class="flex items-center justify-between mb-1">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-medium text-surface-300 w-3">{{ i + 1 }}</span>
          <span class="text-[12px] text-surface-700 font-medium truncate">{{ c.name }}</span>
        </div>
        <span class="text-[11px] text-surface-500 font-medium">¥{{ formatBudget(c.budget) }}</span>
      </div>
      <div class="w-full h-3.5 bg-surface-100 rounded-lg overflow-hidden">
        <div
          class="h-full rounded-lg bg-brand-400 transition-all duration-500 group-hover:bg-brand-500"
          :style="{ width: barWidth(c.budget) }"
        ></div>
      </div>
    </div>
  </div>
  <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无关联客户</p>
</template>
