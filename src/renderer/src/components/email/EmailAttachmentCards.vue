<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from 'vue'
import { toast } from '@/composables/useToast'
import { previewFile, exportFile, copyFileToClipboard } from '@/api/db-client'
import type { EmailAttachment } from '@/api/email-client'
import { lazyLoadEmailImages } from '@/utils/emailLazyImages'
import AppIcon from '@/components/common/AppIcon.vue'

const props = defineProps<{ emailId: string; attachments: EmailAttachment[] }>()

// is_inline 内嵌图已由正文 cid 渲染，不重复展示
const inlineAssets = computed(() => new Set(props.attachments.filter((a) => a.is_inline && a.file_asset_id).map((a) => a.file_asset_id as string)))
function looksImage(a: EmailAttachment): boolean {
  return (a.mime_type || '').startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|svg)$/i.test(a.filename || '')
}
const visibleAtts = computed(() => props.attachments.filter((a) => !a.is_inline && !(looksImage(a) && a.file_asset_id && inlineAssets.value.has(a.file_asset_id))))
// Outlook 双份 part：文件名版与内嵌版同资产（CAS 内容去重同 file_asset_id）→ 副本一并隐藏（迁移前的存量兜底）
// 图片附件（有资产）走正文同款 emimg 懒加载渲染；其余为文件卡片（图片资产缺失的降级进卡片）
const imageAtts = computed(() => visibleAtts.value.filter((a) => looksImage(a) && !!a.file_asset_id))
const fileAtts = computed(() => visibleAtts.value.filter((a) => !(looksImage(a) && !!a.file_asset_id)))

const EXT_BADGE: Record<string, { label: string; cls: string }> = {
  doc: { label: 'DOC', cls: 'bg-[#2b579a]' },
  docx: { label: 'DOCX', cls: 'bg-[#2b579a]' },
  xls: { label: 'XLS', cls: 'bg-[#107c41]' },
  xlsx: { label: 'XLSX', cls: 'bg-[#107c41]' },
  csv: { label: 'CSV', cls: 'bg-[#107c41]' },
  ppt: { label: 'PPT', cls: 'bg-[#d24726]' },
  pptx: { label: 'PPTX', cls: 'bg-[#d24726]' },
  pdf: { label: 'PDF', cls: 'bg-[#b30b00]' },
  zip: { label: 'ZIP', cls: 'bg-[#6b7280]' },
  rar: { label: 'RAR', cls: 'bg-[#6b7280]' },
  '7z': { label: '7Z', cls: 'bg-[#6b7280]' },
  txt: { label: 'TXT', cls: 'bg-[#6b7280]' },
  md: { label: 'MD', cls: 'bg-[#6b7280]' },
  png: { label: 'PNG', cls: 'bg-[#0ea5e9]' },
  jpg: { label: 'JPG', cls: 'bg-[#0ea5e9]' },
  jpeg: { label: 'JPEG', cls: 'bg-[#0ea5e9]' },
  gif: { label: 'GIF', cls: 'bg-[#0ea5e9]' },
  webp: { label: 'WEBP', cls: 'bg-[#0ea5e9]' },
}

function attExt(a: EmailAttachment): string {
  const mime = (a.mime_type || '').toLowerCase()
  if (mime.includes('/pdf')) return 'pdf'
  if (mime.includes('wordprocessingml')) return 'docx'
  if (mime.includes('spreadsheetml') || mime.includes('ms-excel')) return 'xlsx'
  if (mime.includes('presentationml') || mime.includes('ms-powerpoint')) return 'pptx'
  const m = mime.match(/image\/(\w+)/)
  if (m) return m[1] === 'jpeg' ? 'jpg' : m[1]
  if (mime.includes('zip')) return 'zip'
  if (mime.includes('text/plain')) return 'txt'
  const i = (a.filename || '').lastIndexOf('.')
  return i > 0 ? a.filename.slice(i + 1).toLowerCase() : ''
}

function badgeOf(a: EmailAttachment) {
  return EXT_BADGE[attExt(a)] || { label: 'FILE', cls: 'bg-[#6b7280]' }
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1048576) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / 1048576).toFixed(1) + ' MB'
}

async function onPreview(a: EmailAttachment) {
  if (!a.file_asset_id) return
  try {
    await previewFile(a.file_asset_id)
  } catch (e: any) {
    toast({ message: e?.message || '预览失败', type: 'error' })
  }
}
async function onDownload(a: EmailAttachment) {
  if (!a.file_asset_id) return
  try {
    const r = await exportFile(a.file_asset_id)
    if (r?.success) toast({ message: '已保存', type: 'success' })
    else if (r?.error !== '已取消') toast({ message: r?.error || '下载失败', type: 'error' })
  } catch (e: any) {
    toast({ message: e?.message || '下载失败', type: 'error' })
  }
}
async function onCopy(a: EmailAttachment) {
  if (!a.file_asset_id) return
  try {
    const r = await copyFileToClipboard(a.file_asset_id)
    if (r?.success) toast({ message: `已复制「${r.fileName || a.filename}」，可粘贴到文件夹或聊天软件`, type: 'success' })
    else toast({ message: r?.error || '复制失败', type: 'error' })
  } catch (e: any) {
    toast({ message: e?.message || '复制失败', type: 'error' })
  }
}

// 图片附件与正文同款管线：img[data-emimg=资产id] 占位 → lazyLoadEmailImages 视口加载/spinner/淡入
const attImagesEl = ref<HTMLElement | null>(null)
let cancelLazy: (() => void) | null = null
async function restartLazy() {
  cancelLazy?.()
  cancelLazy = null
  if (!imageAtts.value.length || !props.emailId) return
  await nextTick()
  cancelLazy = lazyLoadEmailImages(attImagesEl.value, props.emailId)
}
onMounted(restartLazy)
watch(() => [props.emailId, props.attachments] as const, restartLazy)
onBeforeUnmount(() => {
  cancelLazy?.()
  cancelLazy = null
})
</script>

<template>
  <div v-if="imageAtts.length > 0 || fileAtts.length > 0" class="mt-4">
    <!-- 图片附件：与正文内嵌图完全一致的显示方式（全宽/占位/淡入） -->
    <div v-if="imageAtts.length > 0" ref="attImagesEl" class="email-body space-y-2">
      <img
        v-for="a in imageAtts"
        :key="`${emailId}-${a.id}`"
        :data-emimg="a.file_asset_id"
        :alt="a.filename"
      />
    </div>
    <!-- 文件附件：AI 生成文档同款卡片 -->
    <div v-if="fileAtts.length > 0" class="space-y-2">
      <div
        v-for="a in fileAtts"
        :key="`card-${a.id}`"
        class="group flex items-center gap-2.5 px-3 py-2 rounded-xl border border-[#e5e5ea] bg-[#f8f9fa] hover:bg-white transition-colors w-full max-w-[360px]"
        :class="a.file_asset_id ? '' : 'opacity-60'"
      >
        <div
          class="w-9 h-9 rounded-lg flex items-center justify-center text-white text-[11px] font-bold flex-shrink-0"
          :class="badgeOf(a).cls"
        >
          {{ badgeOf(a).label }}
        </div>
        <div class="min-w-0 flex-1">
          <div
            class="text-[12px] font-medium text-[#1c1c1e] truncate hover:text-blue-600 transition-colors"
            :class="a.file_asset_id ? 'cursor-pointer' : ''"
            v-tooltip.auto="a.filename"
            @click="onPreview(a)"
          >
            {{ a.filename }}
          </div>
          <div class="text-[10px] text-[#8e8e93]">{{ formatSize(a.size) }} · {{ badgeOf(a).label }}</div>
        </div>
        <div v-if="a.file_asset_id" class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
          <button
            class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
            v-tooltip="'下载'"
            @click.stop="onDownload(a)"
          >
            <AppIcon name="download" size="13" />
          </button>
          <button
            class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
            v-tooltip="'复制文件到剪贴板'"
            @click.stop="onCopy(a)"
          >
            <AppIcon name="copy" size="13" />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 与邮件正文页 .email-body 图片规则一致（独立组件容器内复制同款渲染表现） */
.email-body :deep(img) {
  max-width: 100% !important;
  height: auto !important;
}
.email-body :deep(img[data-emimg]) {
  background: transparent;
}
.email-body :deep(.emimg-box) {
  position: relative;
  display: block;
  width: 100%;
  border-radius: 0;
  background: #f1f5f9;
  overflow: hidden;
  margin: 0;
}
.email-body :deep(.emimg-box.emimg-loaded) {
  background: transparent;
}
.email-body :deep(.emimg-box > img) {
  display: block;
  height: auto !important;
  opacity: 0;
  transition: opacity 0.3s ease;
}
.email-body :deep(.emimg-box.emimg-loaded > img) {
  opacity: 1;
}
.email-body :deep(.emimg-spinner) {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 20px;
  height: 20px;
  margin: -10px 0 0 -10px;
  border-radius: 50%;
  border: 2px solid #e2e8f0;
  border-top-color: #3b82f6;
  animation: emimg-spin 0.8s linear infinite;
  opacity: 1;
  transition: opacity 0.25s;
  pointer-events: none;
}
.email-body :deep(.emimg-box.emimg-loaded .emimg-spinner) {
  opacity: 0;
  visibility: hidden;
}
.email-body :deep(.emimg-box.emimg-error .emimg-spinner) {
  opacity: 0.35;
  animation: none;
}
@keyframes emimg-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
