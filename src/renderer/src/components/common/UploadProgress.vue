<script setup lang="ts">
import { useUploadProgress } from '@/composables/useUploadProgress'

const { tasks, visible } = useUploadProgress()
</script>

<template>
  <Transition name="slide-up">
    <div
      v-if="visible"
      class="fixed bottom-4 left-1/2 -translate-x-1/2 z-[9999] w-[260px] max-h-[180px] overflow-y-auto bg-white rounded-xl shadow-xl border border-gray-100 p-3 space-y-2"
    >
      <div v-for="t in tasks" :key="t.id" class="flex flex-col gap-1">
        <div class="flex items-center justify-between">
          <span class="text-[11px] text-gray-700 truncate flex-1 mr-2">{{ t.name }}</span>
          <span
            class="text-[10px] font-medium flex-shrink-0"
            :class="t.status === 'done' ? 'text-emerald-500' : t.status === 'error' ? 'text-red-500' : 'text-brand-600'"
          >{{ t.status === 'done' ? '完成' : t.status === 'error' ? '失败' : '上传中' }}</span>
        </div>
        <div class="h-1 bg-gray-100 rounded-full overflow-hidden">
          <div
            class="h-full rounded-full transition-all duration-500"
            :class="t.status === 'error' ? 'bg-red-400' : t.status === 'done' ? 'bg-emerald-400' : 'bg-brand-500'"
            :style="{ width: t.status === 'uploading' ? '60%' : '100%' }"
          />
        </div>
        <p v-if="t.error" class="text-[10px] text-red-400 truncate">{{ t.error }}</p>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.slide-up-enter-active, .slide-up-leave-active { transition: all 0.3s ease; }
.slide-up-enter-from, .slide-up-leave-to { opacity: 0; transform: translateY(12px); }
</style>
