<script setup lang="ts">
import { computed } from 'vue'
import { toast } from '@/composables/useToast'
import { previewFile, exportFile, copyFileToClipboard } from '@/api/db-client'
import type { GeneratedDocument } from '@/composables/useAgentChat'

const props = defineProps<{ doc: GeneratedDocument }>()

const isExcel = computed(() => (props.doc.file_type || '').toLowerCase() === 'xlsx')
const sizeText = computed(() => {
  const s = props.doc.file_size || 0
  if (s >= 1048576) return (s / 1048576).toFixed(1) + ' MB'
  if (s >= 1024) return (s / 1024).toFixed(1) + ' KB'
  return s + ' B'
})

async function onPreview() {
  try {
    const r = await previewFile(props.doc.file_id)
    if (!r?.success) toast({ message: r?.error || '预览失败', type: 'error' })
  } catch (e: any) {
    toast({ message: e?.message || '预览失败', type: 'error' })
  }
}
async function onDownload() {
  try {
    const r = await exportFile(props.doc.file_id)
    if (r?.success) toast({ message: '已保存', type: 'success' })
    else if (r?.error !== '已取消') toast({ message: r?.error || '下载失败', type: 'error' })
  } catch (e: any) {
    toast({ message: e?.message || '下载失败', type: 'error' })
  }
}
async function onCopy() {
  try {
    const r = await copyFileToClipboard(props.doc.file_id)
    if (r?.success) toast({ message: `已复制「${r.fileName || props.doc.file_name}」，可粘贴到文件夹或聊天软件`, type: 'success' })
    else toast({ message: r?.error || '复制失败', type: 'error' })
  } catch (e: any) {
    toast({ message: e?.message || '复制失败', type: 'error' })
  }
}
</script>

<template>
  <div
    class="flex items-center gap-2.5 px-3 py-2 rounded-xl border border-[#e5e5ea] bg-[#f8f9fa] hover:bg-white transition-colors w-full max-w-[360px]"
  >
    <div
      class="w-9 h-9 rounded-lg flex items-center justify-center text-white text-[11px] font-bold flex-shrink-0"
      :class="isExcel ? 'bg-[#107c41]' : 'bg-[#2b579a]'"
    >
      {{ isExcel ? 'XLS' : 'DOC' }}
    </div>
    <div class="min-w-0 flex-1">
      <div class="text-[12px] font-medium text-[#1c1c1e] truncate" :title="doc.file_name">
        {{ doc.file_name }}
      </div>
      <div class="text-[10px] text-[#8e8e93]">{{ sizeText }} · {{ isExcel ? 'Excel' : 'Word' }}</div>
    </div>
    <div class="flex items-center gap-0.5 flex-shrink-0">
      <button
        class="px-2 py-1 rounded-md text-[11px] text-[#2563eb] hover:bg-[#2563eb]/10 transition-colors"
        @click="onPreview"
      >
        预览
      </button>
      <button
        class="px-2 py-1 rounded-md text-[11px] text-[#2563eb] hover:bg-[#2563eb]/10 transition-colors"
        @click="onDownload"
      >
        下载
      </button>
      <button
        class="px-2 py-1 rounded-md text-[11px] text-[#2563eb] hover:bg-[#2563eb]/10 transition-colors"
        @click="onCopy"
      >
        复制
      </button>
    </div>
  </div>
</template>
