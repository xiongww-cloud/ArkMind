<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, onActivated, watch, nextTick, onBeforeUnmount, defineAsyncComponent } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useSelectionStore } from '@/stores/selection'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import AppIcon from '@/components/common/AppIcon.vue'
const ProjectDetail = defineAsyncComponent(() => import('@/views/ProjectDetail.vue'))
import AtMentionSelector from '@/components/common/AtMentionSelector.vue'
import AttachmentPanel from '@/components/notes/AttachmentPanel.vue'
import UploadModal from '@/components/shared/UploadModal.vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import RichTextToolbar from '@/components/common/RichTextToolbar.vue'
import { getTiptapExtensions } from '@/composables/useTiptapConfig'
import { richContentToHtml } from '@/utils/rtContent'
import { useResizablePanels } from '@/composables/useResizablePanels'
import { useMultiSelect } from '@/composables/useMultiSelect'
import {
  notesV2ListAll, notesV2ListDaily, notesV2ListTrash,
  notesV2ListByNotebook, notesV2ListByProject, notesV2Get, notesV2GetForTrash,
  notesV2Create, notesV2Update, notesV2Delete, notesV2Restore, notesV2PermanentDelete,
  fetchNotebooks, createNotebook, updateNotebook, deleteNotebook,
  countNotesByNotebook,
  fetchProjects,
  deleteFileFromDomain, fetchFilesByNote, previewFile, updateFileMeta,
} from '@/api/db-client'
import { useLinkModals } from '@/composables/useLinkModals'
import { nowLocalDb, tsMillis } from '@/utils/time'
import { sanitizeHtmlSync } from '@/utils/sanitize'

type ViewType = 'all'|'uncategorized'|'daily'|'notebook'|'projectNote'|'trash'
interface Note { note_id:string; id:string; title:string; excerpt:string; content?:string; is_daily:number; notebook_id?:string; notebook_name?:string; project_ids?:string; deleted_at?:string; updated_at:string; created_at:string; tags?:any[] }
interface Notebook { id:string; name:string; color:string; parentId?:string|null }
interface Project { id:string; name:string }

const route = useRoute()
const router = useRouter()
const selectionStore = useSelectionStore()
const currentView = ref<ViewType>('all')
const selectedProjectId = ref<string|null>(null)
const selectedNotebookId = ref<string|null>(null)
// ★ 项目笔记视图（侧栏两级树：项目笔记→项目名；null=顶层聚合=所有项目笔记）
const selectedProjectNoteId = ref<string|null>(null)
const projectNotesExpanded = ref(false)
const selectedNoteId = ref<string|null>(null)
const notes = ref<Note[]>([])
const trashNotes = ref<Note[]>([])
const notebooks = ref<Notebook[]>([])
const projects = ref<Project[]>([])
const editTitle = ref('')
const editContent = ref('')
const lastSavedSnapshot = ref('')
const isDirty = ref(false)
const noteContentLoading = ref(false)
// ★ 筛选按层级分离：左栏=导航结构（笔记本名+项目名）；中栏顶部=笔记内容（标题/摘要）
const navFilterQuery = ref('')
const noteFilterQuery = ref('')


const editingNotebookId = ref<string|null>(null)
const notebookEditName = ref('')
function focusNotebookEdit() { const el = document.querySelector<HTMLInputElement>('[data-notebook-edit]'); el?.focus(); el?.select() }

// ═══════════════════════════════════════════════════════════════════
// Tiptap Editor
// ═══════════════════════════════════════════════════════════════════

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
  atOpen.value = false
}



const { projectModalId, closeProject } = useLinkModals()

const editor = useEditor({
  content: '',
  extensions: getTiptapExtensions({ placeholder: '开始记录...' }),
  onUpdate: ({ editor: ed }) => {
    editContent.value = ed.getHTML()
    scheduleSave()
  },
  editorProps: {
    attributes: { class: 'tiptap-editor' },
  },
})

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
const { sidebarWidth, listWidth, isResizing, onResizeStart } = useResizablePanels()

// ─── Data loading ──────────────────────────────────────────────────
async function loadNotes() {
  try {
    let r:any
    switch (currentView.value) {
      case 'all': {
        // "全部笔记"=库内常规笔记：闪记与项目笔记都不混入（各有专属视图）
        const rr: any = await notesV2ListAll()
        r = { notes: ((rr?.notes || []) as any[]).filter((n: any) => !n.is_daily && !(n.project_ids && String(n.project_ids).trim())) }
        break
      }
      case 'uncategorized': {
        // "未分类"= 无笔记本归属且无项目关联的虚拟集合（AI 建档不传 notebook 即落入此类，归集入口）
        const rr: any = await notesV2ListAll()
        r = { notes: ((rr?.notes || []) as any[]).filter((n: any) => !n.notebook_id && !(n.project_ids && String(n.project_ids).trim())) }
        break
      }
      case 'daily': r = await notesV2ListDaily(); break
      case 'projectNote': {
        if (selectedProjectNoteId.value) {
          r = await notesV2ListByProject(selectedProjectNoteId.value)
        } else {
          // 顶层聚合：所有带项目关联的笔记（无笔记本归属的每日闪记不含项目关联，天然不混入）
          const rr:any = await notesV2ListAll()
          r = { notes: ((rr?.notes||[]) as any[]).filter(n => !!n.project_ids) }
        }
        break
      }
      case 'notebook': if (selectedNotebookId.value) { r = await notesV2ListByNotebook(selectedNotebookId.value); } else { notes.value=[]; return }; break
      case 'trash': r = await notesV2ListTrash(); trashNotes.value = (r?.notes||[]).map((n:any)=>({...n,id:n.note_id,note_id:n.note_id})); return
      default: notes.value = []; return
    }
    notes.value = (r?.notes||[]).map((n:any)=>({...n,id:n.note_id,note_id:n.note_id}))
  } catch { notes.value = [] }
}
async function loadAll() { await Promise.all([loadNotes(), loadNotebooks(), loadProjects()]) }
async function loadNotebooks() { try { const r:any = await fetchNotebooks(); notebooks.value = (r?.notebooks||[]).map((n:any)=>({...n,parentId:n.parent_id||null,sortOrder:n.sort_order||0,id:n.id||n.notebook_id})); const newIds = new Set(notebooks.value.map(n => n.id)); collapsedNotebooks.value = new Set([...collapsedNotebooks.value].filter(id => newIds.has(id))) } catch { notebooks.value = [] } }
async function loadProjects() { try { projects.value = await fetchProjects() } catch { projects.value = [] } }
onMounted(async () => {
  await loadAll()
  await ensureDailyNote()
  handleRouteQuery()
})
// KeepAlive: 切回笔记 Tab 时仅检查是否需要新建今日闪记
// 列表数据的刷新交给用户主动操作（点击侧边栏切换视图时自然会 loadAll）
onActivated(async () => {
  await loadNotes()
  handleRouteQuery()
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
  // 切到对应视图（"全部"已排除项目笔记：带项目关联的笔记深链进来时落到项目笔记视图）
  if (note.notebook_id) {
    currentView.value = 'notebook'
    selectedNotebookId.value = note.notebook_id
  } else if (note.is_daily) {
    currentView.value = 'daily'
  } else if (note.project_ids && String(note.project_ids).trim()) {
    currentView.value = 'projectNote'
    selectedProjectNoteId.value = null
    projectNotesExpanded.value = true
  } else {
    currentView.value = 'all'
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
  // ★ 唯一处自动删除：清除非今日的空闪记（用 excerpt 判断，避免拉取全文）
  const emptyOldDaily = notes.value.filter(n =>
    n.is_daily && n.title !== today && !(n.excerpt || '').replace(/<[^>]*>/g, '').trim()
  )
  if (emptyOldDaily.length) {
    await Promise.all(emptyOldDaily.map(n => notesV2PermanentDelete(n.note_id).catch(e => console.error('[Notes] auto-delete daily note failed:', e))))
    notes.value = notes.value.filter(n => !emptyOldDaily.some(e => e.note_id === n.note_id))
  }
}

async function createDailyNote() {
  const today = getLocalDateStr()
  try {
    const r: any = await notesV2Create({ title: today, content: '', isDaily: true })
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: today, excerpt: '', is_daily: 1, notebook_id: null, updated_at: nowLocalDb(), created_at: nowLocalDb(), tags: [] }
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
  // 左栏导航筛选：命中笔记本保留其祖先链；筛选期间忽略折叠态（自动展开命中路径）
  const q = navFilterQuery.value.toLowerCase().trim()
  const matchSet = new Set<string>()
  if (q) {
    const byId = new Map(notebooks.value.map(n => [n.id, n]))
    for (const n of notebooks.value) if ((n.name || '').toLowerCase().includes(q)) {
      let cur: string | null | undefined = n.id
      while (cur) { matchSet.add(cur); cur = byId.get(cur)?.parentId || null }
    }
  }
  const keep = (n: any) => !q || matchSet.has(n.id)
  const roots = notebooks.value.filter(n => !n.parentId && keep(n))
  const children = notebooks.value.filter(n => n.parentId && keep(n))
  const collapsed = q ? new Set<string>() : collapsedNotebooks.value
  function hasChildren(parentId: string): boolean {
    return children.some(c => c.parentId === parentId)
  }
  function getChildren(parentId: string, depth: number): any[] {
    if (collapsed.has(parentId)) return []
    return [...children].filter(c => c.parentId === parentId)
      .sort((a: any, b: any) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || (a.name || '').localeCompare(b.name || ''))
      .flatMap(c => [{ ...c, _depth: depth, _hasChildren: hasChildren(c.id) }, ...getChildren(c.id, depth + 1)])
  }
  return [...roots].sort((a: any, b: any) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || (a.name || '').localeCompare(b.name || ''))
    .flatMap(r => [{ ...r, _depth: 0, _hasChildren: hasChildren(r.id) }, ...getChildren(r.id, 1)])
})

// ─── Filtered notes ─────────────────────────────────────────────────
const displayedNotes = computed(() => {
  let result = currentView.value === 'trash' ? trashNotes.value : notes.value
  const q = noteFilterQuery.value.toLowerCase().trim()
  if (q) result = result.filter(n => n.title.toLowerCase().includes(q) || (n.excerpt||'').toLowerCase().includes(q))
  if (currentView.value === 'daily') result = [...result].sort((a,b) => tsMillis(b.created_at) - tsMillis(a.created_at))
  else if (currentView.value === 'trash') result = [...result].sort((a,b) => tsMillis(b.deleted_at || b.updated_at) - tsMillis(a.deleted_at || a.updated_at))
  else result = [...result].sort((a,b) => tsMillis(b.updated_at || b.created_at) - tsMillis(a.updated_at || a.created_at))
  return result
})

const { selectedIds, clear: clearMultiSelect, selectSingle: multiSelectSingle, selectRange: multiSelectRange, isSelected, selectedCount } = useMultiSelect(displayedNotes as any)
const pendingShiftDown = ref(false)

function onNoteMouseDown(e: MouseEvent, nid: string) {
  if ((e.target as HTMLElement).closest('input')) return
  if (currentView.value !== 'notebook') { startNoteDrag(e, nid); return }
  if (e.shiftKey) {
    e.preventDefault()
    multiSelectRange(nid)
    pendingShiftDown.value = true
    return
  }
  pendingShiftDown.value = false
  if (!isSelected(nid)) multiSelectSingle(nid)
  startNoteDrag(e, nid)
}

function onNoteCardClick(e: MouseEvent, nid: string) {
  if (currentView.value === 'notebook' && (e.shiftKey || pendingShiftDown.value)) return
  clearMultiSelect()
  selectNote(nid)
}

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
  try {
    await notesV2Update({ noteId: selectedNoteId.value, title: editTitle.value, content: editContent.value })
  } catch (e: any) {
    // ★ 保存失败必须让用户知道：保留 isDirty，提示重试，绝不静默丢编辑
    console.error('[Notes] doSave failed:', e)
    toast({ message: `笔记保存失败：${e?.message || e}（内容仍在编辑器中，请重试）`, type: 'error' })
    return
  }
  lastSavedSnapshot.value = snapshot
  // ★ Update local note card immediately (excerpt = truncated content)
  // Object.assign 原地修改，避免 spread 创建新对象触发 selectedNote computed 引用变化
  const idx = notes.value.findIndex(n => n.note_id === selectedNoteId.value || n.id === selectedNoteId.value)
  if (idx !== -1) {
    // 统一 SQLite datetime 格式，确保排序按时间正确（后端 now_db 同格式）
    Object.assign(notes.value[idx], { title: editTitle.value, excerpt: editContent.value.replace(/<[^>]*>/g,'').slice(0,300), updated_at: nowLocalDb() })
  }
  isDirty.value = false
}
function switchView(view: ViewType) {
  saveBeforeSwitch()
  currentView.value = view; selectedNoteId.value = null; selectedNotebookId.value = null; selectedProjectNoteId.value = null
  if (view === 'daily') { loadAll().then(() => ensureDailyNote()) }
  else loadAll()
}
// ★ 项目笔记视图：null=顶层聚合（全部项目笔记），pid=该项目的笔记
function selectProjectNote(pid: string|null) {
  saveBeforeSwitch()
  currentView.value = 'projectNote'
  selectedProjectNoteId.value = pid
  projectNotesExpanded.value = true
  selectedNoteId.value = null; selectedNotebookId.value = null
  clearMultiSelect()
  loadAll()
}
/// 项目关联笔记的卡片副标题：project_ids(GROUP_CONCAT) → 项目名
function projectNamesOf(ids?: string): string {
  if (!ids) return ''
  return ids.split(',').map(pid => projects.value.find(p => p.id === pid)?.name || '').filter(Boolean).join('、')
}
// 项目过多时按名称筛选侧栏项目列表（与笔记本共用左栏导航筛选框）
const filteredProjectNotes = computed(() => {
  const q = navFilterQuery.value.toLowerCase().trim()
  return q ? projects.value.filter(p => p.name.toLowerCase().includes(q)) : projects.value
})
const listHeaderTitle = computed(() => {
  if (currentView.value === 'daily') return '闪记'
  if (currentView.value === 'all') return '全部'
  if (currentView.value === 'uncategorized') return '未分类'
  if (currentView.value === 'projectNote') {
    return selectedProjectNoteId.value ? (projects.value.find(p => p.id === selectedProjectNoteId.value)?.name || '项目笔记') : '项目笔记'
  }
  return '笔记'
})
function selectNotebook(nid: string) {
  saveBeforeSwitch()
  selectedNotebookId.value = nid; currentView.value = 'notebook'
  selectedProjectId.value = null; selectedNoteId.value = null
  clearMultiSelect()
  loadAll()
}
let selectNoteSeq = 0
function selectNote(id: string) {
  saveBeforeSwitch().then(async () => {
    const seq = ++selectNoteSeq
    const src = currentView.value === 'trash' ? trashNotes.value : notes.value
    const n = src.find(x => x.note_id === id || x.id === id)
    if (!n) return
    selectedNoteId.value = id
    editTitle.value = n.title || ''
    isDirty.value = false
    // 回收站笔记：从后端获取完整内容
    if (currentView.value === 'trash') {
      noteContentLoading.value = true
      try {
        const r = await notesV2Get(id)
        const fullContent = await richContentToHtml(r?.note?.content || '')
        if (seq !== selectNoteSeq) return // 快速切换时丢弃旧响应
        editContent.value = fullContent
      } catch (e) {
        console.error('[Notes] fetch trash note content failed:', e)
        editContent.value = await richContentToHtml(n.excerpt || '')
      } finally {
        noteContentLoading.value = false
      }
      return
    }
    // 正常笔记：从后端获取完整内容
    noteContentLoading.value = true
    try {
      const r = await notesV2Get(id)
      const fullContent = await richContentToHtml(r?.note?.content || '')
      if (seq !== selectNoteSeq) return // 快速切换时丢弃旧响应
      editContent.value = fullContent
      lastSavedSnapshot.value = JSON.stringify({ title: n.title || '', content: fullContent })
      await nextTick()
      editor.value?.commands.setContent(fullContent, false)
      editor.value?.commands.focus()
    } catch (e) {
      console.error('[Notes] fetch note content failed:', e)
      editContent.value = ''
    } finally {
      noteContentLoading.value = false
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
  if (currentView.value === 'projectNote' && selectedProjectNoteId.value) payload.projectIds = [selectedProjectNoteId.value]
  try {
    const r: any = await notesV2Create(payload)
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: '', excerpt: '', is_daily: payload.isDaily ? 1 : 0, notebook_id: payload.notebookId || null, project_ids: payload.projectIds?.join(',') || '', updated_at: nowLocalDb(), created_at: nowLocalDb(), tags: [] }
    notes.value.unshift(newNote)
    selectedNoteId.value = newNote.note_id
    editTitle.value = ''; editContent.value = ''; isDirty.value = false
    await nextTick()
    editor.value?.commands.setContent('')
    editor.value?.commands.focus()
  } catch (e) { console.error('[Notes] create note failed:', e) }
}

async function createNotePrefilled(title: string, content: string) {
  await saveBeforeSwitch()
  const payload: any = { title: title || '', content: content || '', isDaily: false }
  if (currentView.value === 'notebook' && selectedNotebookId.value) payload.notebookId = selectedNotebookId.value
  if (currentView.value === 'projectNote' && selectedProjectNoteId.value) payload.projectIds = [selectedProjectNoteId.value]
  try {
    const r: any = await notesV2Create(payload)
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: title || '', excerpt: '', is_daily: 0, notebook_id: payload.notebookId || null, project_ids: payload.projectIds?.join(',') || '', updated_at: nowLocalDb(), created_at: nowLocalDb(), tags: [] }
    notes.value.unshift(newNote)
    selectedNoteId.value = newNote.note_id
    const prefilled = await richContentToHtml(content || '')
    editTitle.value = title || ''; editContent.value = prefilled; isDirty.value = false
    await nextTick()
    if (prefilled) editor.value?.commands.setContent(prefilled, false)
    editor.value?.commands.focus()
  } catch (e) { console.error('[Notes] create note prefilled failed:', e) }
}

function handleRouteQuery() {
  const action = route.query.action as string
  const focus = route.query.focus as string
  if (action === 'create') {
    const title = route.query.title as string || ''
    const content = route.query.content as string || ''
    // ★ 先消费后清 query：首挂时 onMounted+onActivated 双触发，重复执行会建两条笔记
    router.replace({ path: route.path, query: { ...(focus ? { focus } : {}) } })
    createNotePrefilled(title, content)
  }
  if (focus) {
    selectedNoteId.value = focus
  }
}
// ★ 同路由内 query 变化不触发 onActivated（KeepAlive 已激活），通知快捷操作跳转时在此消费
watch(() => route.query, () => {
  if (route.path === '/notes') handleRouteQuery()
})
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
  document.removeEventListener('mousemove', onDragMove)
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
  let tid = target?.dataset.dropTarget || null
  // "__none__"（未分类）只接受笔记拖入（移出笔记本），笔记本拖拽忽略该目标
  if (tid === '__none__' && dragType.value === 'notebook') tid = null
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
      const idsToMove = selectedCount.value > 1 && isSelected(noteId)
        ? [...selectedIds.value]
        : [noteId]
      const toUncategorized = targetId === '__none__'
      for (const rid of idsToMove) {
        await notesV2Update({ noteId: rid, notebookId: toUncategorized ? null : targetId })
      }
      notes.value = notes.value.filter(n => !idsToMove.includes(n.note_id) && !idsToMove.includes(n.id))
      clearMultiSelect()
      toast({ message: idsToMove.length > 1 ? `已移动 ${idsToMove.length} 条笔记` : '笔记已移动', type: 'success' })
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

// 上传弹窗（统一方案：选择文件 或 粘贴）
const uploadModalRef = ref<InstanceType<typeof UploadModal> | null>(null)
function openNoteUpload() {
  if (!selectedNoteId.value) return
  uploadModalRef.value?.open()
}
async function handleNoteUploaded() {
  await loadNoteFiles()
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
  try { await deleteNotebook(nid); notebooks.value = notebooks.value.filter(n => n.id !== nid); if (selectedNotebookId.value === nid) { selectedNotebookId.value = null; currentView.value = 'all' } } catch (e) { console.error('[Notes] deleteNotebook failed:', e) }
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
      <button @click="startAddNotebook" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" v-tooltip="'新建笔记本'">
        <AppIcon name="plus" size="12" stroke="2.5"/>
      </button>
    </div>
    <!-- Search -->
    <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
      <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
        <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
        <input v-model="navFilterQuery" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="筛选笔记本 / 项目..."/>
      </div>
    </div>

    <!-- Scrollable list: 闪记 → 笔记本区（全部 → 用户树 → 未分类）→ 项目笔记本 → 回收站 -->
    <div class="flex-1 px-2 py-1" style="overflow-y:auto;overflow-x:clip">
      <!-- 闪记 -->
      <button @click="switchView('daily')" class="flex items-center gap-1.5 w-full pr-2 py-1.5 text-xs text-left transition-colors rounded-lg mb-0.5 relative overflow-hidden" :style="{ paddingLeft: '2px' }" :class="currentView==='daily'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
        <div v-if="currentView==='daily'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <span class="w-3 h-3 flex-shrink-0"></span>
        <AppIcon name="bolt" size="14" class="text-surface-400" :class="currentView==='daily'?'text-blue-500':''"/>闪记
      </button>

      <!-- 分隔：闪记 / 用户笔记本 -->
      <div class="mb-2 border-t border-surface-100"></div>

      <!-- 全部笔记：虚拟视图组首（grid 图标 + pl-12） -->
      <button @click="switchView('all')" class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :style="{ paddingLeft: '2px' }" :class="currentView==='all'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
        <div v-if="currentView==='all'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <span class="w-3 h-3 flex-shrink-0"></span>
        <AppIcon name="grid" size="14" class="flex-shrink-0" :class="currentView==='all'?'text-blue-500':'text-surface-400'"/>全部笔记
      </button>

      <!-- User notebooks -->
      <template v-for="(nb, nbi) in notebookTree" :key="nb.id">
        <!-- Insertion line: above target -->
        <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='above'" class="insertion-line" :style="{ marginLeft: `${4 + (nb._depth || 0) * 16}px` }"></div>
        <div v-if="editingNotebookId===nb.id" class="flex items-center gap-1 px-2 py-1" :style="{ paddingLeft: `${2 + (nb._depth || 0) * 12}px` }">
          <input ref="notebookNameRef" data-notebook-edit v-model="notebookEditName" class="flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none focus:ring-1 focus:ring-brand-400 select-text" @keyup.enter="saveNotebookName(nb.id)" @blur="saveNotebookName(nb.id)"/>
        </div>
        <div v-else :data-drop-target="nb.id" class="flex items-center gap-1 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg group cursor-pointer relative overflow-hidden select-none" :class="[(currentView==='notebook'&&selectedNotebookId===nb.id)?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragNotebookId===nb.id?'opacity-30':'', isDragging && dragTargetId===nb.id && dragNotebookId!==nb.id ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']" @click="selectNotebook(nb.id)" @mousedown="startNotebookDrag($event, nb.id)" :style="{ paddingLeft: `${2 + (nb._depth || 0) * 12}px` }">
  <div v-if="currentView==='notebook'&&selectedNotebookId===nb.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r pointer-events-none"></div>
          <!-- Insertion line: inside target -->
          <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='inside'" class="absolute inset-0 border-2 border-brand-400 rounded-lg pointer-events-none"></div>
          <!-- Toggle arrow for parents -->
          <span v-if="nb._hasChildren" @click.stop="toggleNotebook(nb.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0 mr-0.5 relative left-[5px]">
            <svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedNotebooks.has(nb.id) ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
          </span>
          <span v-else class="w-3 flex-shrink-0 mr-0.5"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0" :class="currentView==='notebook'&&selectedNotebookId===nb.id?'!text-blue-500':'text-surface-400'"/>
          <span class="truncate flex-1">{{ nb.name }}</span>
          <span @mousedown.stop @click.stop="startAddNotebook(nb.id)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100" v-tooltip="'新建子笔记本'"><AppIcon name="plus" size="10"/></span>
          <span @mousedown.stop @click.stop="startEditNotebook(nb)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="edit" size="10"/></span>
          <span @click.stop="removeNotebook(nb.id)" class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="10"/></span>
        </div>
        <!-- Insertion line: below target -->
        <div v-if="isDragging && dragType==='notebook' && dragInsert?.targetId===nb.id && dragInsert?.side==='below'" class="insertion-line" :style="{ marginLeft: `${4 + (nb._depth || 0) * 16}px` }"></div>
      </template>

      <!-- 未分类：虚拟组尾（对齐资料库"未分类"分隔层样式）；拖笔记到此=移出笔记本 -->
      <div class="mt-2 pt-1.5 border-t border-surface-100">
        <button @click="switchView('uncategorized')" data-drop-target="__none__" class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden select-none" :style="{ paddingLeft: '2px' }" :class="[currentView==='uncategorized'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragType==='note' && dragTargetId==='__none__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']">
          <div v-if="currentView==='uncategorized'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r pointer-events-none"></div>
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" :class="currentView==='uncategorized'?'!text-blue-500':''"/>未分类
        </button>
      </div>

      <!-- 分隔：项目笔记 -->
      <div class="mb-2 mt-1 border-t border-surface-100"></div>

      <!-- 项目笔记（两级树：顶层=聚合所有项目笔记；项目名=该项目笔记。"全部"已排除项目笔记） -->
      <div @click="selectProjectNote(null)" class="flex items-center gap-1.5 w-full pr-2 py-1.5 text-xs text-left transition-colors rounded-lg mb-0.5 relative overflow-hidden cursor-pointer select-none" :style="{ paddingLeft: '2px' }" :class="currentView==='projectNote'&&!selectedProjectNoteId?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
        <div v-if="currentView==='projectNote'&&!selectedProjectNoteId" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r pointer-events-none"></div>
        <span @click.stop="projectNotesExpanded=!projectNotesExpanded" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0 relative left-[5px]">
          <svg class="w-2.5 h-2.5 transition-transform duration-150" :class="projectNotesExpanded?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
        </span>
        <AppIcon name="folder" size="13" class="flex-shrink-0" :class="currentView==='projectNote'&&!selectedProjectNoteId?'!text-blue-500':'text-surface-400'"/>项目笔记
      </div>
      <template v-if="projectNotesExpanded">
        <div v-for="p in filteredProjectNotes" :key="p.id" @click="selectProjectNote(p.id)" v-tooltip.auto="p.name" class="flex items-center gap-1.5 w-full pr-2 py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden cursor-pointer select-none" :class="currentView==='projectNote'&&selectedProjectNoteId===p.id?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'" style="padding-left:32px">
          <div v-if="currentView==='projectNote'&&selectedProjectNoteId===p.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r pointer-events-none"></div>
          <AppIcon name="folder" size="12" class="flex-shrink-0 text-surface-400"/>
          <span class="truncate flex-1 min-w-0">{{ p.name }}</span>
        </div>
      </template>

      <!-- 分隔：回收站 -->
      <div class="mt-2 pt-1.5 border-t border-surface-100">
        <button @click="switchView('trash')" class="flex items-center gap-1.5 w-full pr-2 py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden" :style="{ paddingLeft: '2px' }" :class="currentView==='trash'?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100'">
          <div v-if="currentView==='trash'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="trash" size="13" class="text-surface-400"/>回收站
        </button>
      </div>
    </div>
  </aside>

  <!-- Note List -->
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{width:listWidth+'px'}" v-if="currentView!=='trash'">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('list', $event)"></div>
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">{{ listHeaderTitle }}</span>
      <button v-if="currentView!=='daily' && !(currentView==='projectNote' && !selectedProjectNoteId)" @click="createNote" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" v-tooltip="'新建'"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
      <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
        <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
        <input v-model="noteFilterQuery" type="text" class="flex-1 min-w-0 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="筛选笔记（标题/内容）..."/>
      </div>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1.5">
      <div v-for="note in displayedNotes" :key="note.note_id" @mousedown="onNoteMouseDown($event, note.note_id||note.id)" class="flex items-center px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden hover:scale-[1.02] hover:z-10 select-none" :class="[selectedNoteId===(note.note_id||note.id)||isSelected(note.note_id||note.id)?'bg-blue-50/70 border-brand-200 shadow-none scale-100':'border-surface-100 hover:border-surface-300 hover:shadow-md', isDragging && dragNoteId===(note.note_id||note.id)?'opacity-50':'']" @click="onNoteCardClick($event, note.note_id||note.id)">
        <div v-if="selectedNoteId===(note.note_id||note.id)" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <div class="flex-1 min-w-0">
          <p class="text-[12px] font-medium text-surface-800 truncate">{{ currentView==='daily' ? (note.title||(note.created_at||'').slice(0,10)) : (note.title||'未命名笔记') }}</p>
          <p class="text-[10px] text-surface-400 line-clamp-1 mt-0.5">{{ (note.excerpt||'').replace(/<[^>]*>/g,'').slice(0,40) || '暂无内容' }}</p>
          <p class="text-[9px] text-surface-300 mt-0.5"><span v-if="currentView==='all'||currentView==='uncategorized'" class="text-surface-400">{{ note.notebook_name || (note.is_daily ? '闪记' : (note.project_ids ? (projectNamesOf(note.project_ids) || '项目笔记') : '未分类')) }} · </span><span v-else-if="currentView==='projectNote'" class="text-surface-400">{{ projectNamesOf(note.project_ids) || '项目笔记' }} · </span>{{ formatNoteTime(note.updated_at || note.created_at || '') }}</p>
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
          <button @click.stop="openRestoreDialog(note.note_id)" class="w-7 h-7 flex items-center justify-center rounded text-surface-500 hover:text-blue-600 hover:bg-blue-50 transition-colors" v-tooltip="'恢复'"><AppIcon name="undo" size="14"/></button>
          <button @click.stop="permanentDelete(note.note_id)" class="w-7 h-7 flex items-center justify-center rounded text-surface-500 hover:text-red-600 hover:bg-red-50 transition-colors" v-tooltip="'永久删除'"><AppIcon name="trash" size="14"/></button>
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
      <div class="flex-1 overflow-y-auto px-6 pb-8"><div class="text-sm text-surface-600 leading-relaxed" v-html="sanitizeHtmlSync(editContent || '')"></div></div>
    </template>
    <div v-else class="flex-1 flex items-center justify-center text-surface-400 text-sm">选择左侧笔记查看内容</div>
  </div>

  <!-- Editor -->
  <div class="flex-1 flex flex-col min-w-0 overflow-hidden relative" v-if="currentView!=='trash'">
    <template v-if="selectedNoteId">
      <RichTextToolbar :editor="editor" :at-active="atOpen" @at="openAtSelector" />
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
          @open-upload="openNoteUpload"
          @preview="(file) => previewFile(file.id)"
          @insert="(file) => insertNoteFileToEditor(file)"
          @delete="(file) => removeNoteFile(file.id)"
          @rename="(file, name) => renameNoteFile(file, name)"
        />
        <UploadModal
          ref="uploadModalRef"
          entity-type="note"
          title="上传笔记附件"
          :resolve-entity-id="() => selectedNoteId.value || undefined"
          @uploaded="handleNoteUploaded"
        />
      </div>
    </template>
    <div v-else class="flex-1 flex items-center justify-center text-surface-400 text-sm">选择或创建一篇笔记</div>
  </div>

</div>


<!-- Drag ghost -->
<Teleport to="body">
  <div v-if="isDragging" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}">
    <template v-if="dragType === 'note'">{{ selectedCount > 1 ? `${selectedCount} 条笔记` : `${notes.find(n => (n.note_id||n.id) === dragNoteId)?.title || '笔记'} · Shift+点击可多选` }}</template>
    <template v-else>{{ notebooks.find(n => n.id === dragNotebookId)?.name || '笔记本' }}</template>
  </div>
</Teleport>

<!-- Restore dialog -->
<Teleport to="body"><Transition name="modal-fade"><div v-if="restoreDialogOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm"><div class="bg-white rounded-2xl w-[420px] max-h-[75vh] overflow-hidden shadow-2xl flex flex-col border border-surface-200">
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