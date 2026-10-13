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
import { ref, onMounted, onBeforeUnmount } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { uploadFile, removeTempFile, fetchFileTree, createFolder } from '@/api/db-client'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { toast } from '@/composables/useToast'
import { pasteShortcut } from '@/utils/platform'
import { uploadPasteFile } from '@/utils/pasteUpload'

const props = defineProps<{
  entityType: string
  resolveEntityId?: () => Promise<string | undefined>
  folderId?: string | null
  customName?: string
  creator?: string
  note?: string
  title?: string
  /** 域内启用「上传目录」：仅 library/project 等有目录树的域传入 */
  folderTree?: boolean
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
  relPath?: string        // 目录上传携带相对路径（'/' 分隔），提交时据此镜像目录树
  uploading?: boolean
  progress?: number       // 0-100（粘贴分片上传进度）
  _cancelled?: boolean
}

const show = ref(false)
const uploading = ref(false)
const pendingUploads = ref<PendingUpload[]>([])
// 单入口小菜单（仅 folderTree 域）：文件/目录两路对话框由系统限制无法混选，点击分流
const entryMenu = ref(false)
function onZoneClick() {
  if (props.folderTree) entryMenu.value = !entryMenu.value
  else pickFiles()
}

function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

const pasteArea = ref<HTMLTextAreaElement | null>(null)

// ─── 窗口级 Ctrl/Cmd+V 兜底：不依赖焦点，切回应用后仍可粘贴文件 ───
function isEditable(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  if (!t) return false
  const tag = t.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t.isContentEditable
}

let lastClipRead = 0
function onPasteKeydown(e: KeyboardEvent) {
  if (!show.value) return
  const mod = e.ctrlKey || e.metaKey
  if (!mod || (e.key.toLowerCase() !== 'v')) return
  // 焦点在可编辑元素：放行默认行为（文本粘贴由浏览器处理，截图/文件走 textarea @paste）
  if (isEditable(e.target)) return
  // 焦点在 body/不可编辑处：切回应用后粘贴场景，preventDefault 后走后端读文件
  e.preventDefault()
  const now = Date.now()
  if (now - lastClipRead < 500) return // 防抖，避免重复后端调用
  lastClipRead = now
  readClipboardFiles()
}

onMounted(() => window.addEventListener('keydown', onPasteKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', onPasteKeydown))

function open() {
  pendingUploads.value = []
  dirSummary.value = null
  entryMenu.value = false
  show.value = true
  // macOS：paste 事件只在可编辑元素上触发，弹窗打开后聚焦隐藏的 textarea
  requestAnimationFrame(() => {
    pasteArea.value?.focus()
  })
}

function close() {
  show.value = false
  pendingUploads.value = []
  entryMenu.value = false
  dirSummary.value = null
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

// ─── 目录上传（镜像目录树；仅 folderTree 域启用） ───────────────────────────
interface DirSummary {
  files: { path: string; relPath: string; name: string; size: number }[]
  skipped: { hidden: number; big: number; deep: number; link: number }
  truncated: boolean
  rootName: string
}
const dirSummary = ref<DirSummary | null>(null)

async function pickDirectory() {
  const dir = await openDialog({ directory: true, canCreateDirectories: false })
  if (!dir) return
  let r: any
  try { r = await invoke('dir_list_uploadable', { dir: dir as string }) } catch (e: any) {
    toast({ message: `目录读取失败: ${e?.message || e}`, type: 'error' }); return
  }
  if (!r?.success) { toast({ message: r?.error || '目录读取失败', type: 'error' }); return }
  if (!(r.files || []).length) { toast({ message: '目录内没有可上传的文件', type: 'info' }); return }
  const rootName = String(r.root || dir).replace(/[\\/]+$/, '').split(/[\\/]/).pop() || String(dir)
  dirSummary.value = { files: r.files, skipped: r.skipped ?? { hidden: 0, big: 0, deep: 0, link: 0 }, truncated: !!r.truncated, rootName }
}

/** 确认闸通过后才入现有队列（条目带 relPath，提交时镜像目录） */
function confirmDir() {
  if (!dirSummary.value) return
  for (const f of dirSummary.value.files) {
    if (pendingUploads.value.some(u => u.isPath && (u.file as string) === f.path)) continue
    const dot = f.name.lastIndexOf('.')
    pendingUploads.value.push({
      file: f.path,
      name: dot > 0 ? f.name.slice(0, dot) : f.name,
      ext: dot > 0 ? f.name.slice(dot + 1).toLowerCase() : '',
      isPath: true,
      relPath: f.relPath,
    })
  }
  dirSummary.value = null
}

function dirOf(relPath?: string): string {
  if (!relPath) return ''
  const i = relPath.lastIndexOf('/')
  return i > 0 ? relPath.slice(0, i) : ''
}

// 镜像缓存：(parentFolderId|name) → folderId；每批上传重置（库内树可能已被外部变更）
let mirrorIndex: Map<string, string> | null = null
function baseFolderId(): string | null {
  return props.folderId && props.folderId !== '__uncategorized__' ? props.folderId : null
}

/** relPath 的目录段逐级懒创建（createFolder 复用同级同名）；某级创建失败则落上一级继续，不阻断文件上传 */
async function ensureMirrorDirs(relPath: string, projectId: string | null): Promise<string | null> {
  const parts = relPath.split('/').slice(0, -1)
  if (!parts.length) return baseFolderId()
  if (!mirrorIndex) {
    mirrorIndex = new Map()
    try {
      const tree: any = await fetchFileTree(projectId ?? undefined)
      for (const fd of tree?.folders ?? []) mirrorIndex.set(`${fd.parent_id ?? ''}|${fd.name}`, fd.id)
    } catch { /* 索引拉取失败退化为全新建：同名复用失效但可用 */ }
  }
  let parentId = baseFolderId()
  for (const seg of parts) {
    const key = `${parentId ?? ''}|${seg}`
    let id: string | null | undefined = mirrorIndex.get(key)
    if (!id) {
      try { id = ((await createFolder(seg, parentId, projectId)) as any)?.id || null } catch { id = null }
      if (!id) return parentId
      mirrorIndex.set(key, id)
    }
    parentId = id
  }
  return parentId
}

/** 分片上传一个粘贴文件：先占位（进度即时可见），完成后回填 tempPath */
async function uploadPastedFile(file: File, ext: string, nameFallback: string): Promise<boolean> {
  const base = (file.name || nameFallback).replace(/\.[^.]+$/, '')
  const item: PendingUpload = {
    file,
    name: base,
    ext,
    isPath: false,
    uploading: true,
    progress: 0,
  }
  pendingUploads.value.push(item)
  try {
    const r = await uploadPasteFile(file, ext, (pct) => { item.progress = pct }, () => !!item._cancelled)
    if (!pendingUploads.value.includes(item)) {
      if (r?.path) removeTempFile(r.path).catch(() => {})
      return false
    }
    item.tempPath = r.path
    item.uploading = false
    const dn = (r.name || file.name || '').replace(/\.[^.]+$/, '')
    if (dn) item.name = dn
    return true
  } catch (err: any) {
    const idx = pendingUploads.value.indexOf(item)
    if (idx >= 0) pendingUploads.value.splice(idx, 1)
    const msg = String(err?.message || err)
    if (!item._cancelled && msg !== 'Error: 已取消') {
      console.warn('[UploadModal] paste save failed:', err)
      if (msg.includes('上限') || msg.includes('超过')) toast({ message: msg, type: 'error' })
    }
    return false
  }
}

async function onUploadPaste(e: ClipboardEvent) {
  let added = 0
  const files = e.clipboardData?.files
  if (files && files.length > 0) {
    e.preventDefault()
    for (const file of Array.from(files)) {
      const ext = extOf(file.name) || file.type.split('/')[1] || 'bin'
      if (await uploadPastedFile(file, ext, 'pasted')) added++
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
        const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : (item.type.split('/')[1] || 'png')
        if (await uploadPastedFile(file, ext, 'pasted')) added++
      }
    }
  }
  // 系统剪贴板中的文件不暴露给 clipboardData（浏览器安全限制），
  // 若浏览器侧未取到任何文件且剪贴板确为文件列表，才回退从后端读取（纯文本粘贴不打后端）
  const clipboardHasFiles = Array.from(e.clipboardData?.types || []).includes('Files')
  if (added === 0 && clipboardHasFiles) {
    await readClipboardFiles()
  }
  // 处理完成后重新聚焦隐藏 textarea，保持后续粘贴可用
  requestAnimationFrame(() => {
    pasteArea.value?.focus()
  })
}

/** 从后端读取系统剪贴板（资源管理器/Finder 复制）的文件，加入待上传列表 */
async function readClipboardFiles() {
  try {
    const r: any = await invoke('chat_read_clipboard_files')
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

function removeUpload(idx: number) {
  const item = pendingUploads.value[idx]
  if (item) {
    // 上传中：标记中断（分片循环下一片检测到后中止并清理）
    if (item.uploading) item._cancelled = true
    else if (!item.isPath && item.tempPath) {
      try { removeTempFile(item.tempPath).catch(() => {}) } catch {}
    }
  }
  pendingUploads.value.splice(idx, 1)
}

function clearAll() {
  for (const item of pendingUploads.value) {
    if (item.uploading) item._cancelled = true
    else if (!item.isPath && item.tempPath) {
      try { removeTempFile(item.tempPath).catch(() => {}) } catch {}
    }
  }
  pendingUploads.value = []
}

async function confirmUpload() {
  if (!pendingUploads.value.length || uploading.value) return
  if (pendingUploads.value.some(u => u.uploading)) {
    toast({ message: '粘贴的文件仍在上传，请稍候', type: 'error' })
    return
  }
  uploading.value = true
  mirrorIndex = null
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
      // 目录上传条目：按 relPath 镜像落位；普通条目维持原 folderId 语义
      const folderIdForItem = item.relPath && props.folderTree
        ? await ensureMirrorDirs(item.relPath, props.entityType === 'project' ? (entityId ?? null) : null)
        : baseFolderId()
      const r: any = await uploadFile({
        filePath,
        fileName: fullName || rawName,
        fileExt: item.ext,
        entityType: props.entityType,
        entityId,
        customName: props.customName ?? fullName ?? rawName,
        creator: props.creator || '',
        note: props.note || '',
        folderId: folderIdForItem,
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
        class="fixed inset-0 z-[80] bg-black/30 flex items-center justify-center"
      >
        <!-- 隐藏 textarea：macOS 上 paste 事件只在可编辑元素触发，用它捕获粘贴 -->
        <textarea
          ref="pasteArea"
          class="fixed top-0 left-0 w-px h-px opacity-0"
          tabindex="-1"
          @paste.prevent="onUploadPaste"
        ></textarea>
        <div class="bg-white rounded-2xl shadow-2xl w-[480px] max-w-[90vw] max-h-[85vh] flex flex-col overflow-hidden" @click.stop>
          <!-- 头部 -->
          <div class="px-6 pt-5 pb-0">
            <h3 class="text-[14px] font-semibold text-surface-800">{{ title || '上传文件' }}</h3>
          </div>

          <!-- 内容区 -->
          <div class="flex-1 overflow-y-auto px-6 py-4">
            <!-- 选择/粘贴区域：单入口；树域点击出「文件/目录」小菜单 -->
            <div
              @click="onZoneClick"
              class="w-full border-2 border-dashed rounded-xl py-5 px-4 text-center transition-all duration-200 cursor-pointer flex items-center justify-center gap-2.5 mb-1.5"
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
                  {{ pendingUploads.length ? `已选择 ${pendingUploads.length} 个文件` : (folderTree ? `点击选择文件或目录，或按 ${pasteShortcut} 粘贴` : `点击选择或按 ${pasteShortcut} 粘贴`) }}
                </p>
                <p v-if="pendingUploads.length" class="text-[12px] text-surface-600 mt-0.5">点击或粘贴追加文件</p>
              </div>
            </div>

            <!-- 入口小菜单（仅树域；平台文件框不能混选文件与目录，由菜单分流两路对话框） -->
            <div v-if="entryMenu" class="mb-2.5 border border-surface-200 rounded-xl shadow-lg bg-white overflow-hidden">
              <button @click="entryMenu = false; pickFiles()" class="w-full flex items-center gap-2 px-4 py-2.5 text-[12px] text-surface-700 hover:bg-brand-50/50 transition-colors">
                <AppIcon name="file-doc" size="13" class="text-surface-400" />选择文件
              </button>
              <button @click="entryMenu = false; pickDirectory()" class="w-full flex items-center gap-2 px-4 py-2.5 text-[12px] text-surface-700 hover:bg-brand-50/50 transition-colors border-t border-surface-50">
                <AppIcon name="folder" size="13" class="text-surface-400" />选择目录（镜像子目录结构）
              </button>
            </div>

            <!-- 目录确认闸：遍历统计如实呈现，确认才入队（防误选大目录） -->
            <div v-if="dirSummary" class="mb-4 border border-amber-200 bg-amber-50/60 rounded-xl px-4 py-3">
              <p class="text-[12px] font-medium text-surface-700">
                目录「{{ dirSummary.rootName }}」：将上传 {{ dirSummary.files.length }} 个文件{{ dirSummary.truncated ? '（已达 500 上限，多余未计入）' : '' }}
              </p>
              <p class="text-[10px] text-surface-500 mt-1">
                子目录结构自动镜像到当前目录内；已跳过：隐藏 {{ dirSummary.skipped.hidden }} · 超大(&gt;200MB) {{ dirSummary.skipped.big }} · 超深(>8层) {{ dirSummary.skipped.deep }} · 符号链接 {{ dirSummary.skipped.link }}
              </p>
              <div class="flex justify-end gap-2 mt-2">
                <button @click="dirSummary = null" class="btn-secondary text-xs">取消</button>
                <button @click="confirmDir" class="btn-primary text-xs">加入列表</button>
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
                class="group relative flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg border border-surface-200 hover:border-surface-300 hover:bg-surface-50/50 transition-all duration-150 overflow-hidden"
              >
                <div class="w-7 h-7 rounded-md bg-surface-50 border border-surface-100 flex items-center justify-center flex-shrink-0" :class="item.uploading ? 'opacity-50' : ''">
                  <AppIcon name="file-doc" size="13" class="text-surface-500" />
                </div>
                <div class="flex-1 min-w-0 flex items-center gap-1">
                  <span v-if="dirOf(item.relPath)" class="text-[9px] text-surface-400 flex-shrink-0 max-w-[120px] truncate" v-tooltip="`目录：${dirOf(item.relPath)}`">{{ dirOf(item.relPath) }}</span>
                  <input
                    v-model="item.name"
                    @focus="$event.target.select()"
                    @blur="() => pasteArea?.focus()"
                    :disabled="item.uploading"
                    class="flex-1 min-w-0 text-[11px] text-surface-800 bg-transparent border border-transparent focus:border-brand-400 focus:bg-white rounded px-1 py-0.5 outline-none transition-colors"
                    v-tooltip="`编辑文件名（当前：${item.name}.${item.ext}）`"
                  />
                  <span class="text-[10px] text-surface-400 flex-shrink-0">.{{ item.ext }}</span>
                  <span v-if="item.uploading" class="text-[10px] text-brand-600 tabular-nums flex-shrink-0">{{ item.progress ?? 0 }}%</span>
                </div>
                <button
                  @click="removeUpload(idx)"
                  class="w-5 h-5 flex items-center justify-center rounded text-surface-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 opacity-0 group-hover:opacity-100"
                  v-tooltip="item.uploading ? '取消上传' : '移除'"
                >
                  <AppIcon name="x" size="11" />
                </button>
                <!-- 粘贴分片上传进度条 -->
                <div v-if="item.uploading" class="absolute left-0 bottom-0 h-[2px] w-full bg-surface-100">
                  <div class="h-full bg-brand-500 transition-all duration-200" :style="{ width: (item.progress ?? 0) + '%' }"></div>
                </div>
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
