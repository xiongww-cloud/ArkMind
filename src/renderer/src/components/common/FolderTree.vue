<script setup lang="ts">
import { ref, computed, onUnmounted, nextTick } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'

interface Folder { id: string; name: string; parent_id?: string|null; project_id?: string|null; sort_order?: number; _depth?: number; _hasChildren?: boolean }

const props = defineProps<{
  folders: Folder[]
  selectedId?: string|null
  projectId?: string|null
  externalDragTargetId?: string|null
}>()

const emit = defineEmits<{
  select: [id: string|null]
  create: [name: string, parentId: string|null]
  rename: [id: string, name: string]
  delete: [id: string]
  move: [id: string, parentId: string|null]
  reorder: [id: string, sortOrder: number]
}>()

// ─── Tree building ──────────────────────────────────────────────
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
    return [...direct].sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || (a.name || '').localeCompare(b.name || '')).flatMap(c => {
      const subs = buildChildren(c.id, depth + 1)
      return [{ ...c, _depth: depth, _hasChildren: subs.length > 0 || hasChildren(c.id) }, ...subs]
    })
  }
  const visible = roots.filter(r => !q || matchSet.has(r.id))
  return [...visible].sort((a, b) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || (a.name || '').localeCompare(b.name || '')).flatMap(r => {
    const subs = buildChildren(r.id, 1)
    return [{ ...r, _depth: 0, _hasChildren: subs.length > 0 || hasChildren(r.id) }, ...subs]
  })
}

const folderTree = computed(() => buildTree(props.folders, collapsedFolders.value, ''))

const effectiveDragTargetId = computed(() => props.externalDragTargetId || dragTargetId.value)

// ─── CRUD state ─────────────────────────────────────────────────
const editingFolderId = ref<string|null>(null)
const folderEditName = ref('')
const addingNew = ref(false)
const addingParentId = ref<string|null>(null)
const newFolderName = ref('')

function isDuplicate(name: string, excludeId?: string, parentId?: string|null): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return props.folders.some(f => f.id !== excludeId && f.name.toLowerCase() === q && (f.parent_id || null) === (parentId || null))
}

function startAddFolder() { let name = '新文件夹'; let i = 1; while (isDuplicate(name, undefined, null)) { name = '新文件夹 ' + (++i) }; addingNew.value = true; addingParentId.value = null; newFolderName.value = name; nextTick(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-add]'); el?.focus(); el?.select() }) }
function startAddSubFolder(parentId: string) { let name = '新文件夹'; let i = 1; while (isDuplicate(name, undefined, parentId)) { name = '新文件夹 ' + (++i) }; addingNew.value = true; addingParentId.value = parentId; newFolderName.value = name; nextTick(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-add]'); el?.focus(); el?.select() }) }
const addingSubmitting = ref(false)
async function confirmAddFolder() {
  if (addingSubmitting.value) return
  const n = newFolderName.value.trim()
  if (!n) { addingNew.value = false; return }
  if (isDuplicate(n, undefined, addingParentId.value || null)) { addingSubmitting.value = false; return }
  addingSubmitting.value = true
  addingNew.value = false; const pid = addingParentId.value; addingParentId.value = null; newFolderName.value = ''
  emit('create', n, pid || null)
  addingSubmitting.value = false
}
function cancelAddFolder() { addingNew.value = false }
function startRename(id: string, name: string) { editingFolderId.value = id; folderEditName.value = name; nextTick(() => { const el = document.querySelector<HTMLInputElement>('[data-folder-edit]'); el?.focus(); el?.select() }) }
async function confirmRename(id: string) {
  const n = folderEditName.value.trim(); if (!n) { editingFolderId.value = null; return }
  const f = props.folders.find(x => x.id === id)
  if (isDuplicate(n, id, f?.parent_id || null)) { editingFolderId.value = null; return }
  emit('rename', id, n)
  editingFolderId.value = null
}
async function handleDeleteFolder(id: string) { emit('delete', id) }

// ─── Drag & drop ─────────────────────────────────────────────────
const isDragging = ref(false)
const dragJustFinished = ref(false)
const dragFolderId = ref<string|null>(null)
const dragX = ref(0); const dragY = ref(0)
const dragStartX = ref(0); const dragStartY = ref(0)
const dragStarted = ref(false)
const dragTimer = ref<ReturnType<typeof setTimeout>|null>(null)
const dragTargetId = ref<string|null>(null)
const dragInsert = ref<{targetId:string;side:'above'|'below'|'inside'}|null>(null)

function startFolderDrag(e: MouseEvent, fid: string) {
  dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onDragMove)
  dragTimer.value = setTimeout(() => { isDragging.value = true; dragFolderId.value = fid; dragStarted.value = true }, 300)
  document.addEventListener('mouseup', onDragUpPre)
}
function onDragUpPre() { document.removeEventListener('mouseup', onDragUpPre); document.removeEventListener('mousemove', onDragMove); if (dragTimer.value) { clearTimeout(dragTimer.value); dragTimer.value = null }; if (isDragging.value) onDragUp() }
function onDragMove(e: MouseEvent) {
  dragX.value = e.clientX; dragY.value = e.clientY
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const t = el?.closest('[data-drop-target]') as HTMLElement|null
  dragTargetId.value = t?.dataset.dropTarget || null
  if (t) {
    const rect = t.getBoundingClientRect()
    const ratio = (e.clientY - rect.top) / rect.height
    const side = ratio < 0.25 ? 'above' : ratio > 0.75 ? 'below' : 'inside'
    dragInsert.value = { targetId: t.dataset.dropTarget || '', side }
  } else { dragInsert.value = null }
}
async function onDragUp() {
  document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp)
  const fid = dragFolderId.value; const tid = dragTargetId.value; const insert = dragInsert.value
  isDragging.value = false; dragFolderId.value = null; dragTargetId.value = null; dragInsert.value = null; dragStarted.value = false
  dragJustFinished.value = true; setTimeout(() => { dragJustFinished.value = false }, 0)
  if (!fid || !tid || fid === tid) return
  if (insert?.side === 'inside') {
    emit('move', fid, tid === '__root__' ? null : tid)
  } else {
    const target = props.folders.find(f => f.id === tid)
    if (!target) return
    const targetOrder = target.sort_order ?? 0
    emit('move', fid, target.parent_id || null)
    emit('reorder', fid, insert?.side === 'above' ? targetOrder - 1 : targetOrder + 1)
  }
}
onUnmounted(() => { if (dragTimer.value) clearTimeout(dragTimer.value); document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp); document.removeEventListener('mouseup', onDragUpPre) })
defineExpose({ startAddFolder })
</script>

<template>
  <div class="flex-1" style="overflow-y:auto;overflow-x:clip">
    <div class="px-2 py-1">
      <!-- 全部文件 -->
      <div>
        <button @click="emit('select', null)" :data-drop-target="'__root__'" class="flex items-center gap-1.5 w-full py-1.5 px-2 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :class="[selectedId===null?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && effectiveDragTargetId==='__root__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']">
          <div v-if="selectedId===null" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <AppIcon name="grid" size="13" class="flex-shrink-0"/>全部文件
        </button>
      </div>

      <!-- 目录树节点 -->
      <template v-for="d in folderTree" :key="d.id">
        <div v-if="isDragging && dragInsert?.targetId===d.id && dragInsert?.side==='above'" class="insertion-line" :style="{ marginLeft: `${4 + (d._depth||0) * 16}px` }"></div>
        <div v-if="editingFolderId===d.id" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" :style="{ paddingLeft: `${12 + (d._depth||0) * 12}px` }">
          <span v-if="d._hasChildren" @click.stop="toggleFolder(d.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedFolders.has(d.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span>
          <span v-else class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
          <input data-folder-edit v-model="folderEditName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none" @keyup.enter="confirmRename(d.id)" @blur="confirmRename(d.id)"/>
        </div>
        <div v-else :data-drop-target="d.id" class="flex items-center gap-1.5 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg group relative overflow-hidden select-none" :class="[isDragging ? 'cursor-grabbing' : 'cursor-pointer', selectedId===d.id?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragFolderId===d.id?'opacity-30':'', isDragging && effectiveDragTargetId===d.id && dragFolderId!==d.id ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']" @click="emit('select', d.id)" @mousedown="startFolderDrag($event, d.id)" :style="{ paddingLeft: `${12 + (d._depth||0) * 12}px` }">
          <div v-if="dragInsert?.targetId===d.id && dragInsert?.side==='inside'" class="absolute inset-0 border-2 border-brand-400 rounded-lg pointer-events-none"></div>
          <div v-if="selectedId===d.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <span v-if="d._hasChildren" @click.stop="toggleFolder(d.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedFolders.has(d.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span>
          <span v-else class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
          <span class="truncate flex-1">{{ d.name }}</span>
          <span @click.stop="startAddSubFolder(d.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="plus" size="10"/></span>
          <span @click.stop="startRename(d.id, d.name)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="edit" size="10"/></span>
          <span @click.stop="handleDeleteFolder(d.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="10"/></span>
        </div>
        <div v-if="addingNew && addingParentId === d.id" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" :style="{ paddingLeft: `${12 + ((d._depth||0) + 1) * 16}px` }">
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
          <input data-folder-add v-model="newFolderName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none" @keyup.enter="confirmAddFolder" @keyup.escape="cancelAddFolder" @blur="confirmAddFolder"/>
        </div>
        <div v-if="isDragging && dragInsert?.targetId===d.id && dragInsert?.side==='below'" class="insertion-line" :style="{ marginLeft: `${4 + (d._depth||0) * 16}px` }"></div>
      </template>

      <!-- 根级新增 -->
      <div v-if="addingNew && !addingParentId" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" style="padding-left:16px">
        <span class="w-3 h-3 flex-shrink-0"></span>
        <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>
        <input data-folder-add v-model="newFolderName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none" @keyup.enter="confirmAddFolder" @keyup.escape="cancelAddFolder" @blur="confirmAddFolder"/>
      </div>

      <!-- 未分类 -->
      <div>
        <button @click="emit('select', '__uncategorized__')" :data-drop-target="'__uncategorized__'" class="flex items-center gap-1.5 w-full py-1.5 px-2 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :style="{ paddingLeft: `${12}px` }" :class="[selectedId==='__uncategorized__'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && effectiveDragTargetId==='__uncategorized__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']">
          <div v-if="selectedId==='__uncategorized__'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/>未分类
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.insertion-line { height: 1.5px; background: #3b82f6; border-radius: 1px; opacity: 0.8; margin: 1px 0; }
</style>
