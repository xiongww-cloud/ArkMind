<script setup lang="ts">
import { computed } from 'vue'
import type { Annotation, TableRevision } from '@/utils/uiBlocks'

const props = defineProps<{ items: Annotation[]; tables: TableRevision[] }>()
const emit = defineEmits<{ remove: [index: number]; submit: []; cancel: []; 'reset-table': [index: number] }>()

// 表格改动与正文批注同列一个清单（顺序对齐【答复修订】消息：先表格后批注，全局连续序号）
const tableCards = computed(() => props.tables.map((tb, i) => ({ tb, i })).filter(x => x.tb.diff.trim()))
const total = computed(() => tableCards.value.length + props.items.length)
</script>

<template>
  <div class="annotate-skip mt-1.5 rounded-xl border border-brand-200 bg-brand-50/40 overflow-hidden">
    <div class="px-3 py-2 flex items-center justify-between border-b border-brand-100">
      <span class="text-[10px] font-semibold text-brand-800">修订中 —— 选中正文文字提出修改要求，表格可直接改</span>
    </div>
    <div v-if="total" class="px-3 py-2 space-y-1.5">
      <div v-for="(x, j) in tableCards" :key="'t' + x.i"
        class="flex items-center gap-2 rounded-lg bg-white border border-brand-100 px-2.5 py-1.5 text-[10px] shadow-sm">
        <span class="w-4 h-4 rounded-full bg-brand-50 border border-brand-200 text-brand-700 font-semibold flex items-center justify-center flex-shrink-0">{{ j + 1 }}</span>
        <div class="flex-1 min-w-0 leading-snug">
          <span class="text-surface-400">表格修改「{{ x.tb.headerLabel }}」</span>
          <div class="mt-0.5 whitespace-pre-line text-surface-700">{{ x.tb.diff }}</div>
        </div>
        <button type="button" class="text-surface-300 hover:text-red-500 leading-none flex-shrink-0" v-tooltip="'撤销该表格的全部改动'" @click="emit('reset-table', x.i)">×</button>
      </div>
      <div v-for="(a, i) in items" :key="'a' + i"
        class="flex items-center gap-2 rounded-lg bg-white border border-brand-100 px-2.5 py-1.5 text-[10px] shadow-sm">
        <span class="w-4 h-4 rounded-full bg-brand-50 border border-brand-200 text-brand-700 font-semibold flex items-center justify-center flex-shrink-0">{{ tableCards.length + i + 1 }}</span>
        <div class="flex-1 min-w-0 leading-snug">
          <span class="text-surface-400">「{{ a.text.length > 40 ? a.text.slice(0, 40) + '…' : a.text }}」</span>
          <span class="text-surface-700"> → {{ a.note }}</span>
        </div>
        <button type="button" class="text-surface-300 hover:text-red-500 leading-none flex-shrink-0" @click="emit('remove', i)">×</button>
      </div>
    </div>
    <div class="px-3 pb-2.5 pt-1 flex items-center justify-between">
      <span class="text-[10px] text-surface-400">{{ total ? `${total} 处修订，提交后 AI 输出修订版` : '选中正文文字提出修改要求，或直接改表格 —— 修订内容会列在这里' }}</span>
      <div class="flex items-center gap-2">
        <button type="button" @click="emit('cancel')"
          class="px-3 py-1 rounded-lg text-[10px] text-surface-500 hover:text-surface-700 hover:bg-white/70">取消</button>
        <button type="button" @click="emit('submit')"
          class="px-3 py-1 rounded-lg text-[10px] font-medium bg-brand-600 text-white hover:bg-brand-700 shadow-sm">提交修订</button>
      </div>
    </div>
  </div>
</template>
