<script setup lang="ts">
import AppIcon from '@/components/common/AppIcon.vue'

defineProps<{
  icon: string
  iconColor?: string
  label: string
  value: string | number
  sub?: string
  delta?: number
  urgent?: boolean
}>()
</script>

<template>
  <div
    class="flex items-start gap-3 bg-white rounded-2xl border border-surface-200 p-4 shadow-sm hover:shadow-md transition-all duration-200 min-h-[110px]"
  >
    <div
      class="w-10 h-10 rounded-xl flex items-center justify-center flex-shrink-0"
      :class="iconColor || 'bg-brand-50 text-brand-600'"
    >
      <AppIcon :name="icon" size="18" class="text-current" />
    </div>
    <div class="flex-1 min-w-0">
      <p class="text-[11px] text-surface-400 mb-0.5">{{ label }}</p>
      <p class="text-[22px] font-bold text-surface-900 leading-tight tracking-tight" :class="{ 'text-red-600 animate-pulse': urgent }">
        {{ value }}
      </p>
      <div class="flex items-center gap-1.5 mt-1">
        <span v-if="sub" class="text-[11px] text-surface-400">{{ sub }}</span>
        <span
          v-if="delta !== undefined && delta !== 0"
          class="text-[10px] font-medium px-1.5 py-0.5 rounded-full"
          :class="delta >= 0 ? 'bg-emerald-50 text-emerald-600' : 'bg-red-50 text-red-500'"
        >
          {{ delta >= 0 ? '↑' : '↓' }}{{ Math.abs(delta) }}%
        </span>
      </div>
    </div>
  </div>
</template>
