<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-all duration-300 ease-out"
      enter-from-class="opacity-0 translate-x-4"
      enter-to-class="opacity-100 translate-x-0"
      leave-active-class="transition-all duration-200 ease-in"
      leave-from-class="opacity-100 translate-x-0"
      leave-to-class="opacity-0 translate-x-4"
    >
      <div
        v-if="visible"
        class="fixed top-4 right-4 z-50 w-80 bg-white rounded-lg shadow-lg border border-blue-200 overflow-hidden"
      >
        <!-- Header -->
        <div class="flex items-center justify-between px-4 pt-3 pb-2">
          <div class="flex items-center gap-2">
            <svg class="w-4 h-4 text-blue-600 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
            </svg>
            <span class="text-sm font-medium text-slate-700">存储搬迁中</span>
          </div>
          <span class="text-xs text-slate-400">{{ progress.current }} / {{ progress.total }}</span>
        </div>

        <!-- Progress bar -->
        <div class="px-4 pb-1">
          <div class="h-1.5 bg-slate-100 rounded-full overflow-hidden">
            <div
              class="h-full bg-blue-500 rounded-full transition-all duration-500 ease-out"
              :style="{ width: percent + '%' }"
            />
          </div>
        </div>

        <!-- Direction -->
        <div class="px-4 pb-3 pt-1.5">
          <span class="text-[11px] text-slate-400">
            {{ directionLabel }}
          </span>
        </div>

        <!-- Completed state -->
        <div v-if="progress.status === 'completed'" class="px-4 pb-3 pt-1">
          <div class="flex items-center gap-1.5">
            <svg class="w-3.5 h-3.5 text-emerald-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12"/>
            </svg>
            <span class="text-xs text-emerald-600 font-medium">搬迁完成</span>
          </div>
        </div>

        <!-- Failed state -->
        <div v-if="progress.status === 'failed'" class="px-4 pb-3 pt-1">
          <div class="flex items-center gap-1.5">
            <svg class="w-3.5 h-3.5 text-red-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/>
            </svg>
            <span class="text-xs text-red-600 font-medium">搬迁失败</span>
          </div>
          <p v-if="progress.errorMessage" class="text-[11px] text-red-400 mt-1 line-clamp-2">
            {{ progress.errorMessage }}
          </p>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue'

const props = defineProps({
  visible: { type: Boolean, default: false },
  progress: {
    type: Object,
    default: () => ({
      status: 'idle',
      direction: '',
      total: 0,
      current: 0,
      errorMessage: '',
    }),
  },
})

const percent = computed(() => {
  if (props.progress.total === 0) return 0
  return Math.round((props.progress.current / props.progress.total) * 100)
})

const directionLabel = computed(() => {
  if (props.progress.direction === 'LOCAL_TO_OSS') return '本地 → 阿里云 OSS'
  if (props.progress.direction === 'OSS_TO_LOCAL') return '阿里云 OSS → 本地'
  return ''
})
</script>
