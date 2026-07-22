<script setup lang="ts">
defineProps<{
  message?: string
  icon?: string
  size?: 'sm' | 'md'
}>()
</script>

<template>
  <div class="flex flex-col items-center justify-center py-8 select-none" :class="size === 'sm' ? 'py-4' : 'py-8'">
    <AppIcon v-if="icon" :name="icon" :class="size === 'sm' ? 'w-6 h-6 text-surface-300' : 'w-8 h-8 text-surface-300'" />
    <p :class="size === 'sm' ? 'text-[11px] text-surface-400 mt-1' : 'text-xs text-surface-400 mt-2'">{{ message || '暂无内容' }}</p>
  </div>
</template>
