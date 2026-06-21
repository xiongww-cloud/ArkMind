<script setup lang="ts">
import { useConfirmDialog } from '@/composables/useConfirm'

const { state, handleConfirm, handleCancel } = useConfirmDialog()
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="state.show" class="fixed inset-0 z-[9999] flex items-center justify-center">
        <!-- Backdrop -->
        <div class="absolute inset-0 bg-black/20" />

        <!-- Dialog -->
        <div class="relative bg-white rounded-xl shadow-lg border border-slate-100 w-full max-w-sm mx-4 p-6 z-10">
          <h3 class="text-sm font-semibold text-slate-800 mb-2">{{ state.title }}</h3>
          <p class="text-sm text-slate-500 leading-relaxed mb-5">{{ state.message }}</p>

          <div class="flex items-center justify-end gap-2">
            <button
              v-if="state.type === 'confirm'"
              @click="handleCancel"
              class="flex items-center gap-1.5 py-1 px-2 rounded text-[12px] text-slate-400 bg-transparent border-none hover:text-slate-600 hover:bg-slate-50 transition-colors"
            >
              <span>{{ state.cancelText }}</span>
            </button>
            <button
              @click="handleConfirm"
              class="flex items-center gap-1.5 py-1 px-3 rounded text-[12px] text-white transition-colors"
              :class="state.danger ? 'bg-red-600 hover:bg-red-700' : 'bg-blue-600 hover:bg-blue-700'"
            >
              <span>{{ state.confirmText }}</span>
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.15s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>
