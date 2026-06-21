<script setup lang="ts">
defineProps<{ expanded: boolean; drawerWidth?: number }>()
defineEmits<{ toggle: [] }>()
</script>

<template>
  <div
    class="absolute top-1/2 -translate-y-1/2 flex flex-col items-center z-40 transition-all duration-200"
    :style="{ right: (drawerWidth || 0) + 'px' }"
  >
    <button
      @click="$emit('toggle')"
      class="w-[18px] h-[36px] bg-brand-500 hover:bg-brand-600 rounded-l-full flex items-center justify-center shadow-md transition-colors text-white"
      :title="expanded ? '收起附件面板' : '展开附件面板'"
    >
      <svg class="w-2.5 h-2.5 transition-transform duration-200" :class="expanded ? 'rotate-180' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M9 5l7 7-7 7"/></svg>
    </button>
  </div>
</template>
