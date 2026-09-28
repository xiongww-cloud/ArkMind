<script setup lang="ts">
defineProps<{ expanded: boolean }>()
const emit = defineEmits<{ toggle: [] }>()
</script>

<template>
  <button
    @click="emit('toggle')"
    class="absolute right-0 top-1/2 -translate-y-1/2 z-10 w-4 h-8 flex items-center justify-center rounded-l border border-r-0 border-surface-200 bg-white hover:bg-surface-50 transition-colors shadow-sm"
    v-tooltip="expanded ? '收起附件' : '展开附件'"
  >
    <svg class="w-2.5 h-2.5 text-surface-400 transition-transform duration-200" :class="expanded ? 'rotate-180' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
      <path d="M15 18l-6-6 6-6" />
    </svg>
  </button>
</template>
