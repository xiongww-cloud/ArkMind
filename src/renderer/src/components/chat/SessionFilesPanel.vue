<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import type { SessionFile } from '@/api/db-client'

/** 会话交付文件抽屉：对齐笔记附件抽屉（AttachmentPanel）的浮动折叠形态 */
const props = defineProps<{ files: SessionFile[]; open: boolean }>()
const emit = defineEmits<{ toggle: []; preview: [f: SessionFile]; download: [f: SessionFile]; copy: [f: SessionFile]; remove: [f: SessionFile] }>()

function fmtSize(bytes: number | null | undefined): string {
  if (!bytes) return ''
  if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(1) + ' GB'
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB'
  if (bytes >= 1024) return (bytes / 1024).toFixed(0) + ' KB'
  return bytes + ' B'
}
function typeClass(f: SessionFile): string {
  const e = (f.ext || '').toLowerCase()
  const map: Record<string, string> = {
    docx: 'bg-blue-50 text-blue-600', doc: 'bg-blue-50 text-blue-600',
    pdf: 'bg-red-50 text-red-600',
    xlsx: 'bg-emerald-50 text-emerald-600', xls: 'bg-emerald-50 text-emerald-600', csv: 'bg-emerald-50 text-emerald-600',
    pptx: 'bg-orange-50 text-orange-600', ppt: 'bg-orange-50 text-orange-600',
  }
  return map[e] || 'bg-slate-50 text-slate-500'
}
function typeBadge(f: SessionFile): string {
  return (f.ext || 'FILE').toUpperCase().slice(0, 4)
}

function onDocDown(e: MouseEvent) {
  if (!props.open) return
  const t = e.target as HTMLElement
  if (t.closest('[data-sf-panel]') || t.closest('[data-sf-toggle]')) return
  emit('toggle')
}
onMounted(() => document.addEventListener('mousedown', onDocDown))
onUnmounted(() => document.removeEventListener('mousedown', onDocDown))
</script>

<template>
  <div class="absolute right-0 top-0 bottom-0 select-none pointer-events-none z-30">
    <div data-sf-panel
      class="relative flex flex-col h-full bg-white shadow-lg transition-all duration-300 ease-in-out pointer-events-auto"
      :class="open ? 'w-80 border-l border-gray-100' : 'w-0 border-l-0'">
      <button data-sf-toggle @click="emit('toggle')"
        class="absolute -left-4 top-1/2 -translate-y-1/2 z-50 w-4 h-14 flex items-center justify-center bg-white border border-y-gray-200/80 border-l-gray-200/60 rounded-l-md hover:bg-gray-50 transition-colors"
        v-tooltip="open ? '收起交付文件' : '展开交付文件'">
        <svg class="h-3 w-3 transition-transform duration-300" :class="{ 'rotate-180': !open }" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7" />
        </svg>
      </button>
      <div v-if="open" class="flex flex-col h-full overflow-hidden min-w-0">
        <div class="flex items-center gap-1.5 px-3 h-9 border-b border-gray-100 shrink-0">
          <span class="text-[11px] font-semibold text-gray-700">交付文件</span>
          <span class="text-[9px] px-1.5 py-0.5 rounded-full bg-gray-100 text-gray-500">{{ files.length }}</span>
        </div>
        <div class="flex-1 overflow-y-auto p-3 space-y-2">
          <div v-if="!files.length" class="h-64 flex flex-col items-center justify-center text-center px-6">
            <div class="w-14 h-14 rounded-2xl bg-gray-50 flex items-center justify-center text-gray-300 mb-4">
              <svg class="h-7 w-7" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.2"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" /></svg>
            </div>
            <p class="text-[13px] font-medium text-gray-400">暂无交付文件</p>
            <p class="text-[11px] text-gray-300 mt-1">AI 生成的 Word / Excel 会汇总在这里</p>
          </div>
          <div v-for="f in files" :key="f.id"
            class="group relative flex items-center p-2 rounded-xl border border-gray-100 bg-white hover:border-gray-200 hover:shadow-sm transition-all">
            <div class="w-9 h-9 flex items-center justify-center rounded-lg font-bold text-[8px] tracking-wider shrink-0 select-none" :class="typeClass(f)">{{ typeBadge(f) }}</div>
            <div class="ml-2.5 flex-1 min-w-0 pr-2">
              <p class="text-[11px] font-medium text-gray-700 truncate leading-tight cursor-pointer hover:text-blue-600 transition-colors"
                v-tooltip.auto="f.name" @click="emit('preview', f)">{{ f.name }}</p>
              <p class="text-[9px] text-gray-400 mt-0.5 truncate">
                {{ (f.created_at || '').slice(5, 16) }}{{ f.size ? ' · ' + fmtSize(f.size) : '' }}
                <span v-if="!f.live_src" class="ml-1 px-1 py-px rounded bg-amber-50 text-amber-600">来源已撤回</span>
              </p>
            </div>
            <div class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
              <button class="w-6 h-6 rounded text-surface-400 hover:text-brand-600 flex items-center justify-center" v-tooltip="'下载'" @click="emit('download', f)"><AppIcon name="download" size="12" /></button>
              <button class="w-6 h-6 rounded text-surface-400 hover:text-brand-600 flex items-center justify-center" v-tooltip="'复制文件'" @click="emit('copy', f)"><AppIcon name="copy" size="12" /></button>
              <button class="w-6 h-6 rounded text-surface-400 hover:text-red-500 flex items-center justify-center" v-tooltip="'删除'" @click="emit('remove', f)"><AppIcon name="trash" size="12" /></button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
