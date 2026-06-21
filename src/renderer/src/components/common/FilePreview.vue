<script setup lang="ts">
import { ref, watch, onUnmounted, nextTick, shallowRef } from 'vue'
import { readFileBytes } from '@/api/db-client'
import VueOfficeExcel from '@vue-office/excel'

const props = defineProps<{ fileId: string; parsedText?: string; fileExt?: string }>()

const loading = ref(false)
const error = ref('')
const previewHtml = ref('')
const blobUrl = ref('')
const excelSrc = shallowRef<ArrayBuffer | null>(null)

const TEXT_EXTS = new Set(['txt', 'md', 'json', 'xml', 'yaml', 'yml', 'log', 'csv', 'ts', 'js', 'tsx', 'jsx', 'vue', 'rs', 'py', 'go', 'java', 'css', 'html', 'sql', 'sh', 'bat', 'ini', 'toml', 'cfg', 'env'])
const IMG_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico'])
const DOCX_EXTS = new Set(['docx'])
const XLSX_EXTS = new Set(['xlsx', 'xls'])

function cleanup() {
  if (blobUrl.value) { URL.revokeObjectURL(blobUrl.value); blobUrl.value = '' }
  excelSrc.value = null
}
function ext() { return (props.fileExt || '').toLowerCase() }

watch(() => props.fileId, () => { cleanup(); loadPreview() }, { immediate: true })
onUnmounted(cleanup)

async function loadPreview() {
  if (!props.fileId) return
  loading.value = true; error.value = ''; previewHtml.value = ''

  const e = ext()

  // Images / PDF: need raw bytes → blob
  if (e === 'pdf' || IMG_EXTS.has(e)) {
    try {
      const r = await readFileBytes(props.fileId)
      if (r?.data) {
        const byteChars = atob(r.data)
        const byteNums = new Array(byteChars.length)
        for (let i = 0; i < byteChars.length; i++) byteNums[i] = byteChars.charCodeAt(i)
        const blob = new Blob([new Uint8Array(byteNums)], { type: e === 'pdf' ? 'application/pdf' : `image/${e === 'jpg' ? 'jpeg' : e}` })
        blobUrl.value = URL.createObjectURL(blob)
      }
    } catch (e: any) { error.value = e.message || '加载失败' }
    loading.value = false; return
  }

  // Excel: use @vue-office/excel (canvas-based, preserves formatting)
  if (XLSX_EXTS.has(e)) {
    try {
      const r = await readFileBytes(props.fileId)
      if (r?.data) {
        const bin = atob(r.data)
        const bytes = new Uint8Array(bin.length)
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
        excelSrc.value = bytes.buffer
      }
    } catch (e: any) { error.value = e.message || '加载失败' }
    loading.value = false; return
  }

  // Text-based files
  if (TEXT_EXTS.has(e) || !e) {
    try {
      const r = await readFileBytes(props.fileId)
      if (r?.data) {
        const text = atob(r.data)
        if (e === 'md') {
          const { Marked } = await import('marked')
          const marked = new Marked({ gfm: true, breaks: true })
          previewHtml.value = marked.parse(text) || ''
        } else {
          previewHtml.value = `<pre style="white-space:pre-wrap;font-family:ui-monospace,monospace;font-size:13px;line-height:1.6;margin:0">${escapeHtml(text)}</pre>`
        }
      }
    } catch (e: any) { error.value = e.message || '加载失败' }
    loading.value = false; return
  }

  // Word: use mammoth
  if (DOCX_EXTS.has(e)) {
    try {
      const r = await readFileBytes(props.fileId)
      if (r?.data) {
        const byteChars = atob(r.data)
        const byteNums = new Array(byteChars.length)
        for (let i = 0; i < byteChars.length; i++) byteNums[i] = byteChars.charCodeAt(i)
        const m = await import('mammoth')
        const mammoth = (m as any).default || m
        const result = await mammoth.convertToHtml({ arrayBuffer: new Uint8Array(byteNums).buffer })
        previewHtml.value = result.value
      }
    } catch (e: any) { error.value = e.message || '加载失败' }
    loading.value = false; return
  }

  loading.value = false
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}
</script>

<template>
  <div class="flex-1 overflow-auto">
    <!-- Loading -->
    <div v-if="loading" class="flex items-center justify-center h-32">
      <p class="text-sm text-surface-400">加载中...</p>
    </div>

    <!-- Error -->
    <div v-else-if="error" class="p-4 text-sm text-red-500">{{ error }}</div>

    <!-- PDF -->
    <iframe v-else-if="ext() === 'pdf' && blobUrl" :src="blobUrl" class="w-full h-full border-0" />

    <!-- Image -->
    <div v-else-if="blobUrl" class="flex items-center justify-center p-4">
      <img :src="blobUrl" class="max-w-full max-h-full object-contain rounded-lg shadow-sm" />
    </div>

    <!-- Excel (@vue-office/excel — canvas rendering, preserves formatting) -->
    <div v-else-if="excelSrc" class="w-full" style="min-height:400px">
      <VueOfficeExcel
        :src="excelSrc"
        style="min-height:400px"
        @rendered="() => {}"
        @error="(e: any) => error = e.message || 'Excel 渲染失败'"
      />
    </div>

    <!-- HTML preview (Word / Markdown / Text) -->
    <div v-else-if="previewHtml" class="p-4 tiptap-preview" v-html="previewHtml" />

    <!-- Fallback: parsed text -->
    <div v-else-if="props.parsedText" class="p-4">
      <p class="text-xs text-surface-600 leading-relaxed whitespace-pre-wrap">{{ props.parsedText }}</p>
    </div>

    <!-- No content -->
    <div v-else class="flex items-center justify-center h-32">
      <p class="text-sm text-surface-400">暂无可预览内容</p>
    </div>
  </div>
</template>

<style>
.tiptap-preview h3 { font-size:14px; font-weight:600; color:#334155; margin:12px 0 8px; }
.tiptap-preview table { border-collapse:collapse; width:100%; font-size:12px; }
.tiptap-preview td { border:1px solid #e2e8f0; padding:4px 8px; white-space:nowrap; }
.tiptap-preview tr:nth-child(even) td { background:#f8fafc; }
</style>
