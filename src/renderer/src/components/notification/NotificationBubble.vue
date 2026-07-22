<script setup lang="ts">
import { ref, onMounted } from 'vue'
import type { Notification } from '@/api/notification-client'

defineProps<{
  notification: Notification
}>()

const emit = defineEmits<{
  close: []
  click: []
}>()

const visible = ref(true)

onMounted(() => {
  setTimeout(() => {
    visible.value = false
    setTimeout(() => emit('close'), 300)
  }, 4000)
})

function onClick() {
  visible.value = false
  setTimeout(() => emit('click'), 200)
}

function onClose() {
  visible.value = false
  setTimeout(() => emit('close'), 200)
}

const priorityColor: Record<string, string> = {
  high: 'border-red-300 bg-red-50',
  medium: 'border-amber-300 bg-amber-50',
  low: 'border-surface-200 bg-white',
}
</script>

<template>
  <Transition name="bubble">
    <div
      v-if="visible"
      @click="onClick"
      class="fixed bottom-20 right-6 w-72 rounded-xl border shadow-lg cursor-pointer
             transition-all duration-300 hover:shadow-xl z-[9999]"
      :class="priorityColor[notification.priority] || priorityColor.medium"
    >
      <div class="flex items-start gap-2 p-3">
        <span class="text-[14px] mt-0.5 flex-shrink-0">
          {{ notification.type === 'bid_deadline' ? '🎯' : notification.type === 'todo_due' ? '⏰' : notification.type === 'project_stale' ? '📌' : '💡' }}
        </span>
        <div class="flex-1 min-w-0">
          <p class="text-[12px] font-semibold text-surface-800 truncate">{{ notification.title }}</p>
          <p class="text-[11px] text-surface-500 mt-0.5 line-clamp-2">{{ notification.content }}</p>
        </div>
        <button @click.stop="onClose" class="text-surface-400 hover:text-surface-600 flex-shrink-0">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M18 6L6 18M6 6l12 12"/>
          </svg>
        </button>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.bubble-enter-active, .bubble-leave-active {
  transition: all 0.3s ease;
}
.bubble-enter-from {
  opacity: 0;
  transform: translateY(20px) scale(0.95);
}
.bubble-leave-to {
  opacity: 0;
  transform: translateY(-10px) scale(0.95);
}
</style>
