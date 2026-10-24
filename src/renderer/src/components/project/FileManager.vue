<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { confirm, dangerConfirm, alert } from '@/composables/useConfirm'
import AppIcon from '@/components/common/AppIcon.vue'
import FolderTree from '@/components/common/FolderTree.vue'
import UploadModal from '@/components/shared/UploadModal.vue'
import { formatDateTime } from '@/utils/time'
import * as api from '@/api/db-client'

const { updateFileMeta, openFile, downloadFile, copyFileToClipboard, fetchFileTree, createFolder, renameFolder, deleteFolder, moveFolder, reorderFolder } = api
import { toast } from '@/composables/useToast'
import { useMultiSelect } from '@/composables/useMultiSelect'

const props = defineProps({
  projectId: { type: String, required: true },
  files: { type: Array, default: () => [] }
})

const projectStore = useProjectStore()
const authStore = useAuthStore()

const isArchived = computed(() => {
  const project = projectStore.getProjectById(props.projectId)
  return project?.isArchived || false
})

// ─── State ────────────────────────────────────────────────────────────────────
const fileList = ref([])
const isFetching = ref(false)
const allFolders = ref<any[]>([])
const selectedFolderId = ref<string|null>(null)

// ─── File name editing ───────────────────────────────────────────────────────
const editingNameFileId = ref<string|null>(null)
const editingNameValue = ref('')
function startEditName(file: any) {
  editingNameFileId.value = file.id
  editingNameValue.value = file.displayName || file.custom_name || file.name || ''
  nextTick(() => { const el = document.querySelector<HTMLInputElement>('[data-name-input]'); el?.focus(); el?.select() })
}
async function saveFileName(fileId: string) {
  const v = editingNameValue.value.trim()
  editingNameFileId.value = null
  if (!v) return
  const f = fileList.value.find((x: any) => x.id === fileId)
  if (f && v !== (f.displayName || f.custom_name || f.name)) {
    try { await updateFileMeta(fileId, { customName: v }); f.custom_name = v; f.name = v; f.displayName = v; toast({ message: '文件名已更新', type: 'success' }) } catch { toast({ message: '重命名失败', type: 'error' }) }
  }
}
function cancelEditName() { editingNameFileId.value = null }

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
const folderFilter = ref('')
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
  e.preventDefault()
  dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onFileDragMove)
  dragFileTimer.value = setTimeout(() => { isDraggingFile.value = true; dragFileId.value = fileId; dragStarted.value = true }, 200)
  document.addEventListener('mouseup', onFileDragUpPre)
}
function onFileDragUpPre() { document.removeEventListener('mouseup', onFileDragUpPre); document.removeEventListener('mousemove', onFileDragMove); if (dragFileTimer.value) { clearTimeout(dragFileTimer.value); dragFileTimer.value = null }; if (isDraggingFile.value) { markDragFinished(); onFileDragUp() } }
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
    const idsToMove = selectedCount.value > 1 && isSelected(fileId)
      ? [...selectedIds.value]
      : [fileId]
    for (const rid of idsToMove) {
      await updateFileMeta(rid, { folderId })
    }
    await loadFiles()
    clearMultiSelect()
    toast({ message: idsToMove.length > 1 ? `已移动 ${idsToMove.length} 个文件` : '文件已移动', type: 'success' })
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

const { selectedIds, clear: clearMultiSelect, isSelected, selectedCount, handleItemClick, handleItemMouseDown, markDragFinished } = useMultiSelect(filteredFiles as any)

function onFileMouseDown(e: MouseEvent, fid: string) {
  if ((e.target as HTMLElement).closest('input, button, a')) return
  // 修饰键在 mousedown 即完成选择（click 只挂在文件名上，卡片空白处也要能多选）
  if (e.ctrlKey || e.metaKey || e.shiftKey) {
    e.preventDefault()
    handleItemClick(e, fid)
    return
  }
  if (!handleItemMouseDown(e, fid)) return
  startFileDrag(e, fid)
}

/** 整行点击：修饰键多选；普通点击多选回退为单选。打开文件只认文件名点击 */
function onFileRowClick(e: MouseEvent, fid: string) {
  handleItemClick(e, fid)
}

function selectFolder(id: string|null) {
  selectedFolderId.value = id
  clearMultiSelect()
}

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

// ─── Upload new file (选择 / 粘贴，经 UploadModal) ─────────────────────────
const uploadModalRef = ref<InstanceType<typeof UploadModal> | null>(null)

function triggerNewFile() {
  uploadModalRef.value?.open()
}

async function resolveProjectBackendId(): Promise<string | undefined> {
  try {
    return await projectStore.syncProjectToBackend(props.projectId)
  } catch (e) {
    console.warn('[FileManager] 同步项目后端失败', e)
    return undefined
  }
}

async function handleProjectUploaded() {
  await loadFiles()
  await projectStore.fetchProjectDynamics(props.projectId)
}

async function handleProjectUploadDone({ ok }: { ok: number; fail: number }) {
  if (ok > 0) {
    await projectStore.addProjectDynamic(props.projectId, `上传了 ${ok} 个文件`)
  }
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

async function copyFile(fileId) {
  try {
    const r = await copyFileToClipboard(fileId)
    if (r?.success) {
      toast({ message: `已复制「${r.fileName || '文件'}」，可粘贴到文件夹或聊天软件`, type: 'success' })
    } else {
      toast({ message: r?.error || '复制失败', type: 'error' })
    }
  } catch (e) {
    toast({ message: '复制失败: ' + ((e && e.message) || e || ''), type: 'error' })
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
      <button v-if="!isArchived" @click="$refs.folderTree?.startAddFolder()" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'新建目录'"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <div class="px-2 py-1.5 border-b border-surface-100 shrink-0">
      <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
        <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
        <input v-model="folderFilter" type="text" class="flex-1 min-w-0 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="筛选目录..."/>
      </div>
    </div>
    <FolderTree ref="folderTree" :folders="allFolders" :selected-id="selectedFolderId" :project-id="projectId" :external-drag-target-id="isDraggingFile ? dragTargetId : null" :filter-query="folderFilter"
      @select="selectFolder"
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
        <button v-if="!isArchived" @click="triggerNewFile" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"><AppIcon name="plus" size="10" stroke="2.5"/>上传文件</button>
      </div>
    </div>

    <div class="flex-1 overflow-y-auto p-2 space-y-1">
      <div v-if="isFetching" class="text-center py-8"><p class="text-xs text-surface-400">加载中...</p></div>
      <div v-else-if="filteredFiles.length === 0" class="text-center py-8"><p class="text-xs text-surface-400">暂无文件</p></div>
      <div v-for="file in filteredFiles" :key="file.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg transition-colors group hover:bg-surface-50 border border-transparent hover:border-surface-200 cursor-pointer select-none" :class="[isDraggingFile && dragFileId===file.id ? 'opacity-30' : '', isSelected(file.id) ? 'bg-blue-50/70 border-blue-200' : '']" @mousedown="onFileMouseDown($event, file.id)" @click="onFileRowClick($event, file.id)">
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 text-[9px] font-bold tracking-wider" :class="getFileColor(file.name)">{{ (file.name || '').split('.').pop()?.toUpperCase() || 'FILE' }}</div>
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 min-w-0">
            <input v-if="editingNameFileId === file.id" v-model="editingNameValue" data-name-input class="text-[12px] font-medium text-surface-800 bg-transparent outline-none border-b border-brand-400 w-full min-w-0 select-text" @keyup.enter="saveFileName(file.id)" @blur="saveFileName(file.id)" @mousedown.stop @keyup.escape="cancelEditName()"/>
            <p v-else class="text-[12px] font-medium text-surface-800 truncate inline-block max-w-full hover:text-blue-600 transition-colors" v-tooltip.auto="file.displayName || file.custom_name || file.name" @click.stop="previewFile(file.id)">{{ file.displayName || file.custom_name || file.name }}</p>
            <span class="text-[11px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ formatFileSize(file.file_size) }}</span>
            <span class="text-[11px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ formatDateTime(file.updated_at || file.created_at) }}</span>
          </div>
          <!-- 标签行 -->

          <div v-if="editingRemarkFileId === file.id" class="mt-1">
            <input v-model="editingRemarkValue" data-remark-input class="w-full text-[11px] text-surface-600 bg-transparent outline-none border-b border-brand-400 pb-0.5 select-text" placeholder="输入备注..." @keyup.enter="saveRemark(file.id)" @blur="saveRemark(file.id)" @mousedown.stop @keyup.escape="cancelEditRemark()"/>
          </div>
          <div v-else class="mt-0.5 flex items-center gap-1">
            <p v-if="file.remark" class="text-[10px] text-surface-400 truncate cursor-pointer hover:text-blue-600" @click="startEditRemark(file)">{{ file.remark }}</p>
            <p v-else class="text-[10px] text-surface-300 cursor-pointer hover:text-blue-600" @click="startEditRemark(file)">无备注</p>
            <button @mousedown.stop @click.stop="startEditRemark(file)" class="shrink-0 w-4 h-4 flex items-center justify-center rounded text-surface-300 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100" v-tooltip="'编辑备注'"><AppIcon name="edit" size="10"/></button>
          </div>
        </div>
        <div class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
          <button v-if="!isArchived" @mousedown.stop @click.stop="startEditName(file)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'编辑文件名'"><AppIcon name="edit" size="13"/></button>
          <button @click.stop="downloadFileFromId(file.id, file.displayName || file.name, (file.name || '').split('.').pop())" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'下载'"><AppIcon name="download" size="12"/></button>
          <button @click.stop="copyFile(file.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'复制文件到剪贴板'"><AppIcon name="copy" size="12"/></button>
          <button v-if="!isArchived" @click.stop="deleteFile(file.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" v-tooltip="'删除'"><AppIcon name="trash" size="12"/></button>
        </div>
      </div>
    </div>
  </div>

  <!-- UploadModal：选择文件 / 粘贴上传 -->
  <UploadModal
    ref="uploadModalRef"
    entity-type="project"
    title="上传项目文件"
    folder-tree
    :resolve-entity-id="resolveProjectBackendId"
    :folder-id="selectedFolderId && selectedFolderId !== '__uncategorized__' ? selectedFolderId : null"
    :creator="authStore.currentUser.name"
    @uploaded="handleProjectUploaded"
    @done="handleProjectUploadDone"
  />

  <!-- Drag ghost -->
  <Teleport to="body">
    <div v-if="isDraggingFile" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}">
      {{ selectedCount > 1 ? `移动 ${selectedCount} 项` : `${filteredFiles.find(f => f.id === dragFileId)?.displayName || filteredFiles.find(f => f.id === dragFileId)?.custom_name || filteredFiles.find(f => f.id === dragFileId)?.name || '文件'}` }}
    </div>
  </Teleport>
</div>
</template>
