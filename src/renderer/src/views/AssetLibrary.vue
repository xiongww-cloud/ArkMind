<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { useResizablePanels } from '@/composables/useResizablePanels'
import { useMultiSelect } from '@/composables/useMultiSelect'
import AppIcon from '@/components/common/AppIcon.vue'
import UploadModal from '@/components/shared/UploadModal.vue'
import {
  fetchFileTree, listGlobalFiles,
  exportFile, deleteFileFromDomain, previewFile, updateFileMeta, copyFileToClipboard,
  moveFileToFolder,
  createFolder, renameFolder, deleteFolder, moveFolder, reorderFolder,
} from '@/api/db-client'

interface Folder { id: string; name: string; node_type: string; allow_children: boolean; file_count: number; parent_id?: string|null; _depth?: number; _hasChildren?: boolean }
interface AssetFile { id: string; file_ext?: string; file_size: number; parsed_text?: string; updated_at: string; created_at: string; lib_gn?: string; lib_cn?: string; lib_remark?: string; lib_folder?: string|null }

const UNCATEGORIZED = '未分类' as const

const loading = ref(false)
const fileTypeFilter = ref<'all'|'word'|'excel'|'ppt'|'pdf'|'image'|'text'|'archive'|'other'>('all')
const filterDropdownOpen = ref(false)
const editingFileId = ref<string|null>(null)
const editingFileName = ref('')
const selectedFolderId = ref<string|null>(null)

const allFolders = ref<Folder[]>([])
const allFiles = ref<AssetFile[]>([])
const rootFileCount = ref(0)

// ─── Layout ──────────────────────────────────────────────────────────
const { sidebarWidth, listWidth, isResizing, onResizeStart } = useResizablePanels()

// ─── Tree (参考 ProductCenter buildTree) ─────────────────────────────────
const collapsedFolders = ref<Set<string>>(new Set())
function toggleFolder(id: string) {
  const s = new Set(collapsedFolders.value)
  if (s.has(id)) s.delete(id); else s.add(id)
  collapsedFolders.value = s
}

function buildTree(dirs: Folder[], collapsed: Set<string>, q: string): Folder[] {
  const roots = dirs.filter(d => !d.parent_id)
  const children = dirs.filter(d => d.parent_id)
  const matchSet = new Set<string>()
  if (q) {
    function markUp(id: string|null) { while (id) { matchSet.add(id); const p = dirs.find(d => d.id === id); id = p?.parent_id || null } }
    for (const d of dirs.filter(d => d.name.toLowerCase().includes(q))) markUp(d.id)
  }
  function hasChildren(pid: string) { return children.some(c => c.parent_id === pid && (!q || matchSet.has(c.id))) }
  function buildChildren(pid: string, depth: number): Folder[] {
    if (collapsed.has(pid)) return []
    const direct = children.filter(c => c.parent_id === pid && (!q || matchSet.has(c.id)))
    return [...direct].sort((a: any, b: any) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || (a.name || '').localeCompare(b.name || '')).flatMap(c => {
      const subs = buildChildren(c.id, depth + 1)
      return [{ ...c, _depth: depth, _hasChildren: subs.length > 0 || hasChildren(c.id) }, ...subs]
    })
  }
  const visible = roots.filter(r => !q || matchSet.has(r.id))
  return visible.flatMap(r => {
    const subs = buildChildren(r.id, 1)
    return [{ ...r, _depth: 0, _hasChildren: subs.length > 0 || hasChildren(r.id) }, ...subs]
  })
}

const folderTree = computed(() => buildTree(allFolders.value, collapsedFolders.value, ''))

function selectFolder(id: string|null) { selectedFolderId.value = id; clearMultiSelect() }

// ─── Data load ────────────────────────────────────────────────────────
onMounted(async () => { await loadFolders(); await loadFiles() })
async function loadFolders() { try { const r: any = await fetchFileTree(); allFolders.value = r?.folders || []; rootFileCount.value = r?.rootFileCount || 0; collapsedFolders.value = new Set(allFolders.value.map(f => f.id)) } catch { allFolders.value = [] } }
async function loadFiles() { loading.value = true; try { const r: any = await listGlobalFiles(); allFiles.value = Array.isArray(r) ? r : (r?.files || []) } catch { allFiles.value = [] }; loading.value = false }

// ─── File display name ────────────────────────────────────────────────
function fileDisplayName(f: AssetFile): string { return f.lib_gn || f.lib_cn || '未命名' }
function startFileRename(f: AssetFile) {
  editingFileId.value = f.id
  editingFileName.value = f.lib_gn || f.lib_cn || ''
  import('vue').then(({ nextTick }) => nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-lib-rename-input]')
    el?.focus(); el?.select()
  }))
}
async function confirmFileRename(f: AssetFile) {
  const n = editingFileName.value.trim()
  if (n) { try { await updateFileMeta(f.id, { customName: n, relationId: (f as any).relation_id }); await loadFiles() } catch (e: any) { toast({ message: '重命名失败: ' + (e.message || ''), type: 'error' }) } }
  editingFileId.value = null
}
function cancelFileRename() { editingFileId.value = null }

// ─── File remark editing ────────────────────────────────────────────
const editingRemarkFileId = ref<string|null>(null)
const editingRemarkValue = ref('')
function startEditRemark(f: AssetFile) {
  editingRemarkFileId.value = f.id
  editingRemarkValue.value = f.lib_remark || ''
  import('vue').then(({ nextTick }) => nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-lib-remark-input]')
    el?.focus(); el?.select()
  }))
}
async function saveRemark(f: AssetFile) {
  const v = editingRemarkValue.value.trim()
  editingRemarkFileId.value = null
  if (v !== (f.lib_remark || '')) {
    try { await updateFileMeta(f.id, { remark: v }); f.lib_remark = v } catch { toast({ message: '保存备注失败', type: 'error' }) }
  }
}
function cancelEditRemark() { editingRemarkFileId.value = null }

// ─── File type category mapping ─────────────────────────────────────
const filterCategoryMap: Record<string, string[]> = {
  word:    ['doc','docx'],
  excel:   ['xls','xlsx','csv'],
  ppt:     ['ppt','pptx'],
  pdf:     ['pdf'],
  image:   ['png','jpg','jpeg','gif','svg','webp','bmp','ico'],
  text:    ['txt','md','json','xml','yml','yaml','log','sql'],
  archive: ['zip','rar','7z','tar','gz'],
  other:   [],
}
const filterCategoryLabels: Record<string, string> = {
  word:'Word', excel:'Excel', ppt:'PPT', pdf:'PDF', image:'图片', text:'文本', archive:'压缩包', other:'其他',
}

function toggleFilterDropdown() { filterDropdownOpen.value = !filterDropdownOpen.value }
function selectFilterType(val: string) { fileTypeFilter.value = val as any; filterDropdownOpen.value = false }

// 点击其他区域关闭下拉
function onPanelClick() { filterDropdownOpen.value = false }

const filteredFiles = computed(() => {
  let result = allFiles.value
  if (selectedFolderId.value) {
    if (selectedFolderId.value === '__uncategorized__') {
      result = result.filter(f => !(f as any).lib_folder)
    } else {
      result = result.filter(f => (f as any).lib_folder === selectedFolderId.value)
    }
  }
  const q = folderSearch.value.toLowerCase().trim()
  if (q) result = result.filter(a => fileDisplayName(a).toLowerCase().includes(q))
  if (fileTypeFilter.value !== 'all') {
    const exts = filterCategoryMap[fileTypeFilter.value] || []
    if (exts.length) {
      result = result.filter(a => exts.includes((a.file_ext||'').toLowerCase()))
    } else {
      // "other" — exclude all known categories
      const allKnown = Object.values(filterCategoryMap).flat().filter(x => x)
      result = result.filter(a => !allKnown.includes((a.file_ext||'').toLowerCase()))
    }
  }
  return [...result].sort((a,b) => (b.updated_at||b.created_at||'').localeCompare(a.updated_at||a.created_at||''))
})

const { selectedIds, clear: clearMultiSelect, selectSingle: multiSelectSingle, selectRange: multiSelectRange, isSelected, selectedCount } = useMultiSelect(filteredFiles as any)
const pendingShiftDown = ref(false)

function onFileMouseDown(e: MouseEvent, fid: string) {
  if ((e.target as HTMLElement).closest('input')) return
  if (e.shiftKey) {
    e.preventDefault()
    multiSelectRange(fid)
    pendingShiftDown.value = true
    return
  }
  pendingShiftDown.value = false
  if (!isSelected(fid)) multiSelectSingle(fid)
  startFileDrag(e, fid)
}

function onFileCardClick(e: MouseEvent, fid: string) {
  if (e.shiftKey || pendingShiftDown.value) return
  clearMultiSelect()
  if (!dragJustFinished.value) handlePreview(fid)
}

// ─── File type display ──────────────────────────────────────────────────
const fileTypeColors: Record<string,string> = {
  pdf: 'text-red-600 bg-red-50',
  doc: 'text-blue-600 bg-blue-50', docx: 'text-blue-600 bg-blue-50',
  xls: 'text-emerald-600 bg-emerald-50', xlsx: 'text-emerald-600 bg-emerald-50', csv: 'text-emerald-600 bg-emerald-50',
  ppt: 'text-orange-600 bg-orange-50', pptx: 'text-orange-600 bg-orange-50',
  png: 'text-purple-600 bg-purple-50', jpg: 'text-purple-600 bg-purple-50', jpeg: 'text-purple-600 bg-purple-50', gif: 'text-purple-600 bg-purple-50', svg: 'text-purple-600 bg-purple-50', webp: 'text-purple-600 bg-purple-50',
  zip: 'text-slate-600 bg-slate-50', rar: 'text-slate-600 bg-slate-50', '7z': 'text-slate-600 bg-slate-50',
  txt: 'text-gray-600 bg-gray-50', md: 'text-gray-600 bg-gray-50',
}
function fileTypeColor(ext?: string): string { const key = ext?.toLowerCase() || ''; return fileTypeColors[key] || 'text-slate-500 bg-slate-50' }

// ─── Upload ───────────────────────────────────────────────────────────
// 上传弹窗：使用公共 UploadModal（选择文件 或 粘贴，支持编辑文件名、逐个上传入库）
const uploadModalRef = ref<InstanceType<typeof UploadModal> | null>(null)

function handleUpload() {
  uploadModalRef.value?.open()
}

async function handleLibraryUploaded() {
  await loadFiles()
  await loadFolders()
}

// ─── Folder CRUD ──────────────────────────────────────────────────────
const editingFolderId = ref<string|null>(null)
const folderEditName = ref('')
const addingNew = ref(false)
const addingParentId = ref<string|null>(null)
const newFolderName = ref('')
const folderSearch = ref('')

function startAddFolder() { let name = '新文件夹'; let i = 1; while (isDuplicateFolderName(name, undefined, null)) { name = '新文件夹 ' + (++i) }; addingNew.value = true; addingParentId.value = null; newFolderName.value = name; setTimeout(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-add]'); el?.focus(); el?.select() }, 50) }
function startAddSubFolder(parentId: string) { let name = '新文件夹'; let i = 1; while (isDuplicateFolderName(name, undefined, parentId)) { name = '新文件夹 ' + (++i) }; addingNew.value = true; addingParentId.value = parentId; newFolderName.value = name; setTimeout(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-add]'); el?.focus(); el?.select() }, 50) }
const addingSubmitting = ref(false)
function isDuplicateFolderName(name: string, excludeId?: string, parentId?: string|null): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return allFolders.value.some((f: any) => f.id !== excludeId && f.name.toLowerCase() === q && (f.parent_id || null) === (parentId || null))
}
async function confirmAddFolder() {
  if (addingSubmitting.value) return
  const n = newFolderName.value.trim()
  if (!n) { addingNew.value = false; return }
  if (isDuplicateFolderName(n, undefined, addingParentId.value || null)) { toast({ message: '同级目录下已存在同名文件夹', type: 'error' }); addingSubmitting.value = false; return }
  addingSubmitting.value = true
  addingNew.value = false; const pid = addingParentId.value; addingParentId.value = null; newFolderName.value = ''
  try { await createFolder(n, pid || null); await loadFolders(); if (pid) { const s = new Set(collapsedFolders.value); s.delete(pid); collapsedFolders.value = s }; toast({ message: '目录已创建', type: 'success' }) } catch (e: any) { toast({ message: '创建失败: ' + (e.message || ''), type: 'error' }) }
  addingSubmitting.value = false
}
function cancelAddFolder() { addingNew.value = false }
function startRename(id: string, name: string) { editingFolderId.value = id; folderEditName.value = name; setTimeout(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-edit]'); el?.focus(); el?.select() }, 50) }
async function confirmRename(id: string) {
  const n = folderEditName.value.trim(); if (!n) { editingFolderId.value = null; return }
  const f = allFolders.value.find((x: any) => x.id === id)
  if (isDuplicateFolderName(n, id, f?.parent_id || null)) { toast({ message: '同级目录下已存在同名文件夹', type: 'error' }); editingFolderId.value = null; return }
  try { await renameFolder(id, n); await loadFolders() } catch (e: any) { toast({ message: '重命名失败: ' + (e.message || ''), type: 'error' }) }
  editingFolderId.value = null
}
async function handleDeleteFolder(id: string) {
  const ok = await dangerConfirm({ title: '删除目录', message: '确定删除此目录？', confirmText: '删除' })
  if (!ok) return
  try { const r: any = await deleteFolder(id); if (!r?.success) { toast({ message: r?.error || '删除失败', type: 'error' }); return }; await loadFolders() } catch (e) { console.error('[Asset] deleteFolder failed:', e) }
}

// ─── Drag & drop ──────────────────────────────────
const isDragging = ref(false)
const dragJustFinished = ref(false)
const dragType = ref<'folder'|'file'|null>(null)
const dragFolderId = ref<string|null>(null)
const dragFileId = ref<string|null>(null)
const dragX = ref(0); const dragY = ref(0)
const dragStartX = ref(0); const dragStartY = ref(0)
const dragStarted = ref(false)
const dragTimer = ref<ReturnType<typeof setTimeout>|null>(null)
const dragTargetId = ref<string|null>(null)
const dragInsert = ref<{targetId:string;side:'above'|'below'|'inside'}|null>(null)

function startFolderDrag(e: MouseEvent, fid: string) {
  e.preventDefault()
  dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onDragMove)
  dragTimer.value = setTimeout(() => { isDragging.value = true; dragType.value = 'folder'; dragFolderId.value = fid; dragStarted.value = true }, 300)
  document.addEventListener('mouseup', onDragUpPre)
}
function startFileDrag(e: MouseEvent, fid: string) {
  e.preventDefault()
  dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onDragMove)
  dragTimer.value = setTimeout(() => { isDragging.value = true; dragType.value = 'file'; dragFileId.value = fid; dragStarted.value = true }, 300)
  document.addEventListener('mouseup', onDragUpPre)
}
function onDragUpPre() { document.removeEventListener('mouseup', onDragUpPre); document.removeEventListener('mousemove', onDragMove); if (dragTimer.value) { clearTimeout(dragTimer.value); dragTimer.value = null }; if (isDragging.value) onDragUp() }
function onDragMove(e: MouseEvent) {
  dragX.value = e.clientX; dragY.value = e.clientY
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const t = el?.closest('[data-drop-target]') as HTMLElement|null
  dragTargetId.value = t?.dataset.dropTarget || null
  if (t && dragType.value === 'folder') {
    const rect = t.getBoundingClientRect()
    const relY = e.clientY - rect.top
    const third = rect.height / 3
    const side = relY < third ? 'above' : relY > rect.height - third ? 'below' : 'inside'
    dragInsert.value = { targetId: t.dataset.dropTarget || '', side }
  } else {
    dragInsert.value = null
  }
}
async function onDragUp() {
  document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp)
  const type = dragType.value; const fid = dragFolderId.value; const fileId = dragFileId.value
  const tid = dragTargetId.value; const insert = dragInsert.value
  isDragging.value = false; dragType.value = null; dragFolderId.value = null; dragFileId.value = null; dragTargetId.value = null; dragInsert.value = null; dragStarted.value = false
  dragJustFinished.value = true
  setTimeout(() => { dragJustFinished.value = false }, 0)
  if (!tid) return
  if (type === 'folder') {
    if (!fid || fid === tid) return
    try {
      if (insert?.side === 'inside') {
        const parentId = tid === '__root__' ? null : tid
        await moveFolder(fid, parentId)
      } else {
        // Reorder: set parent to same as target, adjust sort_order
        const target = allFolders.value.find(f => f.id === tid)
        if (!target) return
        const targetOrder = (target as any).sort_order ?? 0
        await moveFolder(fid, target.parent_id || null)
        await reorderFolder(fid, insert?.side === 'above' ? targetOrder - 1 : targetOrder + 1)
      }
      await loadFolders(); toast({ message: '目录已移动', type: 'success' })
    } catch { toast({ message: '移动失败', type: 'error' }) }
  } else if (type === 'file') {
    if (!fileId || tid === fileId) return
    try {
      const folderId = tid === '__root__' || tid === '__uncategorized__' ? null : tid
      const idsToMove = selectedCount.value > 1 && isSelected(fileId)
        ? [...selectedIds.value]
        : [fileId]
      for (const rid of idsToMove) {
        await moveFileToFolder(rid, folderId)
      }
      await loadFiles(); await loadFolders()
      clearMultiSelect()
      toast({ message: idsToMove.length > 1 ? `已移动 ${idsToMove.length} 个文件` : '文件已移动', type: 'success' })
    } catch { toast({ message: '移动失败', type: 'error' }) }
  }
}
const dragGhostText = computed(() => {
  if (dragType.value === 'folder') return allFolders.value.find(f => f.id === dragFolderId.value)?.name || '目录'
  if (dragType.value === 'file') {
    if (selectedCount.value > 1) return `${selectedCount.value} 个文件`
    return `${fileDisplayName(allFiles.value.find(f => f.id === dragFileId.value) as any)} · Shift+点击可多选`
  }
  return ''
})
onUnmounted(() => { if (dragTimer.value) clearTimeout(dragTimer.value); document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp); document.removeEventListener('mouseup', onDragUpPre) })

function formatSize(bytes: number|undefined): string {
  if (!bytes || bytes === 0) return '--'
  if (bytes >= 1073741824) return (bytes/1073741824).toFixed(1)+' GB'
  if (bytes >= 1048576) return (bytes/1048576).toFixed(1)+' MB'
  if (bytes >= 1024) return (bytes/1024).toFixed(0)+' KB'
  return bytes+' B'
}


// ─── Preview / Export / Delete ────────────────────────────────────────
async function handlePreview(id: string) { try { const r = await previewFile(id); if (!r?.success) toast({ message: r?.error || '预览失败', type: 'error' }) } catch (e: any) { toast({ message: '预览失败', type: 'error' }) } }
async function handleDownload(id: string) { try { const r = await exportFile(id); if (!r?.success) toast({ message: r?.error || '下载失败', type: 'error' }) } catch (e: any) { toast({ message: '下载失败', type: 'error' }) } }
async function handleCopyFile(id: string) {
  try {
    const r = await copyFileToClipboard(id)
    if (r?.success) toast({ message: `已复制「${r.fileName || '文件'}」，可粘贴到文件夹或聊天软件`, type: 'success' })
    else toast({ message: r?.error || '复制失败', type: 'error' })
  } catch (e: any) { toast({ message: '复制失败: ' + (e?.message || e || ''), type: 'error' }) }
}
async function handleDelete(id: string) {
  const ok = await dangerConfirm({ title: '删除文件', message: '确定从资料库中删除此文件？', confirmText: '删除' })
  if (!ok) return
  try { await deleteFileFromDomain(id, 'library', ''); await loadFiles(); await loadFolders(); toast({ message: '文件已删除', type: 'success' }) } catch (e: any) { toast({ message: '删除失败: ' + (e.message || e), type: 'error' }) }
}

</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm" @click="filterDropdownOpen = false">

  <!-- Sidebar: 目录树 (参考 ProductCenter) -->
  <aside class="flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden relative" :style="{width:sidebarWidth+'px'}">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>
    <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">目录</span>
      <button @click="startAddFolder()" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <div class="flex-1" style="overflow-y:auto;overflow-x:clip">
      <div class="px-2 py-1">
        <!-- 全部文件 -->
        <div>
          <button @click="selectFolder(null)" :data-drop-target="'__root__'" class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :style="{ paddingLeft: `${12}px` }" :class="[selectedFolderId===null?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragTargetId==='__root__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']">
            <div v-if="selectedFolderId===null" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
            <AppIcon name="grid" size="13" class="flex-shrink-0"/>全部文件
          </button>
        </div>

        <!-- 目录树节点 -->
        <template v-for="d in folderTree" :key="d.id">
          <!-- Insertion line: above -->
          <div v-if="isDragging && dragType==='folder' && dragInsert?.targetId===d.id && dragInsert?.side==='above'" class="insertion-line" :style="{ marginLeft: `${4 + (d._depth||0) * 16}px` }"></div>
          <!-- 编辑中的目录名 -->
          <div v-if="editingFolderId===d.id" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" :style="{ paddingLeft: `${12 + (d._depth||0) * 12}px` }">
            <span v-if="d._hasChildren" @click.stop="toggleFolder(d.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedFolders.has(d.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span>
            <span v-else class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
            <input data-folder-edit v-model="folderEditName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text" @keyup.enter="confirmRename(d.id)" @blur="confirmRename(d.id)"/>
          </div>

          <!-- 正常目录节点 -->
          <div v-else :data-drop-target="d.id" class="flex items-center gap-1.5 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg group relative overflow-hidden select-none" :class="[isDragging ? 'cursor-grabbing' : 'cursor-pointer', selectedFolderId===d.id?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragFolderId===d.id?'opacity-30':'', isDragging && dragTargetId===d.id && dragFolderId!==d.id ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']" @click="selectFolder(d.id)" @mousedown="startFolderDrag($event, d.id)" :style="{ paddingLeft: `${12 + (d._depth||0) * 12}px` }">
            <div v-if="dragType==='folder' && dragInsert?.targetId===d.id && dragInsert?.side==='inside'" class="absolute inset-0 border-2 border-brand-400 rounded-lg pointer-events-none"></div>
            <div v-if="selectedFolderId===d.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
            <span v-if="d._hasChildren" @click.stop="toggleFolder(d.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedFolders.has(d.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span>
            <span v-else class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
            <span class="truncate flex-1">{{ d.name }}</span>
            <span @click.stop="startAddSubFolder(d.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="plus" size="10"/></span>
            <span @mousedown.stop @click.stop="startRename(d.id, d.name)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="edit" size="10"/></span>
            <span @click.stop="handleDeleteFolder(d.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="10"/></span>
          </div>

          <!-- 在该目录下新增子目录的输入 -->
          <div v-if="addingNew && addingParentId === d.id" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" :style="{ paddingLeft: `${12 + ((d._depth||0) + 1) * 16}px` }">
            <span class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
            <input data-folder-add v-model="newFolderName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text" @keyup.enter="confirmAddFolder" @keyup.escape="cancelAddFolder" @blur="confirmAddFolder"/>
          </div>
          <!-- Insertion line: below -->
          <div v-if="isDragging && dragType==='folder' && dragInsert?.targetId===d.id && dragInsert?.side==='below'" class="insertion-line" :style="{ marginLeft: `${4 + (d._depth||0) * 16}px` }"></div>
        </template>

        <!-- 根级新增目录输入 -->
        <div v-if="addingNew && !addingParentId" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" style="padding-left:16px">
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
          <input data-folder-add v-model="newFolderName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none" @keyup.enter="confirmAddFolder" @keyup.escape="cancelAddFolder" @blur="confirmAddFolder"/>
        </div>

        <!-- 未分类 -->
        <div class="mt-2 pt-1.5 border-t border-surface-100">
          <button @click="selectFolder('__uncategorized__')" :data-drop-target="'__uncategorized__'" class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :style="{ paddingLeft: `${12}px` }" :class="[selectedFolderId==='__uncategorized__'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragTargetId==='__uncategorized__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']">
            <div v-if="selectedFolderId==='__uncategorized__'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
            <span class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>未分类
          </button>
        </div>
      </div>
    </div>
  </aside>

  <div class="w-px cursor-col-resize flex-shrink-0 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>

  <!-- File list -->
  <div class="flex-1 flex flex-col overflow-hidden">
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">文件</span>
      <div class="flex items-center gap-1.5">
        <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
          <AppIcon name="search" size="12" class="text-surface-400"/>
          <input v-model="folderSearch" type="text" class="w-[120px] bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索文件名..."/>
        </div>
        <div class="relative">
          <button
            @mousedown.prevent @click.stop="toggleFilterDropdown"
            class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors duration-200 text-xs"
            :class="fileTypeFilter !== 'all' ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'"
          >
            <span>{{ fileTypeFilter === 'all' ? '全部' : (filterCategoryLabels[fileTypeFilter] || fileTypeFilter) }}</span>
            <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
          </button>
          <div v-if="filterDropdownOpen" class="absolute right-0 top-full mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[100px]" @click.stop>
            <button @click="selectFilterType('all')" class="w-full px-3 py-1.5 text-xs text-left transition-colors hover:bg-surface-50" :class="fileTypeFilter==='all' ? 'bg-blue-50 text-blue-600 font-medium' : 'text-surface-600'">全部</button>
            <template v-for="cat in ['word','excel','ppt','pdf','image','text','archive','other']" :key="cat">
              <button @click="selectFilterType(cat)" class="w-full px-3 py-1.5 text-xs text-left transition-colors hover:bg-surface-50" :class="fileTypeFilter===cat ? 'bg-blue-50 text-blue-600 font-medium' : 'text-surface-600'">{{ filterCategoryLabels[cat] }}</button>
            </template>
          </div>
        </div>
        <button @click="handleUpload" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"><AppIcon name="plus" size="10" stroke="2.5"/>上传文件</button>
      </div>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1">
      <div v-for="f in filteredFiles" :key="f.id"
        class="flex items-center gap-3 px-3 py-2 rounded-lg transition-colors group hover:bg-surface-50 border border-transparent hover:border-surface-200"
        :class="[isSelected(f.id) ? 'bg-blue-50/70 border-blue-200' : '', isDragging && dragFileId===f.id ? 'select-none' : '']"
        @mousedown="onFileMouseDown($event, f.id)"
      >
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 text-[9px] font-bold tracking-wider" :class="fileTypeColor(f.file_ext)">{{ (f.file_ext || '').toUpperCase() || 'FILE' }}</div>
        <div class="flex-1 min-w-0">
          <template v-if="editingFileId === f.id">
            <input
              v-model="editingFileName"
              data-lib-rename-input
              class="w-full text-[12px] font-medium text-surface-800 bg-transparent outline-none border-b border-brand-400 pb-0.5 select-text"
              @keyup.enter="confirmFileRename(f)"
              @blur="confirmFileRename(f)"
              @click.stop
              @mousedown.stop
              @keyup.escape="cancelFileRename()"
            />
          </template>
          <template v-else>
            <span class="text-[12px] text-surface-800 truncate inline-block max-w-full hover:text-blue-600 transition-colors cursor-pointer" v-tooltip.auto="fileDisplayName(f)" @click="onFileCardClick($event, f.id)">{{ fileDisplayName(f) }}</span>
          </template>
          <div v-if="editingRemarkFileId === f.id" class="mt-1">
            <input v-model="editingRemarkValue" data-lib-remark-input class="w-full text-[11px] text-surface-600 bg-transparent outline-none border-b border-brand-400 pb-0.5 select-text" placeholder="输入备注..." @keyup.enter="saveRemark(f)" @blur="saveRemark(f)" @mousedown.stop @keyup.escape="cancelEditRemark()"/>
          </div>
          <div v-else class="mt-0.5 flex items-center gap-1">
            <p v-if="f.lib_remark" class="text-[10px] text-surface-400 truncate cursor-pointer hover:text-blue-600" @click="startEditRemark(f)">{{ f.lib_remark }}</p>
            <p v-else class="text-[10px] text-surface-300 cursor-pointer hover:text-blue-600" @click="startEditRemark(f)">无备注</p>
            <button @mousedown.stop @click.stop="startEditRemark(f)" class="shrink-0 w-4 h-4 flex items-center justify-center rounded text-surface-300 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100" v-tooltip="'编辑备注'"><AppIcon name="edit" size="10"/></button>
          </div>
        </div>
        <span class="text-[11px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ formatSize(f.file_size) }}</span>
        <span class="text-[11px] text-surface-400 whitespace-nowrap flex-shrink-0">{{ (f.updated_at||f.created_at||'').slice(0,10) }}</span>
        <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
            <button @mousedown.stop @click.stop="startFileRename(f)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"><AppIcon name="edit" size="13"/></button>
          <button @click.stop="handleDownload(f.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'下载'"><AppIcon name="download" size="13"/></button>
          <button @click.stop="handleCopyFile(f.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" v-tooltip="'复制文件到剪贴板'"><AppIcon name="copy" size="13"/></button>
          <button @click.stop="handleDelete(f.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" v-tooltip="'删除'"><AppIcon name="trash" size="13"/></button>
        </div>
      </div>
      <div v-if="filteredFiles.length === 0" class="text-center py-8"><p class="text-xs text-surface-400">暂无文件</p></div>
    </div>
  </div>
</div>

<!-- 上传弹窗：选择文件 或 Ctrl+V 粘贴 -->
<UploadModal
  ref="uploadModalRef"
  entity-type="library"
  title="上传文件"
  :folder-id="selectedFolderId && selectedFolderId !== '__uncategorized__' ? selectedFolderId : null"
  @uploaded="handleLibraryUploaded"
/>

<!-- Drag ghost -->
<Teleport to="body">
  <div v-if="isDragging" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}">
    {{ dragGhostText }}
  </div>
</Teleport>
</template>

<style>
.insertion-line {
  height: 1.5px;
  background: #3b82f6;
  border-radius: 1px;
  opacity: 0.8;
  margin: 1px 0;
}
</style>
