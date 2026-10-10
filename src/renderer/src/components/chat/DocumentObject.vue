<script setup lang="ts">
import { computed, ref, onMounted } from 'vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import { previewFile, exportFile, copyFileToClipboard, kbFileExists, kbReleaseGeneratedFile } from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import type { GeneratedDocument } from '@/composables/useAgentChat'

const props = defineProps<{ doc: GeneratedDocument }>()
const missing = ref(false)
onMounted(async () => {
  try { missing.value = !(await kbFileExists(props.doc.file_id))?.exists } catch { /* 查询失败按存在渲染 */ }
})
async function onDelete() {
  const ok = await dangerConfirm({ title: '删除文件', message: `「${props.doc.file_name}」将从资料库、知识库与消息卡片中移除`, confirmText: '删除' })
  if (!ok) return
  try {
    await kbReleaseGeneratedFile(props.doc.file_id)
    missing.value = true
    toast({ message: '已删除', type: 'success' })
    window.dispatchEvent(new CustomEvent('ark:files-changed'))
  } catch (e: any) {
    toast({ message: e?.message || '删除失败', type: 'error' })
  }
}

const fileExt = computed(() => {
  const t = (props.doc.file_type || '').toLowerCase()
  if (t) return t
  const name = props.doc.file_name || ''
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
})
const isExcel = computed(() => fileExt.value.startsWith('xls') || fileExt.value === 'csv')
const extBadge = computed(() => {
  const known = ['doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'pdf', 'csv']
  return known.includes(fileExt.value) ? fileExt.value.toUpperCase() : (isExcel.value ? 'XLSX' : 'DOCX')
})
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
    class="group flex items-center gap-2.5 px-3 py-2 rounded-xl border border-[#e5e5ea] bg-[#f8f9fa] hover:bg-white transition-colors w-full max-w-[360px]"
    :class="missing ? 'opacity-60' : ''"
  >
    <div
      class="w-9 h-9 rounded-lg flex items-center justify-center text-white text-[11px] font-bold flex-shrink-0"
      :class="missing ? 'bg-surface-300' : (isExcel ? 'bg-[#107c41]' : 'bg-[#2b579a]')"
    >
      {{ missing ? '—' : extBadge }}
    </div>
    <div class="min-w-0 flex-1">
      <!-- 点击文件名预览（与资料库文件行一致）；文件已删除→灰壳留档"生成过"的事实 -->
      <div
        class="text-[12px] font-medium text-[#1c1c1e] truncate" :class="missing ? 'line-through text-surface-400' : 'hover:text-blue-600 transition-colors cursor-pointer'"
        v-tooltip.auto="doc.file_name"
        @click="!missing && onPreview()"
      >
        {{ doc.file_name }}
      </div>
      <div class="text-[10px]" :class="missing ? 'text-red-400' : 'text-[#8e8e93]'">
        <template v-if="missing">文件已删除</template>
        <template v-else>{{ sizeText }} · {{ isExcel ? 'Excel' : 'Word' }}</template>
      </div>
    </div>
    <!-- 下载/复制/删除纯 icon，hover 浮现（与资料库行内按钮一致） -->
    <div v-if="!missing" class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
      <button
        class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
        v-tooltip="'下载'"
        @click="onDownload"
      >
        <AppIcon name="download" size="13" />
      </button>
      <button
        class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
        v-tooltip="'复制文件到剪贴板'"
        @click="onCopy"
      >
        <AppIcon name="copy" size="13" />
      </button>
      <button
        class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 transition-colors"
        v-tooltip="'删除文件'"
        @click="onDelete"
      >
        <AppIcon name="trash" size="13" />
      </button>
    </div>
  </div>
</template>
