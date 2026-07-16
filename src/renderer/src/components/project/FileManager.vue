<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { confirm, dangerConfirm, alert } from '@/composables/useConfirm'
import { useUploadProgress } from '@/composables/useUploadProgress'
import AppIcon from '@/components/common/AppIcon.vue'
import FolderTree from '@/components/common/FolderTree.vue'
import { formatDateTime } from '@/utils/time'
import * as api from '@/api/db-client'

const { updateFileMeta, openFile, downloadFile, fetchFileTree, createFolder, renameFolder, deleteFolder, moveFolder, reorderFolder } = api
import { toast } from '@/composables/useToast'

const props = defineProps({
  projectId: { type: String, required: true },
  files: { type: Array, default: () => [] }
})

const projectStore = useProjectStore()
const authStore = useAuthStore()
const { start: upStart, done: upDone, fail: upFail } = useUploadProgress()

const isArchived = computed(() => {
  const project = projectStore.getProjectById(props.projectId)
  return project?.isArchived || false
})

// ─── State ────────────────────────────────────────────────────────────────────
const fileList = ref([])
const isFetching = ref(false)
const allFolders = ref<any[]>([])
const selectedFolderId = ref<string|null>(null)

const showUploadForm = ref(false)
const newFile = ref({ note: '', file: null, customName: '' })
const uploadDragActive = ref(false)

// ─── Folder operations ──────────────────────────────────────────────────────
async function loadFolders() {
  try { const r: any = await fetchFileTree(props.projectId); allFolders.value = r?.folders || [] } catch { allFolders.value = [] }
}
async function onFolderCreate(name: string, parentId: string|null) {
  try { await createFolder(name, parentId, props.projectId); await loadFolders(); toast({ message: '目录已创建', type: 'success' }) } catch (e: any) { toast({ message: '创建失败', type: 'error' }) }
}
async function onFolderRename(id: string, name: string) {
  try { await renameFolder(id, name); await loadFolders() } catch (e: any) { toast({ message: '重命名失败', type: 'error' }) }
}
async function onFolderDelete(id: string) {
  const ok = await dangerConfirm({ title: '删除目录', message: '确定删除此目录？', confirmText: '删除' })
  if (!ok) return
  try { const r: any = await deleteFolder(id); if (!r?.success) { toast({ message: r?.error || '删除失败', type: 'error' }); return }; await loadFolders() } catch (e) { console.error('[FileManager] deleteFolder failed:', e) }
}
async function onFolderMove(id: string, parentId: string|null) {
  try { await moveFolder(id, parentId); await loadFolders() } catch (e: any) { toast({ message: '移动失败', type: 'error' }) }
}
async function onFolderReorder(id: string, sortOrder: number) {
  try { await reorderFolder(id, sortOrder); await loadFolders() } catch (e) { console.error('[FileManager] reorderFolder failed:', e) }
}

// ─── File filter by folder ──────────────────────────────────────────────────
const filesInFolder = computed(() => {
  const files = fileList.value as any[]
  if (!selectedFolderId.value) return files
  if (selectedFolderId.value === '__uncategorized__') return files.filter(f => !f.folder_id)
  return files.filter(f => f.folder_id === selectedFolderId.value)
})

// ─── Remark editing ──────────────────────────────────────────────────────────
const editingRemarkFileId = ref<string|null>(null)
const editingRemarkValue = ref('')

function startEditRemark(file: any) {
  editingRemarkFileId.value = file.id
  editingRemarkValue.value = file.remark || ''
  nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-remark-input]')
    el?.focus(); el?.select()
  })
}
async function saveRemark(fileId: string) {
  const v = editingRemarkValue.value.trim()
  editingRemarkFileId.value = null
  const f = fileList.value.find((x: any) => x.id === fileId)
  if (f && v !== (f.remark || '')) {
    try { await updateFileMeta(fileId, { remark: v }); f.remark = v } catch (e: any) { toast({ message: '保存备注失败', type: 'error' }) }
  }
}
function cancelEditRemark() { editingRemarkFileId.value = null }

// ─── Sidebar width ────────────────────────────────────────────────────────────
const sidebarWidth = ref(220)
const isResizing = ref(false)
const resizeStartX = ref(0)
const resizeStartW = ref(0)
function onResizeStart(e: MouseEvent) { isResizing.value = true; resizeStartX.value = e.clientX; resizeStartW.value = sidebarWidth.value; document.addEventListener('mousemove', onResizeMove); document.addEventListener('mouseup', onResizeEnd) }
function onResizeMove(e: MouseEvent) { if (!isResizing.value) return; sidebarWidth.value = Math.max(140, Math.min(400, resizeStartW.value + e.clientX - resizeStartX.value)) }
function onResizeEnd() { isResizing.value = null; document.removeEventListener('mousemove', onResizeMove); document.removeEventListener('mouseup', onResizeEnd) }

// ─── File drag to folder ──────────────────────────────────────────────────
const isDraggingFile = ref(false)
const dragFileId = ref<string|null>(null)
const dragX = ref(0); const dragY = ref(0)
const dragStartX = ref(0); const dragStartY = ref(0)
const dragStarted = ref(false)
const dragFileTimer = ref<ReturnType<typeof setTimeout>|null>(null)
const dragTargetId = ref<string|null>(null)

function startFileDrag(e: MouseEvent, fileId: string) {
  dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onFileDragMove)
  dragFileTimer.value = setTimeout(() => { isDraggingFile.value = true; dragFileId.value = fileId; dragStarted.value = true }, 300)
  document.addEventListener('mouseup', onFileDragUpPre)
}
function onFileDragUpPre() { document.removeEventListener('mouseup', onFileDragUpPre); if (dragFileTimer.value) { clearTimeout(dragFileTimer.value); dragFileTimer.value = null }; if (isDraggingFile.value) onFileDragUp() }
function onFileDragMove(e: MouseEvent) {
  dragX.value = e.clientX; dragY.value = e.clientY
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const t = el?.closest('[data-drop-target]') as HTMLElement|null
  dragTargetId.value = t?.dataset.dropTarget || null
}
async function onFileDragUp() {
  document.removeEventListener('mousemove', onFileDragMove); document.removeEventListener('mouseup', onFileDragUp)
  const fileId = dragFileId.value; const tid = dragTargetId.value
  isDraggingFile.value = false; dragFileId.value = null; dragTargetId.value = null; dragStarted.value = false
  if (!fileId || !tid || tid === fileId) return
  const folderId = tid === '__root__' || tid === '__uncategorized__' ? null : tid
  try {
    await updateFileMeta(fileId, { folderId })
    await loadFiles()
    toast({ message: '文件已移动', type: 'success' })
  } catch (e: any) { toast({ message: '移动失败', type: 'error' }) }
}

// ─── Search & Tag filter ──────────────────────────────────────────────────────
const searchQuery = ref('')
const selectedTag = ref('')
const allTags = ref([]) // collected from all files
const tagDropdownOpen = ref(false)
const tagDropdownRef = ref(null)

function toggleTagDropdown() { tagDropdownOpen.value = !tagDropdownOpen.value }
function selectTag(tag) { selectedTag.value = tag; tagDropdownOpen.value = false }
function onTagDropdownClickOutside(e) {
  if (tagDropdownRef.value && !tagDropdownRef.value.contains(e.target) && !e.target.closest('[data-tag-trigger]')) {
    tagDropdownOpen.value = false
  }
}

function rebuildTags() {
  const tagSet = new Set()
  const files = Array.isArray(fileList.value) ? fileList.value : []
  files.forEach(f => {
    const tags = (f.tags || '').split(',').map(t => t.trim()).filter(Boolean)
    tags.forEach(t => tagSet.add(t))
  })
  allTags.value = [...tagSet].sort()
}

// ─── Tag editing ──────────────────────────────────────────────────────────────
const addingTagFileId = ref(null)
const newTagValue = ref('')

function parseTags(file) {
  return (file.tags || '').split(',').map(t => t.trim()).filter(Boolean)
}

function startAddTag(file) {
  addingTagFileId.value = file.id
  newTagValue.value = ''
}

async function addTag(fileId) {
  const tag = newTagValue.value.trim()
  if (!tag) { cancelAddTag(); return }
  const f = fileList.value.find(x => x.id === fileId)
  if (!f) { cancelAddTag(); return }
  const existing = parseTags(f)
  if (existing.includes(tag)) { cancelAddTag(); return }
  existing.push(tag)
  const tags = existing.join(',')
  try {
    await updateFileMeta(fileId, { tags })
    f.tags = tags
    rebuildTags()
  } catch (e) {
    console.warn('添加标签失败', e)
  }
  newTagValue.value = ''
  nextTick(() => { addingTagFileId.value = null })
}

async function removeTag(fileId, tag) {
  const f = fileList.value.find(x => x.id === fileId)
  if (!f) return
  const existing = parseTags(f).filter(t => t !== tag)
  const tags = existing.join(',')
  try {
    await updateFileMeta(fileId, { tags })
    f.tags = tags
    rebuildTags()
  } catch (e) {
    console.warn('删除标签失败', e)
  }
}

function cancelAddTag() {
  newTagValue.value = ''
  nextTick(() => { addingTagFileId.value = null })
}

// ─── AI Tag 功能保留在后台（fileGetTags / fileAddTagMapping / tagTagSingle 等后端 API 仍可用）
//     前端 UI 中已隐藏，后续需要时可恢复

function formatFileSize(bytes: number): string {
  if (!bytes) return ''
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
  return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB'
}

// ─── Filtered file list ───────────────────────────────────────────────────────
const filteredFiles = computed(() => {
  const files = filesInFolder.value
  return files.filter(f => {
    const matchSearch = !searchQuery.value.trim() ||
      (f.name || '').toLowerCase().includes(searchQuery.value.toLowerCase()) ||
      (f.displayName || f.custom_name || '').toLowerCase().includes(searchQuery.value.toLowerCase())
    const matchTag = !selectedTag.value ||
      (f.tags || '').split(',').map((t: string) => t.trim()).includes(selectedTag.value)
    return matchSearch && matchTag
  })
})

// ─── Fetch on mount ───────────────────────────────────────────────────────────
let loadGen = 0
async function loadFiles() {
  isFetching.value = true
  const gen = ++loadGen
  try {
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    const [data] = await Promise.all([api.fetchFiles(backendId), loadFolders()])
    fileList.value = Array.isArray(data) ? data : []
    rebuildTags()
  } catch (e) {
    if (gen === loadGen) fileList.value = fileList.value.length ? fileList.value : []
  } finally {
    if (gen === loadGen) isFetching.value = false
  }
}
onMounted(() => { loadFiles(); document.addEventListener('click', onTagDropdownClickOutside) })
onUnmounted(() => {
  document.removeEventListener('click', onTagDropdownClickOutside)
  document.removeEventListener('mousemove', onResizeMove); document.removeEventListener('mouseup', onResizeEnd)
  document.removeEventListener('mousemove', onFileDragMove); document.removeEventListener('mouseup', onFileDragUpPre)
  if (dragFileTimer.value) clearTimeout(dragFileTimer.value)
})

// ─── Upload new file ──────────────────────────────────────────────────────────
async function triggerNewFile() {
  newFile.value = { note: '', file: null, customName: '' }
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const result = await open({ filters: [], multiple: false })
    if (!result) return
    const path = typeof result === 'string' ? result : (result as any).path || result
    const name = path.replace(/^.*[\\\/]/, '')
    newFile.value.file = { path, name }
  } catch (e) { console.warn('文件选择失败', e); toast({ message: '文件选择仅支持桌面环境，请使用拖拽上传', type: 'error' }) }
}
function onUploadDragOver(e) { e.preventDefault(); uploadDragActive.value = true }
function onUploadDragLeave() { uploadDragActive.value = false }
function onUploadDrop(e) {
  e.preventDefault()
  uploadDragActive.value = false
  const file = e.dataTransfer?.files?.[0]
  if (file) {
    const path = (file as any).path
    if (path) {
      newFile.value.file = { path, name: file.name }
    } else {
      toast({ message: '拖拽上传需要桌面环境支持，请点击选择文件', type: 'error' })
    }
  }
}

function uploadFile() {
  if (!newFile.value.file) { toast({ message: '请选择一个文件', type: 'error' }); return }
  const file = newFile.value.file
  const ext = file.name?.includes('.') ? file.name.split('.').pop() || '' : ''
  let rawName = newFile.value.customName || file.name || '未命名文件'
  if (ext && !rawName.endsWith('.' + ext)) rawName += '.' + ext
  const filePath = (file as any).path || file.name
  const note = newFile.value.note || ''

  if (!(file as any).path) {
    toast({ message: '无法获取文件路径', type: 'error' }); return
  }

  // 关闭弹窗，异步上传（右下角进度条）
  showUploadForm.value = false
  newFile.value = { note: '', file: null, customName: '' }
  const task = upStart(rawName)

  projectStore.syncProjectToBackend(props.projectId).then(backendId =>
    api.uploadFile({
      filePath, fileExt: ext, entityType: 'project', entityId: backendId,
      customName: rawName, note, creator: authStore.currentUser.name,
      folderId: selectedFolderId.value && selectedFolderId.value !== '__uncategorized__' ? selectedFolderId.value : null,
    })
  ).then(() => {
    return loadFiles()
  }).then(() => {
    return projectStore.addProjectDynamic(props.projectId, `上传了文件「${rawName}」`)
  }).then(() => {
    return projectStore.fetchProjectDynamics(props.projectId)
  }).then(() => {
    upDone(task)
  }).catch(e => {
    upFail(task, e.message || '上传失败')
  })
}

// ─── Delete file — API-backed cascade delete + physical file removal ──────────
const isDeleting = ref(false)
async function deleteFile(fileId) {
  const delOk = await dangerConfirm({ title: '删除文件', message: '确定删除该文件及其所有版本记录？此操作不可撤销。', confirmText: '删除' })
  if (!delOk) return
  isDeleting.value = true
  const targetFile = (Array.isArray(fileList.value) ? fileList.value : []).find(f => f.id === fileId)
  const fileName = targetFile?.displayName || targetFile?.custom_name || targetFile?.name || '未命名文件'
  try {
    await api.deleteFile(fileId)
    const currentFiles = Array.isArray(fileList.value) ? fileList.value : []
    fileList.value = currentFiles.filter(f => f.id !== fileId)
    rebuildTags()
    await projectStore.addProjectDynamic(props.projectId, `删除了文件「${fileName}」`)
    toast({ message: `文件「${fileName}」已删除`, type: 'success' })
  } catch (e: any) {
    toast({ message: '删除失败: ' + (e.message || e), type: 'error' })
  } finally {
    isDeleting.value = false
  }
}

// ─── Preview file (open with system default app in read-only mode)
async function previewFile(fileId) {
  try {
    const result = await openFile(fileId)
    if (!result.success) {
      alert({ title: '预览失败', message: result.error || '无法打开文件' })
    }
  } catch (e) {
    alert({ title: '预览失败', message: (e && e.message) || '未知错误' })
  }
}

// ─── Download file via save dialog
async function downloadFileFromId(fileId, suggestedName, fileExt) {
  const ext = fileExt || ''
  const fullName = suggestedName && ext ? (suggestedName.endsWith('.' + ext) ? suggestedName : suggestedName + '.' + ext) : (suggestedName || 'download')
  try {
    const result = await downloadFile(fileId, fullName)
    if (!result.success && result.error !== '已取消') {
      alert({ title: '下载失败', message: result.error || '未知错误' })
    }
  } catch (e) {
    alert({ title: '下载失败', message: (e && e.message) || '未知错误' })
  }
}

// ─── Icon & color helpers ─────────────────────────────────────────────────────
const fileTypeIcons: Record<string, string> = {
  doc: 'file-doc', docx: 'file-doc', xls: 'file-spreadsheet', xlsx: 'file-spreadsheet',
  ppt: 'presentation', pptx: 'presentation', pdf: 'file-pdf',
  jpg: 'image', jpeg: 'image', png: 'image', gif: 'image', svg: 'image', webp: 'image', bmp: 'image',
  mp4: 'video', mov: 'video', avi: 'video', mkv: 'video', webm: 'video',
  mp3: 'music', wav: 'music', flac: 'music', ogg: 'music',
  zip: 'archive', rar: 'archive', '7z': 'archive', tar: 'archive', gz: 'archive',
  js: 'code', ts: 'code', jsx: 'code', tsx: 'code', py: 'code', java: 'code',
  html: 'code', css: 'code', json: 'code', xml: 'code', yaml: 'code', yml: 'code',
  md: 'file-doc', txt: 'file-doc', csv: 'file-spreadsheet',
}
function getFileIconName(fileName) {
  const ext = (fileName || '').split('.').pop()?.toLowerCase()
  return fileTypeIcons[ext] || 'file'
}
function getFileColor(fileName) {
  const ext = (fileName || '').split('.').pop()?.toLowerCase()
  const colors = {
    doc: 'text-blue-500 bg-blue-50', docx: 'text-blue-500 bg-blue-50', md: 'text-blue-500 bg-blue-50', txt: 'text-blue-500 bg-blue-50',
    xls: 'text-emerald-500 bg-emerald-50', xlsx: 'text-emerald-500 bg-emerald-50', csv: 'text-emerald-500 bg-emerald-50',
    pdf: 'text-red-500 bg-red-50',
    ppt: 'text-orange-500 bg-orange-50', pptx: 'text-orange-500 bg-orange-50',
    jpg: 'text-purple-500 bg-purple-50', jpeg: 'text-purple-500 bg-purple-50', png: 'text-purple-500 bg-purple-50',
    gif: 'text-purple-500 bg-purple-50', svg: 'text-purple-500 bg-purple-50', webp: 'text-purple-500 bg-purple-50', bmp: 'text-purple-500 bg-purple-50',
    mp4: 'text-pink-500 bg-pink-50', mov: 'text-pink-500 bg-pink-50', avi: 'text-pink-500 bg-pink-50', mkv: 'text-pink-500 bg-pink-50', webm: 'text-pink-500 bg-pink-50',
    mp3: 'text-cyan-500 bg-cyan-50', wav: 'text-cyan-500 bg-cyan-50', flac: 'text-cyan-500 bg-cyan-50', ogg: 'text-cyan-500 bg-cyan-50',
    zip: 'text-amber-500 bg-amber-50', rar: 'text-amber-500 bg-amber-50', '7z': 'text-amber-500 bg-amber-50', tar: 'text-amber-500 bg-amber-50', gz: 'text-amber-500 bg-amber-50',
    js: 'text-yellow-500 bg-yellow-50', ts: 'text-yellow-500 bg-yellow-50', py: 'text-yellow-500 bg-yellow-50', java: 'text-yellow-500 bg-yellow-50',
    html: 'text-yellow-500 bg-yellow-50', css: 'text-yellow-500 bg-yellow-50', json: 'text-yellow-500 bg-yellow-50',
  }
  return colors[ext] || 'text-surface-500 bg-surface-100'
}
</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden">

  <!-- Sidebar: 目录树 -->
  <aside class="flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden relative" :style="{width:sidebarWidth+'px'}">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart"></div>
    <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">目录</span>
      <button v-if="!isArchived" @click="$refs.folderTree?.startAddFolder()" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="新建目录"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <FolderTree ref="folderTree" :folders="allFolders" :selected-id="selectedFolderId" :project-id="projectId"
      @select="(id) => selectedFolderId = id"
      @create="onFolderCreate"
      @rename="onFolderRename"
      @delete="onFolderDelete"
      @move="onFolderMove"
      @reorder="onFolderReorder"
    />
  </aside>

  <div class="w-px cursor-col-resize flex-shrink-0 select-none" @mousedown="onResizeStart"></div>

  <!-- File list -->
  <div class="flex-1 flex flex-col overflow-hidden min-w-0">
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">文件</span>
      <div class="flex items-center gap-1.5">
        <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
          <AppIcon name="search" size="12" class="text-surface-400"/>
          <input v-model="searchQuery" type="text" class="w-[120px] bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索文件名..."/>
        </div>
        <button v-if="!isArchived" @click="showUploadForm = true" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"><AppIcon name="plus" size="10" stroke="2.5"/>上传文件</button>
      </div>
    </div>

    <div class="flex-1 overflow-y-auto p-2 space-y-1">
      <div v-if="isFetching" class="text-center py-8"><p class="text-xs text-surface-400">加载中...</p></div>
      <div v-else-if="filteredFiles.length === 0" class="text-center py-8"><p class="text-xs text-surface-400">暂无文件</p></div>
      <div v-for="file in filteredFiles" :key="file.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg transition-colors group hover:bg-surface-50 border border-transparent hover:border-surface-200" :class="[isDraggingFile && dragFileId===file.id ? 'opacity-30 select-none' : '']" @mousedown="startFileDrag($event, file.id)">
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 text-[9px] font-bold tracking-wider" :class="getFileColor(file.name)">{{ (file.name || '').split('.').pop()?.toUpperCase() || 'FILE' }}</div>
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 min-w-0">
            <p class="text-[12px] font-medium text-surface-800 truncate cursor-pointer hover:text-blue-600 transition-colors" :title="file.displayName || file.custom_name || file.name" @click="previewFile(file.id)">{{ file.displayName || file.custom_name || file.name }}</p>
            <span class="text-[10px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ formatFileSize(file.file_size) }}</span>
            <span class="text-[10px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ formatDateTime(file.updated_at || file.created_at) }}</span>
          </div>
          <!-- 标签行 -->

          <div v-if="editingRemarkFileId === file.id" class="mt-1">
            <input v-model="editingRemarkValue" data-remark-input class="w-full text-[11px] text-surface-600 bg-transparent outline-none border-b border-brand-400 pb-0.5" placeholder="输入备注..." @keyup.enter="saveRemark(file.id)" @blur="saveRemark(file.id)" @keyup.escape="cancelEditRemark()"/>
          </div>
          <div v-else class="mt-0.5 flex items-center gap-1">
            <p v-if="file.remark" class="text-[10px] text-surface-400 truncate cursor-pointer hover:text-blue-600" @click="startEditRemark(file)">{{ file.remark }}</p>
            <p v-else class="text-[10px] text-surface-300 cursor-pointer hover:text-blue-600" @click="startEditRemark(file)">无备注</p>
            <button @click.stop="startEditRemark(file)" class="shrink-0 w-4 h-4 flex items-center justify-center rounded text-surface-300 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100" title="编辑备注"><AppIcon name="edit" size="10"/></button>
          </div>
        </div>
        <div class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
          <button @click.stop="downloadFileFromId(file.id, file.displayName || file.name, (file.name || '').split('.').pop())" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="下载"><AppIcon name="download" size="12"/></button>
          <button v-if="!isArchived" @click.stop="deleteFile(file.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="trash" size="12"/></button>
        </div>
      </div>
    </div>
  </div>

  <!-- Upload Modal -->
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showUploadForm" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/40 backdrop-blur-sm" />
        <div class="relative bg-white rounded-2xl shadow-xl w-full max-w-md mx-4 p-6 z-10">
          <h3 class="text-lg font-semibold text-surface-800 mb-4">上传文件</h3>
          <div class="space-y-4">
            <div>
              <label class="block text-xs font-medium text-surface-500 mb-1.5">选择文件 *</label>
              <div @click="triggerNewFile" @dragover.prevent="onUploadDragOver" @dragenter.prevent="onUploadDragOver" @dragleave="onUploadDragLeave" @drop.prevent="onUploadDrop" class="border-2 border-dashed rounded-xl p-6 text-center cursor-pointer transition-colors" :class="uploadDragActive ? 'border-blue-400 bg-blue-50/40' : 'border-slate-200 hover:border-blue-400 hover:bg-blue-50/30'">
                <AppIcon name="upload" class="w-8 h-8 mx-auto mb-2 text-slate-300" />
                <p v-if="!newFile.file" class="text-xs text-slate-400">拖拽文件到此处或点击选择</p>
                <p v-else class="text-xs text-blue-600 font-medium truncate">{{ newFile.file.name }}</p>
              </div>
            </div>
            <div>
              <label class="block text-xs font-medium text-surface-500 mb-1.5">自定义文件名</label>
              <input v-model="newFile.customName" type="text" class="input-field" placeholder="留空使用原始文件名" />
            </div>
            <div>
              <label class="block text-xs font-medium text-surface-500 mb-1.5">备注</label>
              <input v-model="newFile.note" type="text" class="input-field" placeholder="文件说明" />
            </div>
          </div>
          <div class="flex justify-end gap-3 mt-5 pt-4 border-t border-slate-100">
            <button @click="showUploadForm = false" class="btn-secondary">取消</button>
            <button @click="uploadFile" class="btn-primary"><AppIcon name="upload" class="w-3 h-3" /><span>上传</span></button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- Drag ghost -->
  <Teleport to="body">
    <div v-if="isDraggingFile" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}">
      {{ filteredFiles.find(f => f.id === dragFileId)?.displayName || filteredFiles.find(f => f.id === dragFileId)?.custom_name || filteredFiles.find(f => f.id === dragFileId)?.name || '文件' }}
    </div>
  </Teleport>
</div>
</template>
