<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, onActivated, watch, nextTick, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import { useSelectionStore } from '@/stores/selection'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import AppIcon from '@/components/common/AppIcon.vue'
import ProjectDetail from '@/views/ProjectDetail.vue'
import AtMentionSelector from '@/components/common/AtMentionSelector.vue'
import AttachmentPanel from '@/components/notes/AttachmentPanel.vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import { Node } from '@tiptap/core'
import { getTiptapExtensions } from '@/composables/useTiptapConfig'
import {
  notesV2ListAll, notesV2ListDaily, notesV2ListOrphan, notesV2ListTrash, notesV2ListArchive,
  notesV2Create, notesV2Update, notesV2Delete, notesV2Restore, notesV2PermanentDelete,
  fetchNotebooks, createNotebook, updateNotebook, deleteNotebook,
  countNotesByNotebook,
  fetchProjects,
  addFileNoteMapping, deleteFileFromDomain, fetchFilesByNote, uploadFile, previewFile, updateFileMeta,
} from '@/api/db-client'
import { useLinkModals } from '@/composables/useLinkModals'
import { useUploadProgress } from '@/composables/useUploadProgress'
import { sanitizeHtmlSync } from '@/utils/sanitize'

type ViewType = 'daily'|'orphan'|'notebook'|'trash'
interface Note { note_id:string; id:string; title:string; content:string; is_daily:number; notebook_id?:string; deleted_at?:string; updated_at:string; created_at:string; tags?:any[] }
interface Notebook { id:string; name:string; color:string; parentId?:string|null }
interface Project { id:string; name:string }

const route = useRoute()
const selectionStore = useSelectionStore()
const currentView = ref<ViewType>('daily')
const selectedProjectId = ref<string|null>(null)
const selectedNotebookId = ref<string|null>(null)
const selectedNoteId = ref<string|null>(null)
const notes = ref<Note[]>([])
const trashNotes = ref<Note[]>([])
const notebooks = ref<Notebook[]>([])
const projects = ref<Project[]>([])
const editTitle = ref('')
const editContent = ref('')
const lastSavedSnapshot = ref('')
const isDirty = ref(false)
const notebookSearch = ref('')
const searchQuery = ref('')


const editingNotebookId = ref<string|null>(null)
const notebookEditName = ref('')
function focusNotebookEdit() { const el = document.querySelector<HTMLInputElement>('[data-notebook-edit]'); el?.focus(); el?.select() }

// ═══════════════════════════════════════════════════════════════════
// Tiptap Editor
// ═══════════════════════════════════════════════════════════════════
const headingLabel = ref('正文')
const headingOpen = ref(false)
const listOpen = ref(false)
const alignOpen = ref(false)
const alignLabel = ref('左对齐')
const tableMenuOpen = ref(false)
const tableRows = ref(3)
const tableCols = ref(3)

// ─── @ Link Selector ──────────────────────────────────────────────
const atOpen = ref(false)

function openAtSelector() { atOpen.value = true }

function insertProjectLink(p: any) {
  if (!editor.value) return
  editor.value.chain().focus().insertContent({
    type: 'projectMention',
    attrs: { projectId: p.id, name: p.name },
  }).insertContent(' ').run()
  atOpen.value = false
}

function insertFileLink(f: any) {
  if (!editor.value) return
  const fname = f.lib_gn || f.lib_cn || f.proj_gn || f.prod_gn || '未命名'
  const fileId = f.id
  editor.value.chain().focus().insertContent({
    type: 'fileMention',
    attrs: { fileId, name: fname },
  }).insertContent(' ').run()
  if (selectedNoteId.value && fileId) {
    addFileNoteMapping(fileId, selectedNoteId.value, fname).catch(() => {})
  }
  atOpen.value = false
}



const { projectModalId, closeProject } = useLinkModals()

const TimeTag = Node.create({
  name: 'timeTag',
  group: 'inline',
  inline: true,
  content: 'text*',
  parseHTML: () => [{ tag: 'span.time-tag' }],
  renderHTML: () => ['span', { class: 'time-tag' }, 0],
})


const editor = useEditor({
  content: '',
  extensions: getTiptapExtensions({ placeholder: '开始记录...', extraExtensions: [TimeTag] }),
  onUpdate: ({ editor: ed }) => {
    editContent.value = ed.getHTML()
    scheduleSave()
  },
  onSelectionUpdate: ({ editor: ed }) => {
    if (ed.isActive('heading', { level: 1 })) headingLabel.value = '标题 1'
    else if (ed.isActive('heading', { level: 2 })) headingLabel.value = '标题 2'
    else if (ed.isActive('heading', { level: 3 })) headingLabel.value = '标题 3'
    else headingLabel.value = '正文'
    if (ed.isActive({ textAlign: 'right' })) alignLabel.value = '右对齐'
    else if (ed.isActive({ textAlign: 'center' })) alignLabel.value = '居中'
    else alignLabel.value = '左对齐'
  },
  editorProps: {
    attributes: { class: 'tiptap-editor' },
  },
})

// ─── Toolbar commands — each bound to editor.chain().focus() ───────
function execFormat(cmd: string, val?: string) {
  if (!editor.value) return
  const ed = editor.value.chain().focus()
  switch (cmd) {
    case 'bold': ed.toggleBold().run(); break
    case 'italic': ed.toggleItalic().run(); break
    case 'underline': ed.toggleUnderline().run(); break
    case 'strikeThrough': ed.toggleStrike().run(); break
    case 'justifyLeft': ed.setTextAlign('left').run(); break
    case 'justifyCenter': ed.setTextAlign('center').run(); break
    case 'insertHorizontalRule': ed.setHorizontalRule().run(); break
    case 'formatBlock': if (val === 'p') ed.setParagraph().run(); break
    case 'insertTable': ed.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(); break
    case 'addColumnBefore': ed.addColumnBefore().run(); break
    case 'addColumnAfter': ed.addColumnAfter().run(); break
    case 'deleteColumn': ed.deleteColumn().run(); break
    case 'addRowBefore': ed.addRowBefore().run(); break
    case 'addRowAfter': ed.addRowAfter().run(); break
    case 'deleteRow': ed.deleteRow().run(); break
    case 'deleteTable': ed.deleteTable().run(); break
  }
}
function insertHeading(level: number) {
  editor.value?.chain().focus().toggleHeading({ level: level as 1|2|3 }).run()
  headingOpen.value = false
}
function insertList(type: 'ul'|'ol') {
  if (!editor.value) return
  type === 'ul'
    ? editor.value.chain().focus().toggleBulletList().run()
    : editor.value.chain().focus().toggleOrderedList().run()
}
function toggleTaskList() {
  editor.value?.chain().focus().toggleTaskList().run()
  listOpen.value = false
}
function insertBlock(tag: string) {
  if (!editor.value) return
  if (tag === 'pre') editor.value.chain().focus().toggleCodeBlock().run()
  else if (tag === 'blockquote') editor.value.chain().focus().toggleBlockquote().run()
}

function insertTime() {
  if (!editor.value) return
  const now = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  editor.value.chain().focus().insertContent({
    type: 'timeTag',
    content: [{ type: 'text', text: now }],
  }).insertContent(' ').run()
}

function execAlign(dir: 'left'|'center'|'right') {
  editor.value?.chain().focus().setTextAlign(dir).run()
  alignLabel.value = dir === 'right' ? '右对齐' : dir === 'center' ? '居中' : '左对齐'
  alignOpen.value = false
}
function insertTableCustom() {
  const r = Math.max(1, Math.min(10, tableRows.value));
  const c = Math.max(1, Math.min(8, tableCols.value));
  editor.value?.chain().focus().insertTable({ rows: r, cols: c, withHeaderRow: true }).run();
  tableMenuOpen.value = false;
}
function closePopups() { headingOpen.value = false; listOpen.value = false; alignOpen.value = false; tableMenuOpen.value = false }
onMounted(() => document.addEventListener('click', closePopups))
onUnmounted(() => document.removeEventListener('click', closePopups))

// ═══════════════════════════════════════════════════════════════════
// Data loading & CRUD (unchanged below)
// ═══════════════════════════════════════════════════════════════════

const restoreDialogOpen = ref(false)
const restoringNoteId = ref<string|null>(null)
const restoreSearchQuery = ref('')
const selectedRestoreTarget = ref<string|null>(null)
const isRestoring = ref(false)
const selectedTrashNoteId = ref<string|null>(null)
const trashNotePreview = computed(() => trashNotes.value.find(n => n.note_id === selectedTrashNoteId.value) || null)
// Column widths (resizable)
const sidebarWidth = ref(220)
const listWidth = ref(260)
const isResizing = ref<'sidebar'|'list'|null>(null)
const resizeStartX = ref(0)
const resizeStartW = ref(0)

function onResizeStart(side: 'sidebar'|'list', e: MouseEvent|TouchEvent) {
  isResizing.value = side
  resizeStartX.value = (e as MouseEvent).clientX || (e as TouchEvent).touches[0].clientX
  resizeStartW.value = side === 'sidebar' ? sidebarWidth.value : listWidth.value
  document.addEventListener('mousemove', onResizeMove)
  document.addEventListener('mouseup', onResizeEnd)
}
function onResizeMove(e: MouseEvent) {
  if (!isResizing.value) return
  const dx = e.clientX - resizeStartX.value
  const newW = Math.max(140, Math.min(500, resizeStartW.value + dx))
  if (isResizing.value === 'sidebar') sidebarWidth.value = newW
  else listWidth.value = newW
}
function onResizeEnd() {
  isResizing.value = null
  document.removeEventListener('mousemove', onResizeMove)
  document.removeEventListener('mouseup', onResizeEnd)
}
onUnmounted(() => { document.removeEventListener('mousemove', onResizeMove); document.removeEventListener('mouseup', onResizeEnd) })

// ─── Data loading ──────────────────────────────────────────────────
async function loadNotes() {
  try {
    let r:any
    switch (currentView.value) {
      case 'daily': r = await notesV2ListDaily(); break
      case 'notebook': if (selectedNotebookId.value) { r = await notesV2ListAll(); } else { notes.value=[]; return }; break
      case 'orphan': r = await notesV2ListOrphan(); break
      case 'trash': r = await notesV2ListTrash(); trashNotes.value = (r?.notes||[]).map((n:any)=>({...n,id:n.note_id,note_id:n.note_id})); return
      default: r = await notesV2ListAll()
    }
    notes.value = (r?.notes||[]).map((n:any)=>({...n,id:n.note_id,note_id:n.note_id}))
    if (currentView.value==='notebook' && selectedNotebookId.value) notes.value = notes.value.filter((n:any)=>n.notebook_id===selectedNotebookId.value && !n.is_daily)
  } catch { notes.value = [] }
}
async function loadAll() { await Promise.all([loadNotes(), loadNotebooks(), loadProjects()]) }
async function loadNotebooks() { try { const r:any = await fetchNotebooks(); notebooks.value = (r?.notebooks||[]).map((n:any)=>({...n,parentId:n.parent_id||null,sortOrder:n.sort_order||0,id:n.id||n.notebook_id})); const newIds = new Set(notebooks.value.map(n => n.id)); collapsedNotebooks.value = new Set([...collapsedNotebooks.value].filter(id => newIds.has(id))) } catch { notebooks.value = [] } }
async function loadProjects() { try { projects.value = await fetchProjects() } catch { projects.value = [] } }
onMounted(async () => {
  await loadAll()
  await ensureDailyNote()
})
// KeepAlive: 切回笔记 Tab 时仅检查是否需要新建今日闪记
// 列表数据的刷新交给用户主动操作（点击侧边栏切换视图时自然会 loadAll）
onActivated(async () => {
  await loadNotes()
})
// ★ 响应全局搜索跳转
let processingSeq = 0
watch(() => selectionStore.current.seq, async () => {
  if (selectionStore.current.type !== 'note') return
  const id = selectionStore.current.id!
  const mySeq = ++processingSeq
  // 先用 V2ListAll 查所有笔记（不依赖当前视图）
  let r: any = await notesV2ListAll()
  if (mySeq !== processingSeq) return
  let note = (r?.notes || []).find((n: any) => n.note_id === id || n.id === id)
  // 如果不在普通笔记中，可能是每日闪记
  if (!note) {
    r = await notesV2ListDaily()
    if (mySeq !== processingSeq) return
    note = (r?.notes || []).find((n: any) => n.note_id === id || n.id === id)
  }
  if (!note) { toast({ message: '笔记不存在或已删除', type: 'warning' }); selectionStore.clear(); return }
  // 切到对应视图
  if (note.notebook_id) {
    currentView.value = 'notebook'
    selectedNotebookId.value = note.notebook_id
  } else if (note.is_daily) {
    currentView.value = 'daily'
  } else {
    currentView.value = 'notes'
  }
  await loadNotes()
  if (mySeq !== processingSeq) return
  selectNote(id)
  selectionStore.clear()
}, { immediate: true })

// ─── Daily note (闪记) ──────────────────────────────────────────────
let dailyNoteCreated = false
function getLocalDateStr(): string {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`
}
async function ensureDailyNote() {
  if (currentView.value !== 'daily') return
  const today = getLocalDateStr()
  let todayNote = notes.value.find(n => n.title === today)
  if (todayNote) {
    dailyNoteCreated = true
    selectNote(todayNote.note_id)
  } else {
    await createDailyNote()
  }
  // ★ 唯一一处自动删除：清除非今日的空闪记
  for (const n of notes.value) {
    if (n.is_daily && n.title !== today && !(n.content || '').replace(/<[^>]*>/g, '').trim()) {
      try { await notesV2PermanentDelete(n.note_id) } catch (e) { console.error('[Notes] auto-delete daily note failed:', e) }
    }
  }
  notes.value = notes.value.filter(n => !(n.is_daily && n.title !== today && !(n.content || '').replace(/<[^>]*>/g, '').trim()))
}

async function createDailyNote() {
  const today = getLocalDateStr()
  try {
    const r: any = await notesV2Create({ title: today, content: '', isDaily: true })
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: today, content: '', is_daily: 1, notebook_id: null, updated_at: new Date().toISOString(), created_at: new Date().toISOString(), tags: [] }
    notes.value.unshift(newNote)
    selectNote(newNote.note_id)
    dailyNoteCreated = true
    editTitle.value = today
    editContent.value = ''
    isDirty.value = false
    await nextTick()
    editor.value?.commands.setContent('')
    editor.value?.commands.focus()
    toast({ message: '今日闪记已创建', type: 'success' })
  } catch (e: any) {
    toast({ message: '创建失败: ' + (e.message || e), type: 'error' })
  }
}

// ─── Notebook search ────────────────────────────────────────────────
const collapsedNotebooks = ref<Set<string>>(new Set())
const expandingOnClick = ref(false)
function toggleNotebook(nbId: string) {
  if (collapsedNotebooks.value.has(nbId)) collapsedNotebooks.value.delete(nbId)
  else collapsedNotebooks.value.add(nbId)
  collapsedNotebooks.value = new Set(collapsedNotebooks.value)
}

const notebookTree = computed(() => {
  const q = notebookSearch.value.toLowerCase().trim()
  const roots = notebooks.value.filter(n => !n.parentId && (!q || n.name.toLowerCase().includes(q)))
  const children = notebooks.value.filter(n => n.parentId)
  const collapsed = collapsedNotebooks.value
  function hasChildren(parentId: string): boolean {
    return children.some(c => c.parentId === parentId && (!q || c.name.toLowerCase().includes(q)))
  }
  function getChildren(parentId: string, depth: number): any[] {
    if (collapsed.has(parentId)) return []
    return [...children].filter(c => c.parentId === parentId && (!q || c.name.toLowerCase().includes(q)))
      .sort((a: any, b: any) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || (a.name || '').localeCompare(b.name || ''))
      .flatMap(c => [{ ...c, _depth: depth, _hasChildren: hasChildren(c.id) }, ...getChildren(c.id, depth + 1)])
  }
  return [...roots].sort((a: any, b: any) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || (a.name || '').localeCompare(b.name || ''))
    .flatMap(r => [{ ...r, _depth: 0, _hasChildren: hasChildren(r.id) }, ...getChildren(r.id, 1)])
})

// ─── Filtered notes ─────────────────────────────────────────────────
const displayedNotes = computed(() => {
  let result = currentView.value === 'trash' ? trashNotes.value : notes.value
  const q = notebookSearch.value.toLowerCase().trim()
  if (q) result = result.filter(n => n.title.toLowerCase().includes(q) || (n.content||'').toLowerCase().includes(q))
  if (currentView.value === 'daily') result = [...result].sort((a,b) => (b.created_at||'').localeCompare(a.created_at||''))
  else result = [...result].sort((a,b) => (b.updated_at||'').localeCompare(a.updated_at||''))
  return result
})

// ─── View switching + auto-save ─────────────────────────────────────
let saveTimer: ReturnType<typeof setTimeout> | null = null
function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer)
  isDirty.value = true
  saveTimer = setTimeout(() => doSave(), 3000)
}
async function saveBeforeSwitch() {
  if (isDirty.value && selectedNoteId.value) {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = null }
    try { await doSave() } catch (e) { console.error('[Notes] save on unmount failed:', e) }
  }
}
async function doSave() {
  if (editor.value) editContent.value = editor.value.getHTML()
  if (currentView.value === 'daily') {
    if (!dailyNoteCreated) return  // 没有已创建或选中的闪记，不自动保存
  }
  if (!selectedNoteId.value) return
  const snapshot = JSON.stringify({ title: editTitle.value, content: editContent.value })
  if (snapshot === lastSavedSnapshot.value) return  // 内容未变，跳过
  await notesV2Update({ noteId: selectedNoteId.value, title: editTitle.value, content: editContent.value })
  lastSavedSnapshot.value = snapshot
  // ★ Update local note card immediately
  const idx = notes.value.findIndex(n => n.note_id === selectedNoteId.value || n.id === selectedNoteId.value)
  if (idx !== -1) {
    notes.value[idx] = { ...notes.value[idx], title: editTitle.value, content: editContent.value, updated_at: new Date().toISOString() }
  }
  isDirty.value = false
}
function switchView(view: ViewType) {
  saveBeforeSwitch()
  currentView.value = view; selectedNoteId.value = null; selectedNotebookId.value = null
  if (view === 'daily') { loadAll().then(() => ensureDailyNote()) }
  else loadAll()
}
function selectNotebook(nid: string) {
  saveBeforeSwitch()
  selectedNotebookId.value = nid; currentView.value = 'notebook'
  selectedProjectId.value = null; selectedNoteId.value = null
  loadAll()
}
function selectNote(id: string) {
  saveBeforeSwitch().then(() => {
    // Find note in active list (notes or trashNotes)
    const src = currentView.value === 'trash' ? trashNotes.value : notes.value
    const n = src.find(x => x.note_id === id || x.id === id)
    if (!n) return
    selectedNoteId.value = id
    editTitle.value = n.title || ''
    editContent.value = n.content || ''
    isDirty.value = false
    if (currentView.value !== 'trash') {
      nextTick(() => {
        editor.value?.commands.setContent(n.content || '', false)
        editor.value?.commands.focus()
      })
    }
  })
}
function selectTrashNote(id: string) { selectedTrashNoteId.value = id }

// Auto-save on page leave
function onPageLeave() { if (isDirty.value) doSave() }
onMounted(() => window.addEventListener('beforeunload', onPageLeave))
onUnmounted(() => {
  window.removeEventListener('beforeunload', onPageLeave)
  if (isDirty.value) doSave()
})

// ─── Note CRUD ──────────────────────────────────────────────────────
async function createNote() {
  await saveBeforeSwitch()
  const payload: any = { title: '', content: '', isDaily: currentView.value === 'daily' }
  if (currentView.value === 'notebook' && selectedNotebookId.value) payload.notebookId = selectedNotebookId.value
  try {
    const r: any = await notesV2Create(payload)
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: '', content: '', is_daily: payload.isDaily ? 1 : 0, notebook_id: payload.notebookId || null, updated_at: new Date().toISOString(), created_at: new Date().toISOString(), tags: [] }
    notes.value.unshift(newNote)
    selectedNoteId.value = newNote.note_id
    editTitle.value = ''; editContent.value = ''; isDirty.value = false
    await nextTick()
    editor.value?.commands.setContent('')
    editor.value?.commands.focus()
  } catch (e) { console.error('[Notes] create note failed:', e) }
}
async function deleteNote(id: string) {
  const ok = await dangerConfirm({ title: '删除笔记', message: '确定删除此笔记？将放入回收站，30天后自动清空。', confirmText: '删除' })
  if (!ok) return
  try {
    await notesV2Delete(id)
    notes.value = notes.value.filter(n => n.note_id !== id && n.id !== id)
    if (selectedNoteId.value === id) selectedNoteId.value = null
    toast({ message: '已移至回收站', type: 'success' })
  } catch (e) { console.error('[Notes] deleteNote failed:', e) }
}

// ─── Drag & Drop — custom mouse-event based (bypasses WebView HTML5 drag bugs) ──
const isDragging = ref(false)
const dragType = ref<'note'|'notebook'|null>(null)
const dragNoteId = ref<string|null>(null)
const dragNotebookId = ref<string|null>(null)
const dragX = ref(0)
const dragY = ref(0)
const dragStartX = ref(0)
const dragStartY = ref(0)
const dragStarted = ref(false) // true when cursor moved > 5px
const dragTargetId = ref<string|null>(null)
const dragInsert = ref<{targetId: string; side: 'above'|'below'|'inside'}|null>(null)

function formatNoteTime(ts: string): string {
  if (!ts) return ''
  try {
    const d = ts.includes('T') ? new Date(ts) : new Date(ts.replace(' ', 'T'))
    const diff = Date.now() - d.getTime()
    if (isNaN(diff)) return ts.slice(0, 16)
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return '刚刚'
    if (mins < 60) return `${mins} 分钟前`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs} 小时前`
    const days = Math.floor(hrs / 24)
    if (days < 7) return `${days} 天前`
    return ts.slice(0, 16)
  } catch { return ts.slice(0, 16) }
}

const dragTimer = ref<ReturnType<typeof setTimeout>|null>(null)

function startNoteDrag(e: MouseEvent, noteId: string) {
  e.preventDefault()
  dragX.value = dragStartX.value = e.clientX
  dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onDragMove)
  dragTimer.value = setTimeout(() => {
    isDragging.value = true
    dragType.value = 'note'
    dragNoteId.value = noteId
    dragStarted.value = true
  }, 300)
  document.addEventListener('mouseup', onDragUpPre)
}

function startNotebookDrag(e: MouseEvent, notebookId: string) {
  e.preventDefault()
  e.stopPropagation()
  dragX.value = dragStartX.value = e.clientX
  dragY.value = dragStartY.value = e.clientY
  document.addEventListener('mousemove', onDragMove)
  dragTimer.value = setTimeout(() => {
    isDragging.value = true
    dragType.value = 'notebook'
    dragNotebookId.value = notebookId
    dragStarted.value = true
  }, 300)
  document.addEventListener('mouseup', onDragUpPre)
}

// Before the timer fires, mouseup cancels the drag (treat as click)
function onDragUpPre(_e: MouseEvent) {
  document.removeEventListener('mouseup', onDragUpPre)
  if (dragTimer.value) {
    clearTimeout(dragTimer.value)
    dragTimer.value = null
  }
  // If timer already fired, let onDragUp handle it
  if (isDragging.value) onDragUp(_e)
}

function onDragMove(e: MouseEvent) {
  dragX.value = e.clientX
  dragY.value = e.clientY
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const target = el?.closest('[data-drop-target]') as HTMLElement | null
  const tid = target?.dataset.dropTarget || null
  dragTargetId.value = tid

  if (tid && dragType.value === 'notebook') {
    const rect = target!.getBoundingClientRect()
    const relY = e.clientY - rect.top
    const third = rect.height / 3
    if (relY < third) {
      dragInsert.value = { targetId: tid, side: 'above' }
    } else if (relY > rect.height - third) {
      dragInsert.value = { targetId: tid, side: 'below' }
    } else {
      dragInsert.value = { targetId: tid, side: 'inside' }
    }
  } else {
    dragInsert.value = null
  }
}

async function onDragUp(_e: MouseEvent) {
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragUp)
  const type = dragType.value
  const targetId = dragTargetId.value
  const insert = dragInsert.value
  const noteId = dragNoteId.value
  const notebookId = dragNotebookId.value
  const moved = dragStarted.value
  if (dragTimer.value) { clearTimeout(dragTimer.value); dragTimer.value = null }
  isDragging.value = false
  dragType.value = null
  dragNoteId.value = null
  dragNotebookId.value = null
  dragTargetId.value = null
  dragInsert.value = null
  dragStarted.value = false

  // Not a drag — just a click, don't interfere with click handlers
  if (!moved) return
  if (!type || !targetId) return

  if (type === 'note') {
    if (!noteId || noteId === targetId) return
    try {
      await notesV2Update({ noteId, notebookId: targetId })
      notes.value = notes.value.filter(n => n.note_id !== noteId && n.id !== noteId)
      toast({ message: '笔记已移动', type: 'success' })
    } catch { toast({ message: '移动失败', type: 'error' }) }
  } else {
    if (!notebookId || notebookId === targetId) return

    if (insert?.side === 'inside') {
      try {
        await updateNotebook({ id: notebookId, parentId: targetId, sortOrder: 99 })
        await loadNotebooks()
        toast({ message: '已移入笔记本', type: 'success' })
      } catch { toast({ message: '移动失败', type: 'error' }) }
    } else {
      const list = [...notebooks.value]
      const srcIdx = list.findIndex(n => n.id === notebookId)
      const dstIdx = list.findIndex(n => n.id === targetId)
      if (srcIdx === -1 || dstIdx === -1) return
      const srcNb = list[srcIdx]
      const dstNb = list[dstIdx]
      if (srcNb.parentId !== dstNb.parentId) {
        try {
          await updateNotebook({ id: notebookId, parentId: dstNb.parentId || null, sortOrder: dstNb.sortOrder ?? dstIdx })
          await loadNotebooks()
          toast({ message: '笔记本已移至此处', type: 'success' })
        } catch { toast({ message: '移动失败', type: 'error' }) }
        return
      }
      list.splice(srcIdx, 1)
      const newPos = insert?.side === 'above' ? dstIdx : dstIdx + 1
      list.splice(newPos, 0, srcNb)
      notebooks.value = list
      const siblings = list.filter(n => n.parentId === srcNb.parentId)
      for (let i = 0; i < siblings.length; i++) {
        updateNotebook({ id: siblings[i].id, sortOrder: i }).catch(() => {})
      }
      toast({ message: '笔记本顺序已调整', type: 'success' })
    }
  }
}

onUnmounted(() => {
  if (dragTimer.value) clearTimeout(dragTimer.value)
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragUp)
  document.removeEventListener('mouseup', onDragUpPre)
})

// ─── Note attachments (driven by AttachmentPanel) ────────────────
const noteFiles = ref<any[]>([])

function noteFileName(f: any): string { return f.lib_gn || f.lib_cn || f.custom_name || f.global_name || f.mapping_name || '未命名' }

async function loadNoteFiles() {
  if (!selectedNoteId.value) { noteFiles.value = []; return }
  try { const r: any = await fetchFilesByNote(selectedNoteId.value); noteFiles.value = Array.isArray(r) ? r : (r?.files || []) } catch { noteFiles.value = [] }
}
watch(selectedNoteId, loadNoteFiles)

const { start: upStart, done: upDone, fail: upFail } = useUploadProgress()

async function handleAttachPanel({ path, name }: { path: string; name?: string }) {
  if (!selectedNoteId.value) return
  const fileName = name || path.replace(/^.*[\\\/]/, '')
  const ext = fileName.includes('.') ? fileName.split('.').pop() || '' : ''
  const task = upStart(fileName)
  try {
    const r: any = await uploadFile({ filePath: path, fileName, fileExt: ext, customName: fileName, entityType: 'note', entityId: selectedNoteId.value })
    const fileId = r?.fileId || r?.id
    if (fileId) { await loadNoteFiles() }
    upDone(task)
  } catch (e: any) { upFail(task, e.message || '上传失败') }
}

function insertNoteFileToEditor(f: any) {
  if (!editor.value) return
  const name = noteFileName(f)
  editor.value.chain().focus().insertContent({ type: 'fileMention', attrs: { fileId: f.id, name } }).insertContent(' ').run()
}

async function renameNoteFile(file: any, name: string) {
  try { await updateFileMeta(file.id, { customName: name, relationId: file.relation_id }); await loadNoteFiles() } catch (e) { console.error('[Notes] renameNoteFile failed:', e) }
}
async function removeNoteFile(fileId: string) {
  if (!selectedNoteId.value) return
  const ok = await dangerConfirm({ title: '删除附件', message: '确定删除此附件？', confirmText: '删除' })
  if (!ok) return
  try {
    await deleteFileFromDomain(fileId, 'note', selectedNoteId.value)
    noteFiles.value = noteFiles.value.filter(f => f.id !== fileId)
    toast({ message: '附件已移除', type: 'success' })
  } catch (e: any) { toast({ message: '移除失败: ' + (e.message || e), type: 'error' }) }
}

// ─── Notebook CRUD ──────────────────────────────────────────────────
async function startAddNotebook(parentId?: string) {
  let name = '新笔记本'
  let i = 1
  while (isDuplicateNotebookName(name, undefined, parentId || null)) { name = '新笔记本 ' + (++i) }
  const payload: any = { name }
  if (parentId) payload.parentId = parentId
  const r: any = await createNotebook(payload)
  await loadNotebooks()
  if (parentId) { collapsedNotebooks.value.delete(parentId); collapsedNotebooks.value = new Set(collapsedNotebooks.value) }
  editingNotebookId.value = r?.notebook?.id || r?.id
  notebookEditName.value = name
  await nextTick()
  setTimeout(() => { focusNotebookEdit() }, 50)
}
function startEditNotebook(nb: Notebook) { editingNotebookId.value = nb.id; notebookEditName.value = nb.name; nextTick(() => setTimeout(() => { focusNotebookEdit() }, 50)) }
function isDuplicateNotebookName(name: string, excludeId?: string, parentId?: string|null): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return notebooks.value.some((n: any) => n.id !== excludeId && n.name.toLowerCase() === q && (n.parentId || null) === (parentId || null))
}
async function saveNotebookName(nid: string) {
  const name = notebookEditName.value.trim()
  if (!name) return
  const nb = notebooks.value.find((n: any) => n.id === nid)
  if (isDuplicateNotebookName(name, nid, nb?.parentId || null)) { toast({ message: '同级目录下已存在同名笔记本', type: 'error' }); editingNotebookId.value = null; return }
  try { await updateNotebook({ id: nid, name }); editingNotebookId.value = null; await loadNotebooks() } catch { editingNotebookId.value = null }
}
async function removeNotebook(nid: string) {
  const nb = notebooks.value.find(n => n.id === nid)
  const countData: any = await countNotesByNotebook(nid)
  const count = countData?.count || 0
  const ok = await dangerConfirm({ title: '删除笔记本', message: `确定删除「${nb?.name || '未命名'}」？其中 ${count} 篇笔记将移入回收站。`, confirmText: '删除' })
  if (!ok) return
  try { await deleteNotebook(nid); notebooks.value = notebooks.value.filter(n => n.id !== nid); if (selectedNotebookId.value === nid) { selectedNotebookId.value = null; currentView.value = 'daily' } } catch (e) { console.error('[Notes] deleteNotebook failed:', e) }
}

// ─── Trash / Archive ─────────────────────────────────────────────────
async function openRestoreDialog(nid: string) { restoringNoteId.value = nid; restoreSearchQuery.value = ''; selectedRestoreTarget.value = null; restoreDialogOpen.value = true }
async function confirmRestore() { if (!selectedRestoreTarget.value || !restoringNoteId.value) return; isRestoring.value = true; try { await notesV2Restore(restoringNoteId.value, selectedRestoreTarget.value); restoreDialogOpen.value = false; if (selectedTrashNoteId.value === restoringNoteId.value) selectedTrashNoteId.value = null; restoringNoteId.value = null; await loadAll(); toast({ message: '笔记已恢复', type: 'success' }) } catch (e: any) { toast({ message: '恢复失败: ' + (e.message || e), type: 'error' }) } finally { isRestoring.value = false } }
async function permanentDelete(nid: string) { const ok = await dangerConfirm({ title: '永久删除', message: '此操作不可恢复，确定永久删除？', confirmText: '永久删除' }); if (!ok) return; try { await notesV2PermanentDelete(nid); trashNotes.value = trashNotes.value.filter(n => n.note_id !== nid); if (selectedTrashNoteId.value === nid) selectedTrashNoteId.value = null } catch (e) { console.error('[Notes] permanentDelete failed:', e) } }

const restoreNotebookList = computed(() => {
  const q = restoreSearchQuery.value.toLowerCase()
  return notebooks.value.filter(n => q ? n.name.toLowerCase().includes(q) : true)
})
</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm">
  <!-- Sidebar: Notebook list -->
  <aside class="flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden relative" :style="{width:sidebarWidth+'px'}">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>
    <!-- Title bar -->
    <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">笔记本</span>
      <button @click="startAddNotebook" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建笔记本">
        <AppIcon name="plus" size="12" stroke="2.5"/>
      </button>
    </div>
    <!-- Search -->
    <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
      <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
        <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
        <input v-model="notebookSearch" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索笔记本..."/>
      </div>
    </div>

    <!-- Scrollable list: 闪记 → 用户笔记本 → 项目笔记本 → 归档 → 回收站 -->
    <div class="flex-1 px-2 py-1" style="overflow-y:auto;overflow-x:clip">
      <!-- 闪记 -->
      <button @click="switchView('daily')" class="flex items-center gap-2 w-full pl-[22px] pr-2 py-1.5 text-xs text-left transition-colors rounded-lg mb-0.5 relative overflow-hidden" :class="currentView==='daily'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
        <div v-if="currentView==='daily'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <AppIcon name="bolt" size="14" class="text-surface-400" :class="currentView==='daily'?'text-blue-500':''"/>闪记
      </button>

      <!-- 分隔：闪记 / 用户笔记本 -->
      <div class="mb-2 border-t border-surface-100"></div>

      <!-- User notebooks -->
      <template v-for="(nb, nbi) in notebookTree" :key="nb.id">
        <!-- Insertion line: above target -->
        <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='above'" class="insertion-line" :style="{ marginLeft: `${4 + (nb._depth || 0) * 16}px` }"></div>
        <div v-if="editingNotebookId===nb.id" class="flex items-center gap-1 px-2 py-1" :style="{ paddingLeft: `${12 + (nb._depth || 0) * 16 + 20}px` }">
          <input ref="notebookNameRef" v-model="notebookEditName" class="flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none focus:ring-1 focus:ring-brand-400" @keyup.enter="saveNotebookName(nb.id)" @blur="saveNotebookName(nb.id)"/>
        </div>
        <div v-else :data-drop-target="nb.id" class="flex items-center gap-1 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg group cursor-pointer relative overflow-hidden select-none" :class="[(currentView==='notebook'&&selectedNotebookId===nb.id)?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragNotebookId===nb.id?'opacity-30':'']" @click="selectNotebook(nb.id)" @mousedown="startNotebookDrag($event, nb.id)" :style="{ paddingLeft: `${4 + (nb._depth || 0) * 16}px` }">
  <div v-if="currentView==='notebook'&&selectedNotebookId===nb.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r pointer-events-none"></div>
          <!-- Insertion line: inside target -->
          <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='inside'" class="absolute inset-0 border-2 border-brand-400 rounded-lg pointer-events-none"></div>
          <!-- Toggle arrow for parents -->
          <span v-if="nb._hasChildren" @click.stop="toggleNotebook(nb.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0 mr-0.5">
            <svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedNotebooks.has(nb.id) ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
          </span>
          <span v-else class="w-3 flex-shrink-0 mr-0.5"></span>
          <AppIcon name="folder-open" size="13" class="flex-shrink-0 text-surface-400"/>
          <span class="truncate flex-1">{{ nb.name }}</span>
          <span @click.stop="startAddNotebook(nb.id)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100" title="新建子笔记本"><AppIcon name="plus" size="10"/></span>
          <span @click.stop="startEditNotebook(nb)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="edit" size="10"/></span>
          <span @click.stop="removeNotebook(nb.id)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="10"/></span>
        </div>
        <!-- Insertion line: below target -->
        <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='below'" class="insertion-line" :style="{ marginLeft: `${4 + (nb._depth || 0) * 16}px` }"></div>
      </template>

      <!-- 分隔：回收站 -->
      <div class="mt-2 pt-1.5 border-t border-surface-100">
        <button @click="switchView('trash')" class="flex items-center gap-2 w-full pl-[22px] pr-2 py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :class="currentView==='trash'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
          <div v-if="currentView==='trash'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <AppIcon name="trash" size="13" class="text-surface-400"/>回收站
        </button>
      </div>
    </div>
  </aside>

  <!-- Note List -->
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{width:listWidth+'px'}" v-if="currentView!=='trash'">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('list', $event)"></div>
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">{{ currentView==='daily' ? '闪记' : '笔记' }}</span>
      <button v-if="currentView!=='daily'" @click="createNote" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1.5">
      <div v-for="note in displayedNotes" :key="note.note_id" @mousedown="startNoteDrag($event, note.note_id||note.id)" class="flex items-center px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden hover:scale-[1.02] hover:z-10 select-none" :class="[selectedNoteId===(note.note_id||note.id)?'bg-blue-50/70 border-brand-200 shadow-none scale-100':'border-surface-100 hover:border-surface-300 hover:shadow-md', isDragging && dragNoteId===(note.note_id||note.id)?'opacity-50':'']" @click="selectNote(note.note_id||note.id)">
        <div v-if="selectedNoteId===(note.note_id||note.id)" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <div class="flex-1 min-w-0">
          <p class="text-[12px] font-medium text-surface-800 truncate">{{ currentView==='daily' ? (note.title||(note.created_at||'').slice(0,10)) : (note.title||'未命名笔记') }}</p>
          <p class="text-[10px] text-surface-400 line-clamp-1 mt-0.5">{{ (note.content||'').replace(/<[^>]*>/g,'').slice(0,40) || '暂无内容' }}</p>
          <p class="text-[9px] text-surface-300 mt-0.5">{{ formatNoteTime(note.updated_at || note.created_at || '') }}</p>
        </div>
        <button @click.stop="deleteNote(note.note_id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0"><AppIcon name="trash" size="13"/></button>
      </div>
    </div>
  </div>

  <!-- Trash list -->
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{width:listWidth+'px'}" v-if="currentView==='trash'">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('list', $event)"></div>
    <div class="flex items-center px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">回收站</span>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1.5">
      <div v-for="note in displayedNotes" :key="note.note_id" class="flex items-center px-3 h-14 rounded-lg cursor-pointer transition-all duration-150 group border bg-white shadow-sm" :class="selectedNoteId===(note.note_id||note.id)?'bg-amber-50 border-amber-300':'border-surface-100 hover:border-surface-300 hover:shadow-md'" @click="selectNote(note.note_id)">
        <div class="flex-1 min-w-0">
          <p class="text-[13px] font-medium text-surface-800 truncate">{{ note.title||'未命名' }}</p>
          <p class="text-[11px] text-surface-400 mt-0.5">删除于 {{ note.deleted_at?.slice(0,10)||'-' }}</p>
        </div>
        <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0 ml-2">
          <button @click.stop="openRestoreDialog(note.note_id)" class="w-7 h-7 flex items-center justify-center rounded text-surface-500 hover:text-blue-600 hover:bg-blue-50 transition-colors" title="恢复"><AppIcon name="undo" size="14"/></button>
          <button @click.stop="permanentDelete(note.note_id)" class="w-7 h-7 flex items-center justify-center rounded text-surface-500 hover:text-red-600 hover:bg-red-50 transition-colors" title="永久删除"><AppIcon name="trash" size="14"/></button>
        </div>
      </div>
    </div>
  </div>

  <!-- Trash readonly preview -->
  <div class="flex-1 flex flex-col min-w-0 overflow-hidden" v-if="currentView==='trash'">
    <template v-if="selectedNoteId">
      <div class="flex items-center px-5 h-10 border-b border-surface-100 bg-amber-50/50 shrink-0">
        <p class="text-xs text-amber-700">此笔记在回收站中，30天后自动永久删除。</p>
      </div>
      <div class="px-6 pt-4 pb-1"><h1 class="text-lg font-semibold text-surface-800">{{ editTitle||'未命名笔记' }}</h1></div>
      <div class="flex-1 overflow-y-auto px-6 pb-8"><div class="text-sm text-surface-600 leading-relaxed" v-html="sanitizeHtmlSync(editContent)"></div></div>
    </template>
    <div v-else class="flex-1 flex items-center justify-center text-surface-400 text-sm">选择左侧笔记查看内容</div>
  </div>

  <!-- Editor -->
  <div class="flex-1 flex flex-col min-w-0 overflow-hidden relative" v-if="currentView!=='trash'">
    <template v-if="selectedNoteId">
      <div class="flex items-center gap-0 px-2 h-10 border-b border-surface-100 shrink-0">
        <!-- Group 1: Heading Dropdown -->
        <div class="relative">
          <button @mousedown.prevent @click.stop="headingOpen=!headingOpen; listOpen=false" class="flex items-center gap-0.5 w-[54px] h-7 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
            <span>{{ headingLabel }}</span>
            <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
          </button>
          <div v-if="headingOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[90px]">
            <button @mousedown.prevent="execFormat('formatBlock','p'); headingLabel='正文'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50">正文</button>
            <button @mousedown.prevent="insertHeading(1)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 1</button>
            <button @mousedown.prevent="insertHeading(2)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 2</button>
            <button @mousedown.prevent="insertHeading(3)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 3</button>
          </div>
        </div>
        <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <!-- Group 2: Text Styles -->
        <div class="flex items-center gap-0.5">
          <button @mousedown.prevent="execFormat('bold')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('bold')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><b class="text-xs">B</b><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">加粗</span></button>
          <button @mousedown.prevent="execFormat('italic')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('italic')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><i class="text-xs">I</i><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">斜体</span></button>
          <button @mousedown.prevent="execFormat('underline')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('underline')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><u class="text-xs">U</u><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">下划线</span></button>
          <button @mousedown.prevent="execFormat('strikeThrough')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100 line-through relative group" :class="editor?.isActive('strike')?'bg-blue-50 text-blue-600':''"><span>S</span><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">删除线</span></button>
        </div>
        <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <!-- Group 3: List dropdown -->
        <div class="relative">
          <button @mousedown.prevent @click.stop="listOpen=!listOpen; headingOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
            <AppIcon name="list" size="15"/>
            <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
          </button>
          <div v-if="listOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[110px]">
            <button @mousedown.prevent="insertList('ul'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list" size="14"/>无序列表</button>
            <button @mousedown.prevent="insertList('ol'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list-ol" size="14"/>有序列表</button>
          </div>
        </div>
        <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <!-- Task list button -->
        <button @mousedown.prevent="toggleTaskList()" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('taskList')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'" title="任务清单">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><path d="M8 12l3 3 5-6"/></svg>
          <span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">任务清单</span>
        </button>
        <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <!-- Group 4: Alignment dropdown -->
        <div class="relative">
          <button @mousedown.prevent @click.stop="alignOpen=!alignOpen; headingOpen=false; listOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
            <AppIcon :name="alignLabel==='右对齐'?'align-right':alignLabel==='居中'?'align-center':'align-left'" size="15"/>
            <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
          </button>
          <div v-if="alignOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[120px]">
            <button @mousedown.prevent="execAlign('left')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-left" size="14"/>左对齐</button>
            <button @mousedown.prevent="execAlign('center')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-center" size="14"/>居中</button>
            <button @mousedown.prevent="execAlign('right')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-right" size="14"/>右对齐</button>
          </div>
        </div>
        <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <!-- Group 5: Inserts & Table -->
        <div class="flex items-center gap-0.5">
          <button @mousedown.prevent="execFormat('insertHorizontalRule')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"><span class="text-xs">—</span><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">分隔线</span></button>
          <button @mousedown.prevent="insertBlock('pre')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"><AppIcon name="code-bracket" size="15"/><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">代码块</span></button>
          <button @mousedown.prevent="insertBlock('blockquote')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"><AppIcon name="quote" size="15"/><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">引用</span></button>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <button @mousedown.prevent="insertTime()" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group" title="插入时间"><AppIcon name="clock" size="15"/><span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">插入时间</span></button>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <!-- @ mention button -->
          <div class="relative">
            <button
              data-at-btn
              @mousedown.prevent @click.stop="openAtSelector()"
              class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"
              :class="atOpen ? 'bg-blue-50 text-blue-600' : ''"
              title="插入链接"
            >
              <svg class="w-[15px] h-[15px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 12v1a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
              <span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">插入链接</span>
            </button>
          </div>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <!-- Table button with rows/cols inputs -->
          <div class="relative">
            <button @mousedown.prevent @click.stop="tableMenuOpen=!tableMenuOpen; headingOpen=false; listOpen=false; alignOpen=false" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('table')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'">
              <AppIcon name="table-grid" size="15"/>
              <svg class="w-2 h-2 ml-0.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
              <span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">表格</span>
            </button>
            <div v-if="tableMenuOpen" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-lg shadow-sm border border-surface-100 p-1 w-[92px]" @click.stop>
              <div v-if="!editor?.isActive('table')" class="space-y-1.5 mb-2">
                <div class="flex items-center gap-1.5">
                  <span class="text-[11px] text-surface-500 w-4">行</span>
                  <input v-model.number="tableRows" type="number" min="1" max="10" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/>
                </div>
                <div class="flex items-center gap-1.5">
                  <span class="text-[11px] text-surface-500 w-4">列</span>
                  <input v-model.number="tableCols" type="number" min="1" max="8" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/>
                </div>
              </div>
              <button v-if="!editor?.isActive('table')" @click.prevent="insertTableCustom()" class="w-full py-1 text-[10px] bg-brand-600 text-white rounded hover:bg-brand-700 transition-colors flex items-center justify-center gap-1">
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M12 4v16m8-8H4"/></svg>
                插入表格
              </button>
              <template v-if="editor?.isActive('table')">
                <hr class="border-surface-100 my-2" />
                <button @mousedown.prevent="execFormat('addColumnBefore'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">左侧插入列</button>
                <button @mousedown.prevent="execFormat('addColumnAfter'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">右侧插入列</button>
                <button @mousedown.prevent="execFormat('deleteColumn'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">删除列</button>
                <hr class="border-surface-100 my-1" />
                <button @mousedown.prevent="execFormat('addRowBefore'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">上方插入行</button>
                <button @mousedown.prevent="execFormat('addRowAfter'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">下方插入行</button>
                <button @mousedown.prevent="execFormat('deleteRow'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">删除行</button>
                <hr class="border-surface-100 my-1" />
                <button @mousedown.prevent="execFormat('deleteTable'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-red-500 hover:bg-red-50 rounded">删除表格</button>
              </template>
            </div>
          </div>
        </div>
      </div>
      <!-- Title + Editor + floating drawer -->
      <div class="flex-1 flex flex-col min-w-0 relative">
        <div v-if="currentView!=='daily'" class="px-4 pt-3 pb-1 shrink-0">
          <input v-model="editTitle" class="w-full text-lg font-semibold text-surface-800 bg-transparent outline-none placeholder:text-surface-300" placeholder="笔记标题" @input="scheduleSave()" @keydown.enter.prevent="editor?.commands.focus()"/>
          <hr class="border-surface-200 mt-1.5" />
        </div>
        <div v-else class="h-3 shrink-0"></div>
        <!-- 滚动发生在 .tiptap-editor 内部 -->
        <div class="flex-1 min-h-0 relative">
          <div class="absolute inset-0 overflow-y-auto px-[17px] py-2">
            <EditorContent :editor="editor" />
          </div>
        </div>
        <AttachmentPanel
          :key="'ap-' + selectedNoteId"
          :note-id="selectedNoteId"
          :files="noteFiles"
          @upload="(file) => handleAttachPanel(file)"
          @preview="(file) => previewFile(file.id)"
          @insert="(file) => insertNoteFileToEditor(file)"
          @delete="(file) => removeNoteFile(file.id)"
          @rename="(file, name) => renameNoteFile(file, name)"
        />
      </div>
    </template>
    <div v-else class="flex-1 flex items-center justify-center text-surface-400 text-sm">选择或创建一篇笔记</div>
  </div>

</div>


<!-- Drag ghost -->
<Teleport to="body">
  <div v-if="isDragging" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}">
    <template v-if="dragType === 'note'">{{ notes.find(n => (n.note_id||n.id) === dragNoteId)?.title || '笔记' }}</template>
    <template v-else>{{ notebooks.find(n => n.id === dragNotebookId)?.name || '笔记本' }}</template>
  </div>
</Teleport>

<!-- Restore dialog -->
<Teleport to="body"><Transition name="modal-fade"><div v-if="restoreDialogOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" @click.self="restoreDialogOpen=false"><div class="bg-white rounded-2xl w-[420px] max-h-[75vh] overflow-hidden shadow-2xl flex flex-col border border-surface-200">
  <div class="flex items-center justify-between px-5 py-4 border-b border-surface-100">
    <h3 class="text-sm font-semibold text-surface-800">选择目标笔记本</h3>
    <button @click="restoreDialogOpen=false" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600">✕</button>
  </div>
  <div class="px-4 py-2 border-b border-surface-100"><div class="flex items-center gap-2 bg-surface-50 rounded-lg px-3 py-2"><AppIcon name="search" size="13" class="text-surface-400"/><input v-model="restoreSearchQuery" type="text" class="flex-1 bg-transparent text-xs outline-none" placeholder="搜索笔记本..."/></div></div>
  <div class="flex-1 overflow-y-auto p-3 space-y-1">
    <button v-for="nb in restoreNotebookList" :key="nb.id" @click="selectedRestoreTarget=nb.id" class="w-full flex items-center gap-2 px-3 py-2 text-xs rounded-lg transition-colors" :class="selectedRestoreTarget===nb.id?'bg-brand-50 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
      <AppIcon name="folder" size="12"/><span class="truncate">{{nb.name}}</span>
    </button>
  </div>
  <div class="flex justify-end gap-2 px-5 py-4 border-t border-surface-100">
    <button @click="restoreDialogOpen=false" class="btn-secondary text-xs">取消</button>
    <button @click="confirmRestore" :disabled="!selectedRestoreTarget||isRestoring" class="btn-primary text-xs">{{isRestoring?'恢复中':'恢复'}}</button>
  </div>
</div></div></Transition></Teleport>

<!-- Project detail modal -->
<Teleport to="body">
  <Transition name="fade">
    <div v-if="projectModalId" class="fixed inset-0 z-[70] flex items-center justify-center bg-black/30 backdrop-blur-sm" @click.self="closeProject()">
      <div class="bg-white rounded-2xl w-[1100px] shadow-2xl flex flex-col border border-surface-200 overflow-hidden" style="height:88vh" @click.stop>
        <div class="flex items-center justify-between px-5 py-3 border-b border-surface-100 shrink-0">
          <span class="text-sm font-semibold text-surface-800">项目详情</span>
          <button @click="closeProject()" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600">✕</button>
        </div>
        <div class="flex-1 min-h-0">
          <ProjectDetail :id="projectModalId" :embedded="true" />
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

<AtMentionSelector
  v-if="atOpen"
  :projects="projects"
  @select-project="insertProjectLink($event)"
  @select-file="insertFileLink($event)"
  @close="atOpen = false"
/>
</template>

<style>
.no-spinner::-webkit-outer-spin-button,
.no-spinner::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.no-spinner { -moz-appearance: textfield; }
.tiptap-editor {
  padding: 8px 0 16px;
  font-size: 14px;
  line-height: 1.55;
  color: #334155;
  outline: none;
}
.tiptap-editor .ProseMirror { min-height: 200px; }
.tiptap-editor ul[data-type="taskList"] { list-style: none; padding-left: 0; }
.tiptap-editor ul[data-type="taskList"] li { list-style: none; display: flex; gap: 0.5em; align-items: flex-start; }
.tiptap-editor ul[data-type="taskList"] li::marker { content: none; }
.tiptap-editor ul[data-type="taskList"] li label { margin-top: 0.15em; flex-shrink: 0; }
.tiptap-editor ul[data-type="taskList"] li div { flex: 1; min-height: 0; }
.tiptap-editor ul[data-type="taskList"] li div p { margin: 0; }
.tiptap-editor p { margin: 0 0 0.5em; }
.tiptap-editor h1 { font-size: 1.6em; font-weight: 700; margin: 0.5em 0 0.3em; }
.tiptap-editor h2 { font-size: 1.3em; font-weight: 600; margin: 0.4em 0 0.25em; }
.tiptap-editor h3 { font-size: 1.1em; font-weight: 600; margin: 0.3em 0 0.2em; }
.tiptap-editor ul { list-style-type: disc; padding-left: 1.5em; }
.tiptap-editor ol { list-style-type: decimal; padding-left: 1.5em; }
.tiptap-editor ul ul { list-style-type: circle; }
.tiptap-editor ul ul ul { list-style-type: square; }
.tiptap-editor li { margin: 0.2em 0; }
.tiptap-editor li p { margin: 0; }
.tiptap-editor blockquote { border-left: 3px solid #e2e8f0; padding-left: 1em; margin: 0.5em 0; color: #64748b; }
.tiptap-editor pre { background: #f1f5f9; border-radius: 6px; padding: 12px 16px; font-family: ui-monospace, monospace; font-size: 13px; margin: 0.5em 0; overflow-x: auto; }
.tiptap-editor code { background: #f1f5f9; border-radius: 4px; padding: 1px 5px; font-size: 0.9em; }
.tiptap-editor pre code { background: none; padding: 0; }
.tiptap-editor hr { border: none; border-top: 1px solid #e2e8f0; margin: 1em 0; }
.tiptap-editor table { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
.tiptap-editor table td, .tiptap-editor table th { border: 1px solid #e2e8f0; padding: 6px 10px; font-size: 13px; min-width: 60px; }
.tiptap-editor table th { background: #f8fafc; font-weight: 600; }
.tiptap-editor .selectedCell { background: #e0f2fe; }
.tiptap-editor a { color: #3b82f6; text-decoration: underline; }
.insertion-line {
  height: 2px;
  margin: 0px 8px;
  border-radius: 1px;
  background: #3b82f6;
  animation: insertionPulse 0.8s ease-in-out infinite alternate;
  pointer-events: none;
}
@keyframes insertionPulse {
  from { opacity: 0.6; }
  to   { opacity: 1; }
}

.tiptap-editor p.is-editor-empty:first-child::before {
  content: attr(data-placeholder);
  float: left;
  color: #94a3b8;
  pointer-events: none;
  height: 0;
}

/* ─── Flash quick-note blockquotes ─── */
.tiptap-editor blockquote.is-flash {
  background: #f0f9ff;
  border-left: 3px solid #3b82f6;
  border-radius: 6px;
  padding: 10px 14px;
  margin: 12px 0 12px 1.5rem;
}
.tiptap-editor blockquote.is-flash p:first-child {
  font-size: 11px;
  color: #64748b;
  margin-bottom: 4px;
}

.tiptap-editor .time-tag {
  background: #f1f5f9;
  border-radius: 5px;
  padding: 1px 7px;
  font-size: 12px;
  color: #64748b;
  white-space: nowrap;
}

/* ─── Project / File link nodes ─── */
.tiptap-editor .project-link,
.tiptap-editor .file-link {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 13px;
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.15s;
}
.tiptap-editor .project-link {
  background: #eff6ff;
  border: 1px solid #bfdbfe;
  color: #1d4ed8;
}
.tiptap-editor .project-link:hover {
  background: #dbeafe;
}
.tiptap-editor .file-link {
  background: #f0fdf4;
  border: 1px solid #bbf7d0;
  color: #15803d;
}
.tiptap-editor .file-link:hover {
  background: #dcfce7;
}
.tiptap-editor .link-svg {
  flex-shrink: 0;
  display: inline;
}
</style>