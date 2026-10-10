<script setup lang="ts">
import type { Annotation } from '@/utils/uiBlocks'

defineProps<{ items: Annotation[] }>()
const emit = defineEmits<{ remove: [index: number]; submit: []; cancel: [] }>()
</script>

<template>
  <div class="annotate-skip mt-1.5 rounded-xl border border-brand-200 bg-brand-50/40 overflow-hidden">
    <div class="px-3 py-2 flex items-center justify-between border-b border-brand-100">
      <span class="text-[10px] font-semibold text-brand-800">批注修订中 —— 选中正文文字可加批注，表格可直接改</span>
    </div>
    <div v-if="items.length" class="px-3 py-2 space-y-1.5">
      <div v-for="(a, i) in items" :key="i"
        class="flex items-center gap-2 rounded-lg bg-white border border-brand-100 px-2.5 py-1.5 text-[10px] shadow-sm">
        <span class="w-4 h-4 rounded-full bg-brand-50 border border-brand-200 text-brand-700 font-semibold flex items-center justify-center flex-shrink-0">{{ i + 1 }}</span>
        <div class="flex-1 min-w-0 leading-snug">
          <span class="text-surface-400">「{{ a.text.length > 40 ? a.text.slice(0, 40) + '…' : a.text }}」</span>
          <span class="text-surface-700"> → {{ a.note }}</span>
        </div>
        <button type="button" class="text-surface-300 hover:text-red-500 leading-none flex-shrink-0" @click="emit('remove', i)">×</button>
      </div>
    </div>
    <div class="px-3 pb-2.5 pt-1 flex items-center justify-between">
      <span class="text-[10px] text-surface-400">{{ items.length ? `${items.length} 条批注` : '表格改动或将正文批注加入后一并提交' }}</span>
      <div class="flex items-center gap-2">
        <button type="button" @click="emit('cancel')"
          class="px-3 py-1 rounded-lg text-[10px] text-surface-500 hover:text-surface-700 hover:bg-white/70">取消</button>
        <button type="button" @click="emit('submit')"
          class="px-3 py-1 rounded-lg text-[10px] font-medium bg-brand-600 text-white hover:bg-brand-700 shadow-sm">提交修订</button>
      </div>
    </div>
  </div>
</template>
