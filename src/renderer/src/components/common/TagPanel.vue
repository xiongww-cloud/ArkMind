<script setup lang="ts">
import { ref } from 'vue'

const props = defineProps<{
  entityType: string
  entityId: string
}>()

const tags = ref<string[]>([])
</script>

<template>
  <div>
    <div class="flex items-center justify-between mb-2">
      <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">标签</h3>
    </div>
    <div v-if="tags.length === 0" class="text-xs text-slate-400 italic">暂无标签</div>
    <div v-else class="flex flex-wrap gap-1">
      <span v-for="tag in tags" :key="tag" class="px-2 py-0.5 bg-slate-100 text-slate-600 rounded text-[10px]">{{ tag }}</span>
    </div>
  </div>
</template>
