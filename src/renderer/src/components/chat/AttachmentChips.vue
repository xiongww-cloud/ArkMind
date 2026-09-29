<script setup lang="ts">
import AppIcon from '@/components/common/AppIcon.vue'
import { convertFileSrc } from '@tauri-apps/api/core'

interface AttachmentFile {
  id?: string
  path: string
  name: string
  kind: 'image' | 'doc'
  previewUrl?: string
  uploading?: boolean
  progress?: number
}

const props = withDefaults(defineProps<{
  files: AttachmentFile[]
  removable?: boolean
}>(), { removable: false })

const emit = defineEmits<{ remove: [index: number] }>()

const IMG_EXTS = new Set(['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'])
function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

function isImage(f: AttachmentFile): boolean {
  return f.kind === 'image' || IMG_EXTS.has(extOf(f.name))
}

function imgSrc(f: AttachmentFile): string {
  if (f.previewUrl) return f.previewUrl
  try { return convertFileSrc(f.path) } catch { return '' }
}

function iconForFile(name: string): string {
  const ext = extOf(name)
  if (['doc', 'docx'].includes(ext)) return 'file-doc'
  if (['xls', 'xlsx', 'csv'].includes(ext)) return 'file-spreadsheet'
  if (['ppt', 'pptx'].includes(ext)) return 'presentation'
  if (ext === 'pdf') return 'file-pdf'
  if (['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'].includes(ext)) return 'image'
  if (['txt', 'md', 'json'].includes(ext)) return 'file-doc'
  return 'file'
}
</script>

<template>
  <div v-if="files.length" class="flex flex-wrap gap-1.5">
    <div v-for="(f, i) in files" :key="f.id || f.path || i"
      class="relative flex items-center gap-1.5 pl-1 pr-1.5 py-0.5 rounded-md border text-[11px] max-w-[200px] transition-colors overflow-hidden"
      :class="isImage(f)
        ? 'border-purple-200 bg-purple-50 text-purple-700'
        : 'border-blue-200 bg-blue-50 text-blue-700'">
      <img v-if="isImage(f) && imgSrc(f)" :src="imgSrc(f)"
        class="w-5 h-5 rounded object-cover flex-shrink-0"
        :class="f.uploading ? 'opacity-50' : ''" />
      <AppIcon v-else :name="iconForFile(f.name)" size="14" class="flex-shrink-0" :class="f.uploading ? 'opacity-50' : ''" />
      <span class="truncate" v-tooltip.auto="f.name" :class="f.uploading ? 'opacity-70' : ''">{{ f.name }}</span>
      <span v-if="f.uploading" class="text-[9px] tabular-nums flex-shrink-0 opacity-80">{{ f.progress ?? 0 }}%</span>
      <button v-if="removable" @click.stop="emit('remove', i)"
        class="w-3.5 h-3.5 flex items-center justify-center rounded hover:bg-black/10 flex-shrink-0 -mr-0.5"
        :title="f.uploading ? '取消上传' : '移除'">
        <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
      <!-- 上传中：底部进度条 -->
      <div v-if="f.uploading" class="absolute left-0 bottom-0 h-[2px] w-full bg-black/5">
        <div class="h-full bg-brand-500 transition-all duration-200" :style="{ width: (f.progress ?? 0) + '%' }"></div>
      </div>
    </div>
  </div>
</template>
