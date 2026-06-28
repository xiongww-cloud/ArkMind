<script setup lang="ts">
import { ref, computed, onMounted, onActivated, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useSelectionStore } from '@/stores/selection'
import AppIcon from '@/components/common/AppIcon.vue'
import TagPanel from '@/components/common/TagPanel.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import { useUploadProgress } from '@/composables/useUploadProgress'
import { fetchProjects, fetchTodos, fetchFiles, createTodo as apiCreateTodo, updateTodo as apiUpdateTodo, deleteTodo as apiDeleteTodo, toggleTodo as apiToggleTodo, previewFile, uploadFile, deleteFileFromDomain, fetchFilesByTodo, fetchFileTree } from '@/api/db-client'
import TodoDetail from './TodoDetail.vue'

interface Todo { id: string; title: string; content: string; done: boolean; createdAt: string; dueDate?: string; priority?: 'P1'|'P2'|'P3'; repeat?: 'daily'|'weekly'|'monthly'; projectId?: string; projectName?: string }
interface Project { id:string; name:string }

const listWidth = ref(260)
const loading = ref(true)

const projects = ref<Project[]>([])
const todos = ref<Todo[]>([])
const lastTodoSave = ref('')
let todoLoadGen = 0
const route = useRoute()
const selectionStore = useSelectionStore()
const router = useRouter()

const selectedTodoId = ref<string|null>(null)
const selectedTodo = computed(() => todos.value.find(t => t.id === selectedTodoId.value) || null)
function selectTodo(id: string) { selectedTodoId.value = id }

// ─── Attachments ──────────────────────────────────────────────────
interface AttachedFile { id: string; file_ext?: string; file_size: number; lib_gn?: string; lib_cn?: string; proj_gn?: string; proj_cn?: string; prod_gn?: string; prod_cn?: string; displayName?: string; name?: string; custom_name?: string }
const todoFiles = ref<AttachedFile[]>([])
const filesLoading = ref(false)
const isUploading = ref(false)
const attTab = ref<'todo'|'project'>('todo')
const editingFileId = ref<string|null>(null)
const editingFileName = ref('')

// ★ 项目文件目录树状态
const projFilesPanel = ref<any[]>([])
const projFilesFolders = ref<any[]>([])
const projFilesExpanded = ref<Set<string|null>>(new Set())
const projFilesGrouped = computed(() => {
  const folderMap = new Map(projFilesFolders.value.map((f: any) => [f.id, f.name]))
  const byFolder = new Map<string|null, any[]>()
  for (const f of projFilesPanel.value) {
    const fid = (f as any).folder_id || null
    if (!byFolder.has(fid)) byFolder.set(fid, [])
    byFolder.get(fid)!.push(f)
  }
  const groups: { folderId: string|null; folderName: string; files: any[] }[] = []
  for (const [fid, files] of byFolder) {
    groups.push({ folderId: fid, folderName: fid ? folderMap.get(fid) || '未命名' : '未分类', files })
  }
  groups.sort((a, b) => {
    if (a.folderId === null && b.folderId === null) return 0
    if (a.folderId === null) return 1
    if (b.folderId === null) return -1
    return a.folderName.localeCompare(b.folderName)
  })
  return groups
})
function toggleProjFolder(fid: string|null) {
  if (projFilesExpanded.value.has(fid)) projFilesExpanded.value.delete(fid)
  else projFilesExpanded.value.add(fid)
}

async function loadTodoAttachments() {
  if (!selectedTodoId.value) { todoFiles.value = []; return }
  filesLoading.value = true
  try {
    const r: any = await fetchFilesByTodo(selectedTodoId.value)
    todoFiles.value = Array.isArray(r) ? r : (r?.files || [])
  } catch { todoFiles.value = [] }
  finally { filesLoading.value = false }
}

async function loadProjectFiles() {
  if (!selectedTodoId.value) { projFilesPanel.value = []; return }
  const t = todos.value.find(x => x.id === selectedTodoId.value)
  if (!t?.projectId) { projFilesPanel.value = []; return }
  filesLoading.value = true
  try {
    const backendId = t.projectId
    const [filesData, treeData] = await Promise.all([
      fetchFiles(backendId),
      fetchFileTree(backendId)
    ])
    projFilesPanel.value = Array.isArray(filesData) ? filesData : []
    projFilesFolders.value = treeData?.folders || []
  } catch { projFilesPanel.value = []; projFilesFolders.value = [] }
  finally { filesLoading.value = false }
}

async function refreshAttachments() {
  if (!selectedTodoId.value) { todoFiles.value = []; return }
  const t = todos.value.find(x => x.id === selectedTodoId.value)
  if (t?.projectId) {
    // 关联项目时按当前tab加载
    if (attTab.value === 'project') loadProjectFiles()
    else loadTodoAttachments()
  } else {
    loadTodoAttachments()
  }
}

watch(selectedTodoId, () => { attTab.value = 'todo'; refreshAttachments() })
watch(attTab, () => refreshAttachments())
onMounted(refreshAttachments)
onActivated(refreshAttachments)

function fileDisplayName(f: AttachedFile): string {
  return (f as any).custom_name || (f as any).global_name || f.lib_gn || f.lib_cn || f.proj_gn || f.proj_cn || f.prod_gn || f.prod_cn || (f as any).mapping_name || '未命名'
}

function previewAttachment(fileId: string) {
  previewFile(fileId).catch(() => toast({ message: '预览失败', type: 'error' }))
}

function startRename(f: AttachedFile) {
  editingFileId.value = f.id
  editingFileName.value = f.custom_name || (f as any).global_name || f.lib_gn || f.lib_cn || f.proj_gn || f.proj_cn || f.prod_gn || f.prod_cn || (f as any).mapping_name || ''
  import('vue').then(({ nextTick }) => nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-todo-rename-input]')
    el?.focus(); el?.select()
  }))
}
async function confirmRename(f: AttachedFile) {
  const n = editingFileName.value.trim()
  if (n) {
    try {
      const { updateFileMeta } = await import('@/api/db-client')
      await updateFileMeta(f.id, { customName: n, relationId: f.relation_id })
      await refreshAttachments()
    } catch (e) { console.error('[Todo] renameAttachment failed:', e) }
  }
  editingFileId.value = null
}
function cancelRename() { editingFileId.value = null }

function fileTypeClass(ext: string) {
  const map: Record<string, string> = {
    docx: 'bg-blue-50 text-blue-600', doc: 'bg-blue-50 text-blue-600',
    pdf: 'bg-red-50 text-red-600',
    xlsx: 'bg-emerald-50 text-emerald-600', xls: 'bg-emerald-50 text-emerald-600',
    pptx: 'bg-orange-50 text-orange-600', ppt: 'bg-orange-50 text-orange-600',
    png: 'bg-purple-50 text-purple-600', jpg: 'bg-purple-50 text-purple-600', jpeg: 'bg-purple-50 text-purple-600',
  }
  return map[ext.toLowerCase()] || 'bg-slate-50 text-slate-500'
}

function formatTodoFileSize(bytes: number | undefined): string {
  if (!bytes || bytes === 0) return '--'
  if (bytes >= 1073741824) return (bytes/1073741824).toFixed(1)+' GB'
  if (bytes >= 1048576) return (bytes/1048576).toFixed(1)+' MB'
  if (bytes >= 1024) return (bytes/1024).toFixed(0)+' KB'
  return bytes+' B'
}

const { start: tdStart, done: tdDone, fail: tdFail } = useUploadProgress()

// ─── Upload / Remove ───────────────────────────────────────────
async function handleUpload() {
  if (!selectedTodoId.value) return
  const { open } = await import('@tauri-apps/plugin-dialog')
  const result = await open({ multiple: false })
  if (!result) return
  const filePath = result as string
  const fileName = filePath.replace(/^.*[\\\/]/, '')
  const ext = fileName.includes('.') ? fileName.split('.').pop() || '' : ''
  const task = tdStart(fileName)
  try {
    const r: any = await uploadFile({ filePath, fileName, fileExt: ext, customName: fileName, entityType: 'todo', entityId: selectedTodoId.value })
    const fileId = r?.fileId || r?.id
    if (fileId) {
      const list: any = await fetchFilesByTodo(selectedTodoId.value)
      todoFiles.value = list?.files || []
      tdDone(task); toast({ message: '附件已上传', type: 'success' })
    }
  } catch (e: any) { tdFail(task, e.message || '上传失败'); toast({ message: '上传失败: ' + (e.message || e), type: 'error' }) }
}

async function removeAttachment(fileId: string) {
  if (!selectedTodoId.value) return
  const ok = await dangerConfirm({ title: '删除附件', message: '确定删除此附件？', confirmText: '删除' })
  if (!ok) return
  try {
    await deleteFileFromDomain(fileId, 'todo', selectedTodoId.value)
    todoFiles.value = todoFiles.value.filter(f => f.id !== fileId)
    toast({ message: '已移除附件', type: 'success' })
  } catch { toast({ message: '移除失败', type: 'error' }) }
}

// ─── Insert file to editor ──────────────────────────────
const todoDetailRef = ref<any>(null)
function insertAttachmentToEditor(f: AttachedFile) {
  const name = fileDisplayName(f)
  todoDetailRef.value?.insertFileAtCursor(f.id, name)
  toast({ message: '已插入到光标位置', type: 'success' })
}

async function loadTodos() {
  loading.value = true
  try {
    const r = await fetchTodos()
    if (r?.todos) {
      todos.value = r.todos.map((t: any) => ({
        id: t.id,
        title: t.title || '',
        content: t.content || '',
        done: t.done === 1 || t.done === true,
        createdAt: t.created_at || new Date().toISOString(),
        dueDate: t.due_date || undefined,
        priority: t.priority || undefined,
        repeat: t.repeat || undefined,
        projectId: t.project_id || undefined,
        projectName: t.project_name || '',
      }))
    }
  } catch (e: any) {
    toast({ message: '加载待办失败: ' + (e.message || e), type: 'error' })
  } finally { loading.value = false; todoLoadGen++ }
}

async function handleCreate() {
  try {
    const r = await apiCreateTodo({ title: '新待办' })
    if (r?.todo) {
      const t = r.todo
      todos.value.unshift({
        id: t.id, title: t.title || '新待办', content: t.content || '',
        done: false, createdAt: t.created_at || new Date().toISOString(),
        dueDate: t.due_date || undefined, priority: t.priority || undefined,
        repeat: t.repeat || undefined,
        projectId: t.project_id || undefined, projectName: t.project_name || '',
      })
      selectedTodoId.value = t.id
    }
  } catch (e: any) {
    toast({ message: '创建失败: ' + (e.message || e), type: 'error' })
  }
}

async function updateTodoLocal(id: string, data: Partial<Todo>) {
  const idx = todos.value.findIndex(t => t.id === id)
  if (idx === -1) return
  Object.assign(todos.value[idx], data)
  try {
    const payload: Record<string, any> = {}
    if (data.title !== undefined) payload.title = data.title
    if (data.content !== undefined) payload.content = data.content
    if (data.done !== undefined) payload.done = data.done
    if (data.priority !== undefined) payload.priority = data.priority || ''
    if (data.dueDate !== undefined) payload.dueDate = data.dueDate || ''
    if (data.repeat !== undefined) payload.repeat = data.repeat || ''
    if ('projectId' in data) payload.projectId = data.projectId || ''
    const snapshot = JSON.stringify(payload)
    if (snapshot === lastTodoSave.value) return  // 无变化，跳过
    await apiUpdateTodo(id, payload)
    lastTodoSave.value = snapshot
  } catch (e: any) {
    toast({ message: '更新失败: ' + (e.message || e), type: 'error' })
  }
}

async function toggleDone(id: string) {
  try {
    const r = await apiToggleTodo(id)
    if (r?.todo) {
      const idx = todos.value.findIndex(t => t.id === id)
      if (idx !== -1) {
        todos.value[idx].done = r.todo.done === 1 || r.todo.done === true
        if (r.repeat_advanced) {
          todos.value[idx].dueDate = r.todo.due_date || undefined
          toast({ message: '✅ 已完成，下次打开自动重置', type: 'success' })
        }
      }
    }
  } catch (e: any) {
    console.error('[toggleDone] error:', e)
    toast({ message: '操作失败: ' + (e.message || e), type: 'error' })
  }
}

async function handleDelete(id: string) {
  const ok = await dangerConfirm({ title: '删除待办', message: '确定删除此待办？', confirmText: '删除' })
  if (!ok) return
  if (id === selectedTodoId.value) selectedTodoId.value = null
  todos.value = todos.value.filter(t => t.id !== id)
  try {
    await apiDeleteTodo(id)
    toast({ message: '待办已删除', type: 'success' })
  } catch (e: any) {
    toast({ message: '删除失败: ' + (e.message || e), type: 'error' })
    await loadTodos() // refresh on failure
  }
}

// ── Sort
const doneDrawerOpen = ref(false)
const priorityRank: Record<string, number> = { 'P1': 1, 'P2': 2, 'P3': 3 }
const sortByDate = (arr: any[]) => [...arr].filter(t => t.dueDate).sort((a, b) => new Date(a.dueDate).getTime() - new Date(b.dueDate).getTime())
const sortByPriority = (arr: any[]) => [...arr].filter(t => !t.dueDate).sort((a, b) => (priorityRank[a.priority] || 999) - (priorityRank[b.priority] || 999))
const undoneTodos = computed(() => {
  const undone = todos.value.filter(t => !t.done)
  return [...sortByDate(undone), ...sortByPriority(undone)]
})
const doneTodos = computed(() => {
  const done = todos.value.filter(t => t.done)
  return [...sortByDate(done), ...sortByPriority(done)]
})

onMounted(async () => {
  try { projects.value = await fetchProjects() } catch { projects.value = [] }
  await loadTodos()
})

// 监听路由参数 + 全局搜索跳转
watch(() => route.query, (q) => {
  if (q.create === 'true') {
    router.replace('/todos').then(() => setTimeout(() => handleCreate(), 50))
  }
  if (q.select) {
    selectedTodoId.value = q.select as string
  }
})
watch(() => selectionStore.current.seq, () => {
  if (selectionStore.current.type === 'todo') {
    selectedTodoId.value = selectionStore.current.id
    selectionStore.clear()
  }
}, { immediate: true })

onActivated(async () => {
  try { projects.value = await fetchProjects() } catch { projects.value = [] }
  await loadTodos()
})

// ─── Priority helpers ──────────────────────────────────────────────
const PRIO = { color: { P1: '#ef4444', P2: '#f59e0b', P3: '#3b82f6' } as Record<string,string>, bar: { P1: '#ef4444', P2: '#f59e0b', P3: '#3b82f6' } as Record<string,string>, ring: { P1: 'ring-red-300', P2: 'ring-amber-300', P3: 'ring-blue-300' } as Record<string,string> }

function fmtDate(d?: string): string {
  if (!d) return '无'
  try {
    const date = new Date(d); if (isNaN(date.getTime())) return '无'
    const today = new Date(); today.setHours(0,0,0,0)
    const tomorrow = new Date(today); tomorrow.setDate(tomorrow.getDate()+1)
    const prefix = date.getTime() === today.getTime() ? '今天' : date.getTime() === tomorrow.getTime() ? '明天' : ''
    const datePart = `${date.getFullYear()}/${date.getMonth()+1}/${date.getDate()}`
    const timePart = d.includes('T') ? ` ${String(date.getHours()).padStart(2,'0')}:${String(date.getMinutes()).padStart(2,'0')}` : ''
    const full = datePart + timePart
    return prefix ? `${prefix} · ${full}` : full
  } catch { return '无' }
}
function isOverdue(t: Todo): boolean {
  if (!t.dueDate || t.done) return false
  const d = new Date(t.dueDate); if (isNaN(d.getTime())) return false
  const today = new Date(); today.setHours(0,0,0,0)
  return d.getTime() < today.getTime()
}
</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm">
  <!-- ═══ Column 1: Todo List ═══ -->
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200" :style="{ width: listWidth + 'px' }">
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">待办清单</span>
      <div class="flex items-center gap-2">
        <span class="text-[10px] text-surface-400 font-medium">{{ undoneTodos.length }}</span>
        <button @click="handleCreate" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建待办">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>
        </button>
      </div>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1.5">
      <div v-if="loading" class="text-center py-8">
        <p class="text-xs text-surface-400">加载中...</p>
      </div>
      <div v-else>
        <!-- Undone todos -->
        <div v-for="todo in undoneTodos" :key="todo.id"
          class="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden"
          :class="[
            'border-surface-100 hover:border-surface-300 hover:shadow-md',
            selectedTodoId === todo.id ? 'shadow-lg scale-[1.02] z-10' : 'hover:shadow-lg hover:scale-[1.02] hover:z-10',
            selectedTodoId === todo.id && todo.priority ? 'bg-blue-50/70' : selectedTodoId === todo.id ? 'bg-gray-50' : ''
          ]"
          :style="selectedTodoId === todo.id && todo.priority ? { borderColor: PRIO.color[todo.priority] } : selectedTodoId === todo.id ? { borderColor: '#d1d5db' } : {}"
          @click="selectTodo(todo.id)"
        >
          <div v-if="selectedTodoId === todo.id" class="absolute left-0 top-0 bottom-0 w-[3px] rounded-r" :style="{ background: todo.priority ? PRIO.color[todo.priority] : '#9ca3af' }"></div>
          <button class="w-5 h-5 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-200 hover:shadow-md"
            :style="{ borderColor: todo.priority ? PRIO.color[todo.priority] : '#d1d5db' }"
            @click.stop="toggleDone(todo.id)"
          >
            <svg v-if="todo.done" class="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
          </button>
          <div class="flex-1 min-w-0">
            <p class="text-[12px] font-medium truncate text-surface-800">{{ todo.title }}</p>
            <p class="text-[9px] mt-1 text-surface-500"><span class="text-surface-400">项目：</span>{{ todo.projectName || '无' }}</p>
            <div class="flex items-center gap-1.5 mt-0.5">
              <p class="text-[9px]" :class="isOverdue(todo) ? 'text-red-500' : 'text-surface-500'" v-text="fmtDate(todo.dueDate)" />
              <span v-if="todo.repeat" class="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[9px] font-medium" :class="todo.repeat==='daily'?'bg-blue-50 text-blue-600':todo.repeat==='weekly'?'bg-purple-50 text-purple-600':'bg-green-50 text-green-600'">
                <svg class="w-2 h-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
                <span>{{ todo.repeat==='daily'?'每天':todo.repeat==='weekly'?'每周':'每月' }}</span>
              </span>
            </div>
          </div>
          <button @click.stop="handleDelete(todo.id)" class="w-4 h-4 flex items-center justify-center rounded-full text-surface-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6M10 10v7M14 10v7"/></svg>
          </button>
        </div>
        <div v-if="undoneTodos.length === 0" class="text-center py-8">
          <p class="text-xs text-surface-400">暂无待办事项</p>
        </div>
      </div>
    </div>

    <!-- Done drawer -->
    <div v-if="doneTodos.length > 0" class="border-t border-surface-100 bg-white">
      <button @click="doneDrawerOpen = !doneDrawerOpen" class="w-full flex items-center gap-1.5 px-3 py-2 text-[10px] text-surface-400 hover:text-surface-600 transition-colors">
        <svg class="w-3 h-3 transition-transform" :class="doneDrawerOpen ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 5l7 7-7 7"/></svg>
        <span>已完成</span>
        <span class="ml-auto text-surface-300">{{ doneTodos.length }} 项</span>
      </button>
      <div v-show="doneDrawerOpen" class="px-2 pb-2 space-y-1 max-h-[200px] overflow-y-auto">
        <div v-for="todo in doneTodos" :key="todo.id"
          class="flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer group border bg-gray-50/80 border-gray-200"
          :class="selectedTodoId === todo.id ? 'bg-gray-100/70 border-gray-300' : ''"
          @click="selectTodo(todo.id)"
        >
          <button class="w-4 h-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all"
            style="border-color: #9ca3af; background: #9ca3af"
            @click.stop="toggleDone(todo.id)"
          >
            <svg class="w-2.5 h-2.5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
          </button>
          <div class="flex-1 min-w-0">
            <p class="text-[11px] font-medium truncate text-gray-400 line-through">{{ todo.title }}</p>
            <div class="flex items-center gap-1 mt-0.5">
              <span class="text-[8px] text-gray-300" v-text="fmtDate(todo.dueDate)" />
              <span v-if="todo.repeat" class="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[8px] font-medium bg-gray-100 text-gray-400">
                <svg class="w-2 h-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
                <span>{{ todo.repeat==='daily'?'每天':todo.repeat==='weekly'?'每周':'每月' }}</span>
              </span>
            </div>
          </div>
          <button @click.stop="handleDelete(todo.id)" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除">
            <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6M10 10v7M14 10v7"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>

  <!-- ═══ Column 2: Todo Detail ═══ -->
  <TodoDetail
    :key="'td-' + todoLoadGen"
    ref="todoDetailRef"
    v-if="selectedTodo"
    :todo="selectedTodo"
    :projects="projects"
    @update="(id, data) => updateTodoLocal(id, data)"
    @close="selectedTodoId = null"
  />
  <div v-else class="flex-1 flex items-center justify-center text-surface-400 text-sm">选择或创建一条待办</div>

  <!-- ═══ Column 3: Attachments / Project files ═══ -->
  <div v-if="selectedTodoId" :key="'ac-' + selectedTodoId" class="flex-shrink-0 w-[300px] border-l border-surface-200 flex flex-col bg-surface-50/20">
    <!-- Tabs: 附件 + 项目文件 (only when project-linked) -->
    <div v-if="selectedTodo?.projectId" class="flex border-b border-surface-100 shrink-0 h-10">
      <button
        @click="attTab = 'todo'"
        class="flex-1 py-1.5 text-[10px] font-medium transition-colors"
        :class="attTab === 'todo' ? 'text-brand-600 border-b-2 border-brand-600' : 'text-surface-500 hover:text-surface-700'"
      >附件</button>
      <button
        @click="attTab = 'project'"
        class="flex-1 py-1.5 text-[10px] font-medium transition-colors"
        :class="attTab === 'project' ? 'text-brand-600 border-b-2 border-brand-600' : 'text-surface-500 hover:text-surface-700'"
      >项目文件</button>
    </div>
    <div v-else class="flex items-center px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">附件</span>
    </div>
    <!-- Upload button row (only for attachment tab) -->
    <div v-if="attTab !== 'project'" class="px-2 pt-1.5 pb-1 shrink-0">
      <button
        @click="handleUpload()"
        :disabled="isUploading"
        class="w-full flex items-center justify-center gap-1.5 py-1.5 rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors text-[10px] disabled:opacity-50 cursor-pointer"
      >
        <AppIcon name="plus" size="10" stroke="2.5"/>
        <span>上传附件</span>
      </button>
    </div>
    <div class="flex-1 overflow-y-auto p-2">
      <!-- 项目文件：目录树 -->
      <div v-if="attTab === 'project'">
        <div v-if="filesLoading" class="text-center py-4"><p class="text-xs text-surface-400">加载中...</p></div>
        <template v-else-if="projFilesGrouped.length > 0">
          <div v-for="group in projFilesGrouped" :key="group.folderId || '__uncat__'" class="mb-0.5">
            <button @click="toggleProjFolder(group.folderId)" class="w-full flex items-center gap-1 px-1.5 py-1 rounded text-[10px] font-medium text-surface-500 hover:text-surface-700 hover:bg-surface-100 transition-colors cursor-pointer">
              <svg class="w-2.5 h-2.5 transition-transform shrink-0" :class="projFilesExpanded.has(group.folderId) ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
              <svg class="w-3.5 h-3.5 shrink-0" :class="group.folderId ? 'text-amber-500' : 'text-surface-400'" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
              <span class="truncate ml-1">{{ group.folderName }}</span>
              <span class="ml-auto text-surface-300 tabular-nums">{{ group.files.length }}</span>
            </button>
            <div v-if="projFilesExpanded.has(group.folderId)" class="ml-3 space-y-0.5 mt-0.5">
              <div v-for="f in group.files" :key="f.id" class="group relative flex items-center py-1 px-1.5 rounded hover:bg-white hover:shadow-sm transition-all">
                <div class="w-6 h-6 flex items-center justify-center rounded font-bold text-[7px] tracking-wider shrink-0 select-none" :class="fileTypeClass(f.file_ext || '')">{{ (f.file_ext || 'file').toUpperCase() }}</div>
                <div class="ml-1.5 flex-1 min-w-0 flex items-center gap-1">
                  <p class="text-[10px] font-medium text-surface-700 truncate leading-tight cursor-pointer hover:text-blue-600 flex-1 min-w-0" :title="fileDisplayName(f)" @click="previewAttachment(f.id)">{{ fileDisplayName(f) }}</p>
                  <button @click.stop="insertAttachmentToEditor(f)" class="shrink-0 w-4.5 h-4.5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100" title="插入到待办"><AppIcon name="file-plus" size="12"/></button>
                </div>
              </div>
            </div>
          </div>
        </template>
        <div v-else class="h-48 flex flex-col items-center justify-center text-center px-6">
          <div class="w-14 h-14 rounded-2xl bg-gray-50 flex items-center justify-center text-gray-300 mb-4">
            <svg class="h-7 w-7" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.2"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
          </div>
          <p class="text-[13px] font-medium text-gray-400">暂无项目文件</p>
        </div>
      </div>
      <!-- 附件：平铺列表 -->
      <div v-else>
        <div v-if="filesLoading" class="text-center py-4">
          <p class="text-xs text-surface-400">加载中...</p>
        </div>
        <div v-else-if="todoFiles.length > 0" class="space-y-1.5">
        <div
          v-for="f in todoFiles" :key="f.id"
          class="group relative flex items-center p-2 rounded-xl border border-gray-100 bg-white hover:border-gray-200 hover:shadow-sm transition-all duration-200"
          :class="editingFileId === f.id ? 'cursor-default' : ''"
        >
          <!-- Type badge -->
          <div class="w-9 h-9 flex items-center justify-center rounded-lg font-bold text-[8px] tracking-wider shrink-0 select-none" :class="fileTypeClass(f.file_ext || '')">{{ (f.file_ext || 'file').toUpperCase() }}</div>
          <!-- File info -->
          <div class="ml-2.5 flex-1 min-w-0 pr-16">
            <template v-if="editingFileId === f.id">
              <input
                v-model="editingFileName"
                data-todo-rename-input
                class="w-full text-[11px] font-medium text-gray-700 bg-transparent outline-none border-b border-brand-400 pb-0.5"
                @keyup.enter="confirmRename(f)"
                @blur="confirmRename(f)"
                @click.stop
                @keyup.escape="cancelRename()"
              />
            </template>
            <template v-else>
              <p class="text-[11px] font-medium text-gray-700 truncate leading-tight cursor-pointer hover:text-blue-600 transition-colors" :title="fileDisplayName(f)" @click="previewAttachment(f.id)">{{ fileDisplayName(f) }}</p>
            </template>
            <p class="text-[9px] text-gray-400 mt-0.5">{{ formatTodoFileSize(f.file_size) }} · {{ (f.mapped_at || f.updated_at || f.created_at || '').slice(0,16) }}</p>
          </div>
          <!-- Action buttons -->
          <div class="absolute right-1.5 top-1/2 -translate-y-1/2 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity duration-150 bg-gradient-to-l from-white via-white pl-4">
            <button
              @click.stop="startRename(f)"
              class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"
              title="重命名"
            ><AppIcon name="edit" size="13"/></button>
            <button
              v-if="selectedTodoId"
              @click.stop="insertAttachmentToEditor(f)"
              class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"
              title="插入到光标处"
            ><AppIcon name="file-plus" size="13"/></button>
            <button
              v-if="attTab !== 'project'"
              @click.stop="removeAttachment(f.id)"
              class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500"
              title="删除附件"
            ><AppIcon name="trash" size="13"/></button>
          </div>
        </div>
      </div>
      <div v-else class="h-48 flex flex-col items-center justify-center text-center px-6">
        <div class="w-12 h-12 rounded-xl bg-gray-50 flex items-center justify-center text-gray-300 mb-3">
          <svg class="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.2"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
        </div>
        <p class="text-[12px] font-medium text-gray-400">{{ attTab === 'project' ? '项目暂无文件' : '暂无附件' }}</p>
        <p class="text-[10px] text-gray-300 mt-1" v-if="attTab !== 'project'">点击 + 上传文件</p>
      </div>
      </div>
    </div>
    <!-- 标签 -->
    <div class="shrink-0 px-3 py-2 border-t border-surface-100">
      <TagPanel entity-type="todos" :entity-id="selectedTodoId" />
    </div>
  </div>
</div>

</template>
