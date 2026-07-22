<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

const props = defineProps<{
  stages: Array<{ key: string; label: string; color: string; count: number; budget: number }>
  maxCount: number
}>()

const router = useRouter()

function formatBudget(v: number): string {
  if (v >= 10000) return `${(v / 10000).toFixed(1)}亿`
  if (v >= 1) return `${v.toLocaleString()}万`
  return v > 0 ? `${v}万` : ''
}

function barWidth(count: number): string {
  return `${Math.max((count / props.maxCount) * 100, 4)}%`
}
</script>

<template>
  <div class="space-y-3.5">
    <div
      v-for="s in stages"
      :key="s.key"
      class="group cursor-pointer"
      @click="router.push('/projects')"
    >
      <div class="flex items-center justify-between mb-1">
        <div class="flex items-center gap-2">
          <div class="w-2 h-2 rounded-full" :style="{ backgroundColor: s.color }"></div>
          <span class="text-[12px] text-surface-600 font-medium">{{ s.label }}</span>
        </div>
        <div class="flex items-center gap-3">
          <span class="text-[11px] text-surface-400">{{ s.count }}个</span>
          <span class="text-[11px] text-surface-500 font-medium">¥{{ formatBudget(s.budget) }}</span>
        </div>
      </div>
      <div class="w-full h-2.5 bg-surface-100 rounded-full overflow-hidden">
        <div
          class="h-full rounded-full transition-all duration-500 group-hover:brightness-110"
          :style="{ width: barWidth(s.count), backgroundColor: s.color }"
        ></div>
      </div>
    </div>
  </div>
</template>
