<script setup lang="ts">
import { useToast } from '@/composables/useToast'

const { toasts } = useToast()

const typeStyles = {
  success: 'bg-emerald-500 text-white',
  error: 'bg-red-500 text-white',
  info: 'bg-slate-800 text-white',
}
</script>

<template>
  <Teleport to="body">
    <TransitionGroup
      name="toast"
      tag="div"
      class="fixed bottom-6 right-6 z-[100] flex flex-col items-end gap-2 pointer-events-none"
    >
      <div
        v-for="t in toasts"
        :key="t.id"
        class="pointer-events-auto rounded-lg px-4 py-2 text-sm font-medium shadow-lg transition-all"
        :class="typeStyles[t.type] || typeStyles.info"
      >
        {{ t.message }}
      </div>
    </TransitionGroup>
  </Teleport>
</template>

<style scoped>
.toast-enter-active {
  transition: all 0.2s ease-out;
}
.toast-leave-active {
  transition: all 0.15s ease-in;
}
.toast-enter-from {
  opacity: 0;
  transform: translateY(20px);
}
.toast-leave-to {
  opacity: 0;
  transform: translateY(10px);
}
</style>
