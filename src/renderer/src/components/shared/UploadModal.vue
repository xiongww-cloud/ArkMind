<script setup lang="ts">
/**
 * UploadModal — 通用上传弹窗：选择文件 或 Ctrl+V 粘贴
 *
 * 支持：多文件选择、剪贴板粘贴（文件/截图）、待上传列表内编辑文件名、
 * 逐个上传入库、成功后删除粘贴产生的临时文件。
 *
 * Props:
 *   - entityType: 业务域（library / project / todo / note / product）
 *   - resolveEntityId: 异步函数，返回上传目标 entityId（如项目需先同步后端）
 *   - folderId: 可选，上传到指定目录
 *   - customName / creator / note: 透传给 uploadFile 的附加字段
 *   - title: 弹窗标题
 * Emits:
 *   - uploaded: 每个文件成功上传后触发 (fileId)
 *   - done: 全部完成后触发 ({ ok, fail })
 */
import { ref } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { uploadFile, removeTempFile } from '@/api/db-client'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { toast } from '@/composables/useToast'
import { isMac, pasteShortcut } from '@/utils/platform'

const props = defineProps<{
  entityType: string
  resolveEntityId?: () => Promise<string | undefined>
  folderId?: string | null
  customName?: string
  creator?: string
  note?: string
  title?: string
}>()

const emit = defineEmits<{
  uploaded: [fileId: string]
  done: [{ ok: number; fail: number }]
}>()

interface PendingUpload {
  file: File | string
  name: string
  ext: string
  isPath: boolean
  tempPath?: string
}

const show = ref(false)
const uploading = ref(false)
const pendingUploads = ref<PendingUpload[]>([])

function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

const modalRef = ref<HTMLElement | null>(null)

function open() {
  pendingUploads.value = []
  show.value = true
  // macOS：paste 事件需焦点在可聚焦元素上，弹窗打开后聚焦容器
  requestAnimationFrame(() => {
    modalRef.value?.focus()
  })
}

function close() {
  show.value = false
  pendingUploads.value = []
}

async function pickFiles() {
  const result = await openDialog({ multiple: true, canCreateDirectories: false })
  if (!result) return
  const paths: string[] = Array.isArray(result) ? result : [result as string]
  for (const p of paths) {
    const name = p.replace(/^.*[\\\/]/, '')
    const ext = extOf(name)
    if (pendingUploads.value.some(u => u.isPath && (u.file as string) === p)) continue
    pendingUploads.value.push({ file: p, name: name.replace(/\.[^.]+$/, ''), ext, isPath: true })
  }
}

async function onUploadPaste(e: ClipboardEvent) {
  let added = 0
  const files = e.clipboardData?.files
  if (files && files.length > 0) {
    e.preventDefault()
    for (const file of Array.from(files)) {
      const ext = extOf(file.name) || file.type.split('/')[1] || 'bin'
      try {
        const buf = await file.arrayBuffer()
        const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: file.name || null })
        if (r?.success && r.path) {
          const base = (r.name || file.name || 'pasted').replace(/\.[^.]+$/, '')
          pendingUploads.value.push({ file, name: base, ext, isPath: false, tempPath: r.path })
          added++
        }
      } catch (err) { console.warn('[UploadModal] paste save failed:', err) }
    }
  }
  // 回退：从 clipboardData.items 获取图片（截图等无文件名）
  if (added === 0) {
    const items = e.clipboardData?.items
    if (items) {
      for (const item of items) {
        if (!item.type.startsWith('image/')) continue
        e.preventDefault()
        const file = item.getAsFile()
        if (!file) continue
        try {
          const buf = await file.arrayBuffer()
          const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : (item.type.split('/')[1] || 'png')
          const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: null })
          if (r?.success && r.path) {
            pendingUploads.value.push({ file, name: r.name || 'pasted', ext, isPath: false, tempPath: r.path })
            added++
          }
        } catch (err) { console.warn('[UploadModal] paste image failed:', err) }
      }
    }
  }
  // macOS：WebKit 不把剪贴板文件暴露给 clipboardData，从后端 NSPasteboard 读取
  if (isMac && added === 0) {
    await readMacClipboardFiles()
  }
}

/** macOS：从后端 NSPasteboard 读取 Finder 复制的文件，加入待上传列表 */
async function readMacClipboardFiles() {
  try {
    const r: any = await invoke('chat_read_macos_clipboard_files')
    if (r?.success && Array.isArray(r.files) && r.files.length) {
      for (const f of r.files) {
        const name = f.name || 'pasted'
        const ext = extOf(name) || f.kind || 'bin'
        // 去重：keydown 与 paste 可能双触发
        if (pendingUploads.value.some(u => u.tempPath === f.path)) continue
        pendingUploads.value.push({ file: f, name: name.replace(/\.[^.]+$/, ''), ext, isPath: false, tempPath: f.path })
      }
    }
  } catch (err) { console.warn('[UploadModal] macos clipboard read failed:', err) }
}

/** 容器 keydown 兜底：⌘V/Ctrl+V 直接读取剪贴板（macOS 上 paste 事件不可靠） */
function onModalKeydown(e: KeyboardEvent) {
  const meta = isMac ? e.metaKey : e.ctrlKey
  if (!meta || (e.key !== 'v' && e.key !== 'V')) return
  // 焦点在文件名输入框内时交系统粘贴事件处理
  const el = document.activeElement
  if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) return
  e.preventDefault()
  readMacClipboardFiles()
}

function removeUpload(idx: number) {
  const item = pendingUploads.value[idx]
  if (item && !item.isPath && item.tempPath) {
    try { removeTempFile(item.tempPath).catch(() => {}) } catch {}
  }
  pendingUploads.value.splice(idx, 1)
}

function clearAll() {
  for (const item of pendingUploads.value) {
    if (!item.isPath && item.tempPath) {
      try { removeTempFile(item.tempPath).catch(() => {}) } catch {}
    }
  }
  pendingUploads.value = []
}

async function confirmUpload() {
  if (!pendingUploads.value.length || uploading.value) return
  uploading.value = true
  let ok = 0
  let fail = 0
  let entityId: string | undefined
  try {
    entityId = props.resolveEntityId ? await props.resolveEntityId() : undefined
  } catch {
    entityId = undefined
  }
  for (const item of pendingUploads.value) {
    const fullName = item.name.trim() ? `${item.name.trim()}.${item.ext}` : undefined
    const rawName = item.isPath ? (item.file as string).replace(/^.*[\\\/]/, '') : (item.name || 'pasted')
    try {
      const filePath = item.isPath ? (item.file as string) : (item.tempPath as string)
      const r: any = await uploadFile({
        filePath,
        fileName: fullName || rawName,
        fileExt: item.ext,
        entityType: props.entityType,
        entityId,
        customName: props.customName ?? fullName ?? rawName,
        creator: props.creator || '',
        note: props.note || '',
        folderId: props.folderId && props.folderId !== '__uncategorized__' ? props.folderId : null,
      })
      ok++
      const fileId = r?.fileId || r?.id || r?.objectId
      if (fileId) emit('uploaded', fileId)
    } catch (e) {
      fail++
      console.warn('[UploadModal] upload failed:', e)
    } finally {
      if (!item.isPath && item.tempPath) {
        try { await removeTempFile(item.tempPath) } catch {}
      }
    }
  }
  uploading.value = false
  const doneResult = { ok, fail }
  pendingUploads.value = []
  show.value = false
  if (ok > 0 && fail === 0) toast({ message: `上传成功 ${ok} 个文件`, type: 'success' })
  else if (ok > 0) toast({ message: `上传完成：成功 ${ok}，失败 ${fail}`, type: 'warning' })
  else toast({ message: '上传失败', type: 'error' })
  emit('done', doneResult)
}

defineExpose({ open, close })
</script>

<template>
  <Teleport to="body">
    <Transition name="modal-fade">
      <div
        v-if="show"
        ref="modalRef"
        tabindex="-1"
        class="fixed inset-0 z-[80] bg-black/30 flex items-center justify-center outline-none"
        @paste.prevent="onUploadPaste"
        @keydown="onModalKeydown"
      >
        <div class="bg-white rounded-2xl shadow-2xl w-[480px] max-w-[90vw] max-h-[85vh] flex flex-col overflow-hidden" @click.stop>
          <!-- 头部 -->
          <div class="px-6 pt-5 pb-0">
            <h3 class="text-[14px] font-semibold text-surface-800">{{ title || '上传文件' }}</h3>
          </div>

          <!-- 内容区 -->
          <div class="flex-1 overflow-y-auto px-6 py-4">
            <!-- 选择/粘贴区域 -->
            <div
              @click="pickFiles"
              class="w-full border-2 border-dashed rounded-xl py-5 px-4 text-center transition-all duration-200 cursor-pointer flex items-center justify-center gap-2.5 mb-4"
              :class="pendingUploads.length
                ? 'border-emerald-300 bg-emerald-50/40'
                : 'border-surface-200 hover:border-brand-400 hover:bg-brand-50/30'"
            >
              <div
                class="w-8 h-8 rounded-full flex items-center justify-center shrink-0 transition-colors"
                :class="pendingUploads.length ? 'bg-emerald-100 text-emerald-600' : 'bg-surface-100 text-surface-400'"
              >
                <AppIcon name="upload" size="14" />
              </div>
              <div class="text-left">
                <p class="text-[12px] font-medium text-surface-700">
                  {{ pendingUploads.length ? `已选择 ${pendingUploads.length} 个文件` : `点击选择或按 ${pasteShortcut} 粘贴` }}
                </p>
                <p v-if="pendingUploads.length" class="text-[12px] text-surface-600 mt-0.5">点击或粘贴追加文件</p>
              </div>
            </div>

            <!-- 待上传文件列表 -->
            <div v-if="pendingUploads.length" class="space-y-1.5">
              <div class="flex items-center justify-between mb-1.5">
                <span class="text-[10px] font-medium text-surface-400">已选择 {{ pendingUploads.length }} 个文件</span>
                <button @click="clearAll" class="text-[10px] text-surface-400 hover:text-red-500 transition-colors">清空</button>
              </div>
              <div
                v-for="(item, idx) in pendingUploads"
                :key="idx"
                class="group flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg border border-surface-200 hover:border-surface-300 hover:bg-surface-50/50 transition-all duration-150"
              >
                <div class="w-7 h-7 rounded-md bg-surface-50 border border-surface-100 flex items-center justify-center flex-shrink-0">
                  <AppIcon name="file-doc" size="13" class="text-surface-500" />
                </div>
                <div class="flex-1 min-w-0 flex items-center gap-1">
                  <input
                    v-model="item.name"
                    @focus="$event.target.select()"
                    class="flex-1 min-w-0 text-[11px] text-surface-800 bg-transparent border border-transparent focus:border-brand-400 focus:bg-white rounded px-1 py-0.5 outline-none transition-colors"
                    :title="`编辑文件名（当前：${item.name}.${item.ext}）`"
                  />
                  <span class="text-[10px] text-surface-400 flex-shrink-0">.{{ item.ext }}</span>
                </div>
                <button
                  @click="removeUpload(idx)"
                  class="w-5 h-5 flex items-center justify-center rounded text-surface-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 opacity-0 group-hover:opacity-100"
                  title="移除"
                >
                  <AppIcon name="x" size="11" />
                </button>
              </div>
            </div>
            <p v-else class="text-center py-5 text-[11px] text-surface-300">暂无待上传文件</p>
          </div>

          <!-- 底部操作 -->
          <div class="flex items-center justify-end gap-2 px-6 py-4 border-t border-surface-100">
            <button @click="close" class="btn-secondary text-xs">取消</button>
            <button
              @click="confirmUpload"
              :disabled="uploading || !pendingUploads.length"
              class="btn-primary text-xs disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {{ uploading ? '上传中...' : `上传${pendingUploads.length ? `（${pendingUploads.length}）` : ''}` }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.2s;
}
.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}
</style>
