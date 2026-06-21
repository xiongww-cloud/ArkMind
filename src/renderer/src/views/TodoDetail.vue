<script setup lang="ts">
import { ref, watch, nextTick, computed, onUnmounted } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Underline from '@tiptap/extension-underline'
import TextAlign from '@tiptap/extension-text-align'
import Placeholder from '@tiptap/extension-placeholder'
import Table from '@tiptap/extension-table'
import TableRow from '@tiptap/extension-table-row'
import TableCell from '@tiptap/extension-table-cell'
import TableHeader from '@tiptap/extension-table-header'
import TaskList from '@tiptap/extension-task-list'
import TaskItem from '@tiptap/extension-task-item'
import AppIcon from '@/components/common/AppIcon.vue'
import ProjectDetail from '@/views/ProjectDetail.vue'
import AtMentionSelector from '@/components/common/AtMentionSelector.vue'
import CalendarPicker from './CalendarPicker.vue'
import { createProject } from '@/api/db-client'
import { toast } from '@/composables/useToast'
import { ProjectExtension, FileExtension } from '@/components/editor/extensions'
import { useLinkModals } from '@/composables/useLinkModals'

const PRIO = { color: { P1: '#ef4444', P2: '#f59e0b', P3: '#3b82f6' } as Record<string,string> }

interface Todo { id: string; title: string; content: string; done: boolean; createdAt: string; dueDate?: string; priority?: 'P1'|'P2'|'P3'; repeat?: 'daily'|'weekly'|'monthly'; projectId?: string; projectName?: string }
interface Project { id: string; name: string }

const PRIORITY_OPTIONS = [
  { key: 'P1' as const, label: '高', color: 'text-red-500 bg-red-50', dot: '#ef4444' },
  { key: 'P2' as const, label: '中', color: 'text-amber-500 bg-amber-50', dot: '#f59e0b' },
  { key: 'P3' as const, label: '低', color: 'text-blue-500 bg-blue-50', dot: '#3b82f6' },
]

const props = defineProps<{ todo: Todo; projects: Project[]; hideProjectSelector?: boolean }>()
const emit = defineEmits<{
  update: [id: string, data: Partial<Todo>]
  close: []
}>()

const headingLabel = ref('正文')
const headingOpen = ref(false)
const listOpen = ref(false)
const alignOpen = ref(false)
const alignLabel = ref('左对齐')
const tableMenuOpen = ref(false)
const tableRows = ref(3)
const tableCols = ref(3)

// ── Date picker: custom calendar popover ──
const datePickerOpen = ref(false)
const dateInput = ref(props.todo?.dueDate || '')
function setDate(v: string) { dateInput.value = v; emit('update', props.todo.id, { dueDate: v || null }) }
function clearDate() { dateInput.value = ''; emit('update', props.todo.id, { dueDate: null }) }
const displayDate = computed(() => {
  if (!props.todo.dueDate) return ''
  try {
    const d = new Date(props.todo.dueDate)
    if (isNaN(d.getTime())) return ''
    const today = new Date(); today.setHours(0,0,0,0)
    const tomorrow = new Date(today); tomorrow.setDate(tomorrow.getDate()+1)
    const prefix = d.getTime() === today.getTime() ? '今天' : d.getTime() === tomorrow.getTime() ? '明天' : ''
    const datePart = `${d.getFullYear()}年${d.getMonth()+1}月${d.getDate()}日`
    const timePart = props.todo.dueDate?.includes('T') ? ` ${String(d.getHours()).padStart(2,'0')}:${String(d.getMinutes()).padStart(2,'0')}` : ''
    const full = datePart + timePart
    return prefix ? `${prefix} · ${full}` : full
  } catch { return '' }
})
const isOverdue = computed(() => {
  if (!props.todo.dueDate) return false
  const d = new Date(props.todo.dueDate)
  if (isNaN(d.getTime())) return false
  const today = new Date(); today.setHours(0,0,0,0)
  return d.getTime() < today.getTime() && !props.todo.done
})

// ── Priority popover ──
const priorityOpen = ref(false)
function setPriority(p: 'P1'|'P2'|'P3') { emit('update', props.todo.id, { priority: p }); priorityOpen.value = false }
function clearPriority() { emit('update', props.todo.id, { priority: null }); priorityOpen.value = false }
const priorityInfo = computed(() => PRIORITY_OPTIONS.find(o => o.key === props.todo.priority))

// ── Project picker ──
const projectOpen = ref(false)
const projectQuickCreate = ref(false)
const projectSearch = ref('')
const filteredProjects = computed(() => {
  const q = projectSearch.value.toLowerCase().trim()
  return q ? props.projects.filter(p => p.name.toLowerCase().includes(q)) : props.projects
})
// ── New project inline creation ──
const newProjectName = ref('')
const isCreatingProject = ref(false)
async function handleCreateProject() {
  const name = newProjectName.value.trim()
  if (!name || isCreatingProject.value) return
  isCreatingProject.value = true
  try {
    const r: any = await createProject({ name })
    const newId = r?.id
    if (newId) {
      // Inject into parent's projects list so it appears immediately
      const newP = { id: newId, name }
      const idx = ProjectListValues.value?.findIndex((p: Project) => p.id === newId)
      emit('update', props.todo.id, { projectId: newId, projectName: name })
      projectOpen.value = false
      projectSearch.value = ''
      newProjectName.value = ''
      toast({ message: '项目已创建', type: 'success' })
    }
  } catch (e: any) { toast({ message: e.message || '创建失败', type: 'error' }) }
  finally { isCreatingProject.value = false }
}
const ProjectListValues = computed(() => props.projects)

function setProject(p: Project) { emit('update', props.todo.id, { projectId: p.id, projectName: p.name }); projectOpen.value = false; projectSearch.value = '' }
function clearProject() { emit('update', props.todo.id, { projectId: undefined, projectName: undefined }) }
// Navigate to project detail page
function goToProject() {
  if (props.todo.projectId) openProject(props.todo.projectId)
}

// ── @ selector ───────────────────────────────────────────
const atOpen = ref(false)

function openAtSelector() { atOpen.value = true }
function insertProjectLink(p: { id: string; name: string }) {
  if (!editor.value) return
  editor.value.chain().focus().insertContent({ type: 'projectMention', attrs: { projectId: p.id, name: p.name } }).insertContent(' ').run()
  atOpen.value = false
}
function insertFileLink(f: any) {
  if (!editor.value) return
  const fname = f.lib_gn || f.lib_cn || f.proj_gn || f.prod_gn || '未命名'
  editor.value.chain().focus().insertContent({ type: 'fileMention', attrs: { fileId: f.id, name: fname } }).insertContent(' ').run()
  atOpen.value = false
}

const { projectModalId, openProject, closeProject } = useLinkModals()

function insertFileAtCursor(fileId: string, fileName: string) {
  if (!editor.value) return
  editor.value.chain().focus().insertContent({ type: 'fileMention', attrs: { fileId, name: fileName } }).insertContent(' ').run()
}
defineExpose({ insertFileAtCursor })

// Editor
const editor = useEditor({
  content: props.todo.content || '',
  extensions: [
    StarterKit.configure({ heading: { levels: [1, 2, 3] } }), Underline,
    Table.configure({ resizable: true }), TableRow, TableCell, TableHeader,
    TaskList, TaskItem.configure({ nested: true }),
    TextAlign.configure({ types: ['heading', 'paragraph'] }),
    Placeholder.configure({ placeholder: '添加描述...' }),
    ProjectExtension,
    FileExtension,
  ],
  onUpdate: ({ editor: ed }) => { emit('update', props.todo.id, { content: ed.getHTML() }) },
  onSelectionUpdate: ({ editor: ed }) => {
    if (ed.isActive('heading', { level: 1 })) headingLabel.value = '标题 1'
    else if (ed.isActive('heading', { level: 2 })) headingLabel.value = '标题 2'
    else if (ed.isActive('heading', { level: 3 })) headingLabel.value = '标题 3'
    else headingLabel.value = '正文'
    if (ed.isActive({ textAlign: 'right' })) alignLabel.value = '右对齐'
    else if (ed.isActive({ textAlign: 'center' })) alignLabel.value = '居中'
    else alignLabel.value = '左对齐'
  },
  editorProps: { attributes: { class: 'tiptap-editor text-left' } },
})

watch(() => props.todo.id, () => { nextTick(() => editor.value?.commands.setContent(props.todo.content || '')); dateInput.value = props.todo.dueDate || '' })

function execFormat(cmd: string) { if (!editor.value) return; const ed = editor.value.chain().focus(); switch (cmd) { case 'bold': ed.toggleBold().run(); break; case 'italic': ed.toggleItalic().run(); break; case 'underline': ed.toggleUnderline().run(); break; case 'strikeThrough': ed.toggleStrike().run(); break; case 'justifyLeft': ed.setTextAlign('left').run(); break; case 'justifyCenter': ed.setTextAlign('center').run(); break; case 'justifyRight': ed.setTextAlign('right').run(); break; case 'insertHorizontalRule': ed.setHorizontalRule().run(); break; case 'insertTable': ed.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(); break; case 'addColumnBefore': ed.addColumnBefore().run(); break; case 'addColumnAfter': ed.addColumnAfter().run(); break; case 'deleteColumn': ed.deleteColumn().run(); break; case 'addRowBefore': ed.addRowBefore().run(); break; case 'addRowAfter': ed.addRowAfter().run(); break; case 'deleteRow': ed.deleteRow().run(); break; case 'deleteTable': ed.deleteTable().run(); break; } }
function insertHeading(level: number) { editor.value?.chain().focus().toggleHeading({ level: level as 1 | 2 | 3 }).run(); headingOpen.value = false }
function insertList(type: 'ul' | 'ol') { if (!editor.value) return; type === 'ul' ? editor.value.chain().focus().toggleBulletList().run() : editor.value.chain().focus().toggleOrderedList().run() }
function toggleTaskList() { editor.value?.chain().focus().toggleTaskList().run(); listOpen.value = false }
function insertBlock(tag: string) { if (!editor.value) return; if (tag === 'pre') editor.value.chain().focus().toggleCodeBlock().run(); else if (tag === 'blockquote') editor.value.chain().focus().toggleBlockquote().run() }
function execAlign(dir: 'left' | 'center' | 'right') { editor.value?.chain().focus().setTextAlign(dir).run(); alignLabel.value = dir === 'right' ? '右对齐' : dir === 'center' ? '居中' : '左对齐'; alignOpen.value = false }
function insertTableCustom() { const r = Math.max(1, Math.min(10, tableRows.value)); const c = Math.max(1, Math.min(8, tableCols.value)); editor.value?.chain().focus().insertTable({ rows: r, cols: c, withHeaderRow: true }).run(); tableMenuOpen.value = false }
function toggleDone() { emit('update', props.todo.id, { done: !props.todo.done }) }
function closePopups() { headingOpen.value = false; listOpen.value = false; alignOpen.value = false; tableMenuOpen.value = false; datePickerOpen.value = false; priorityOpen.value = false; projectOpen.value = false }
onUnmounted(() => editor.value?.destroy())
</script>

<template>
  <div v-if="todo" class="flex-1 flex flex-col min-w-0 overflow-y-auto" @click="closePopups">
    <!-- Toolbar -->
    <div class="flex items-center gap-0 px-2 h-10 border-b border-surface-100 shrink-0">
      <div class="relative">
        <button @mousedown.prevent @click.stop="headingOpen=!headingOpen; listOpen=false; alignOpen=false" class="flex items-center gap-0.5 w-[54px] h-7 justify-center rounded transition-colors text-xs text-surface-500 hover:bg-surface-100"><span>{{ headingLabel }}</span><svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg></button>
        <div v-if="headingOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[90px]">
          <button @mousedown.prevent="editor?.chain().focus().setParagraph().run(); headingLabel='正文'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50">正文</button>
          <button @mousedown.prevent="insertHeading(1)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 1</button>
          <button @mousedown.prevent="insertHeading(2)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 2</button>
          <button @mousedown.prevent="insertHeading(3)" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 3</button>
        </div>
      </div>
      <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
      <div class="flex items-center gap-0.5">
        <button @mousedown.prevent="execFormat('bold')" class="w-7 h-7 flex items-center justify-center rounded transition-colors" :class="editor?.isActive('bold')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><b class="text-xs">B</b></button>
        <button @mousedown.prevent="execFormat('italic')" class="w-7 h-7 flex items-center justify-center rounded transition-colors" :class="editor?.isActive('italic')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><i class="text-xs">I</i></button>
        <button @mousedown.prevent="execFormat('underline')" class="w-7 h-7 flex items-center justify-center rounded transition-colors" :class="editor?.isActive('underline')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><u class="text-xs">U</u></button>
        <button @mousedown.prevent="execFormat('strikeThrough')" class="w-7 h-7 flex items-center justify-center rounded transition-colors text-xs text-surface-500 hover:bg-surface-100 line-through" :class="editor?.isActive('strike')?'bg-blue-50 text-blue-600':''"><span>S</span></button>
      </div>
      <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
      <div class="relative">
        <button @mousedown.prevent @click.stop="listOpen=!listOpen; headingOpen=false; alignOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors text-xs text-surface-500 hover:bg-surface-100"><AppIcon name="list" size="15"/><svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg></button>
        <div v-if="listOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[110px]"><button @mousedown.prevent="insertList('ul'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-surface-600 hover:bg-surface-50"><AppIcon name="list" size="14"/>无序列表</button><button @mousedown.prevent="insertList('ol'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-surface-600 hover:bg-surface-50"><AppIcon name="list-ol" size="14"/>有序列表</button></div>
      </div>
      <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
      <button @mousedown.prevent="toggleTaskList()" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="editor?.isActive('taskList')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'" title="任务清单">
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.6"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><path d="M8 12l3 3 5-6"/></svg>
        <span class="absolute -top-8 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">任务清单</span>
      </button>
      <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
      <div class="relative">
        <button @mousedown.prevent @click.stop="alignOpen=!alignOpen; headingOpen=false; listOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors text-xs text-surface-500 hover:bg-surface-100"><AppIcon :name="alignLabel==='右对齐'?'align-right':alignLabel==='居中'?'align-center':'align-left'" size="15"/><svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg></button>
        <div v-if="alignOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[120px]"><button @mousedown.prevent="execAlign('left')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-surface-600 hover:bg-surface-50"><AppIcon name="align-left" size="14"/>左对齐</button><button @mousedown.prevent="execAlign('center')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-surface-600 hover:bg-surface-50"><AppIcon name="align-center" size="14"/>居中</button><button @mousedown.prevent="execAlign('right')" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-surface-600 hover:bg-surface-50"><AppIcon name="align-right" size="14"/>右对齐</button></div>
      </div>
      <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
      <div class="flex items-center gap-0.5">
        <button @mousedown.prevent="execFormat('insertHorizontalRule')" class="w-7 h-7 flex items-center justify-center rounded transition-colors text-surface-500 hover:bg-surface-100"><span class="text-xs">—</span></button>
        <button @mousedown.prevent="insertBlock('pre')" class="w-7 h-7 flex items-center justify-center rounded transition-colors text-surface-500 hover:bg-surface-100"><AppIcon name="code-bracket" size="15"/></button>
        <button @mousedown.prevent="insertBlock('blockquote')" class="w-7 h-7 flex items-center justify-center rounded transition-colors text-surface-500 hover:bg-surface-100"><AppIcon name="quote" size="15"/></button>
        <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <button
          data-at-btn
          @mousedown.prevent @click.stop="openAtSelector()"
          class="w-7 h-7 flex items-center justify-center rounded transition-colors text-surface-500 hover:bg-surface-100 relative"
          :class="atOpen ? 'bg-blue-50 text-blue-600' : ''"
          title="插入链接"
        >
          <svg class="w-[15px] h-[15px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 12v1a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
        </button>
        <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
        <div class="relative">
          <button @mousedown.prevent @click.stop="tableMenuOpen=!tableMenuOpen; headingOpen=false; listOpen=false; alignOpen=false" class="w-7 h-7 flex items-center justify-center rounded transition-colors" :class="editor?.isActive('table')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><AppIcon name="table-grid" size="15"/><svg class="w-2 h-2 ml-0.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg></button>
          <div v-if="tableMenuOpen" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-lg shadow-sm border border-surface-100 p-1.5 w-[92px]" @click.stop><template v-if="!editor?.isActive('table')"><div class="space-y-1.5 mb-2"><div class="flex items-center gap-1.5"><span class="text-[11px] text-surface-500 w-4">行</span><input v-model.number="tableRows" type="number" min="1" max="10" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/></div><div class="flex items-center gap-1.5"><span class="text-[11px] text-surface-500 w-4">列</span><input v-model.number="tableCols" type="number" min="1" max="8" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/></div></div><button @click.prevent="insertTableCustom()" class="w-full py-1 text-[10px] bg-brand-600 text-white rounded hover:bg-brand-700 transition-colors flex items-center justify-center gap-1"><svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M12 4v16m8-8H4"/></svg>插入表格</button></template></div>
        </div>
      </div>
    </div>

    <!-- Header: checkbox + title -->
    <div class="flex items-start gap-4 px-4 pt-6 pb-2 shrink-0">
      <button @click="toggleDone" class="w-7 h-7 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-200 hover:shadow-md mt-0.5" :style="todo.done ? { borderColor: todo.priority ? PRIO.color[todo.priority] : '#9ca3af', background: todo.priority ? PRIO.color[todo.priority] : '#9ca3af' } : { borderColor: todo.priority ? PRIO.color[todo.priority] : '#d1d5db' }">
        <svg v-if="todo.done" class="w-4 h-4 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
      </button>
      <div class="flex-1">
        <input :value="todo.title" @input="emit('update', todo.id, { title: ($event.target as HTMLInputElement).value })" class="w-full text-xl font-bold text-surface-800 bg-transparent outline-none placeholder:text-surface-300" placeholder="待办标题"/>
        <!-- Metadata: project (row 1) -->
        <div class="flex items-center gap-2 mt-2">
          <!-- Project -->
          <div class="relative inline-flex">
            <template v-if="props.hideProjectSelector">
              <span class="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md border bg-surface-100 text-surface-600 border-surface-200">
                <AppIcon name="folder-open" size="13" stroke="1.5"/><span>{{ todo.projectName || '项目' }}</span>
              </span>
            </template>
            <template v-else>
            <button
              @click.stop="projectOpen=!projectOpen; priorityOpen=false; datePickerOpen=false; projectSearch=''; projectQuickCreate=false; newProjectName=''"
              class="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md transition-colors border"
              :class="todo.projectId ? 'bg-surface-100 text-surface-600 border-surface-200' : 'bg-surface-100 text-surface-400 border-surface-200 hover:bg-surface-200 hover:text-surface-600'"
            >
              <AppIcon name="folder-open" size="13" stroke="1.5"/><span>{{ todo.projectName || '项目' }}</span>
              <button v-if="todo.projectId" @click.stop="goToProject" class="inline-flex items-center justify-center text-surface-400 hover:text-brand-600 ml-0.5 relative z-20 transition-colors" title="跳转到项目"><AppIcon name="arrow-up-right" size="9" stroke="2.5"/></button>
              <button v-if="todo.projectId" @click.stop="clearProject" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-surface-400 hover:text-red-400 ml-0.5 relative z-20"><svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 18L18 6M6 6l12 12"/></svg></button>
            </button>
            <div v-if="projectOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 w-[220px]" @click.stop>
              <div class="p-2 border-b border-surface-100">
                <input v-model="projectSearch" type="text" class="w-full border border-surface-200 rounded px-2 py-1 text-[11px] outline-none focus:border-brand-400" placeholder="搜索项目..." @keydown.escape="projectOpen=false"/>
              </div>
              <div class="max-h-[220px] overflow-y-auto py-1">
                <button v-for="p in filteredProjects" :key="p.id" @click="setProject(p)" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left transition-colors" :class="todo.projectId===p.id ? 'bg-blue-50 text-blue-600 font-medium' : 'text-surface-600 hover:bg-surface-50'"><AppIcon name="folder-open" size="12" class="text-surface-400"/><span class="truncate">{{ p.name }}</span></button>
                <div v-if="filteredProjects.length === 0" class="px-3 py-2 text-[11px] text-surface-400 text-center">无匹配项目</div>
              </div>
              <div class="border-t border-surface-100 p-2">
                <div v-if="projectQuickCreate" class="flex items-center gap-1.5">
                  <input v-model="newProjectName" type="text" class="flex-1 border border-surface-200 rounded px-2 py-1 text-[11px] outline-none focus:border-brand-400" placeholder="项目名称" @keyup.enter="handleCreateProject" @keyup.escape="projectQuickCreate=false; newProjectName=''"/>
                  <button @click="handleCreateProject" :disabled="isCreatingProject || !newProjectName.trim()" class="px-2.5 py-1 text-[10px] bg-brand-600 text-white rounded hover:bg-brand-700 disabled:opacity-40 transition-colors flex-shrink-0">{{ isCreatingProject ? '...' : '创建' }}</button>
                </div>
                <button v-else @click="projectQuickCreate=true" class="w-full flex items-center gap-1.5 text-[11px] text-surface-500 hover:text-brand-600 transition-colors">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>新建项目
                </button>
              </div>
            </div>
            </template>
          </div>
        </div>
        <!-- Metadata: date + priority (row 2) -->
        <div class="flex items-center gap-2 mt-1.5">

          <!-- Due date -->
          <div class="relative inline-flex">
            <button
              @click.stop="datePickerOpen=!datePickerOpen; priorityOpen=false"
              class="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md transition-colors border"
              :class="todo.dueDate ? (isOverdue ? 'bg-red-50 text-red-500 border-red-200' : 'bg-surface-100 text-surface-600 border-surface-200') : 'bg-surface-100 text-surface-400 border-surface-200 hover:bg-surface-200 hover:text-surface-600'"
            >
              <svg class="w-3 h-3 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg><span>{{ displayDate || '日期' }}</span>
              <span v-if="todo.repeat" class="ml-1 inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[9px] font-medium"
                :class="todo.repeat==='daily'?'bg-blue-50 text-blue-600':todo.repeat==='weekly'?'bg-purple-50 text-purple-600':'bg-green-50 text-green-600'">
                <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
                <span>{{ todo.repeat==='daily'?'每天':todo.repeat==='weekly'?'每周':'每月' }}</span>
              </span>
              <button v-if="todo.dueDate" @click.stop="clearDate" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-surface-400 hover:text-red-400 ml-0.5 relative z-20"><svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 18L18 6M6 6l12 12"/></svg></button>
            </button>
            <div v-if="datePickerOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100" @click.stop>
              <CalendarPicker
                :modelValue="dateInput"
                :repeatValue="todo.repeat"
                @update:modelValue="setDate"
                @update:repeatValue="(v: string) => emit('update', todo.id, { repeat: v || null })"
                @close="datePickerOpen = false"
              />
            </div>
          </div>

          <!-- Priority -->
          <div class="relative inline-flex">
            <button
              @click.stop="priorityOpen=!priorityOpen"
              class="inline-flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md transition-colors border"
              :class="priorityInfo ? [priorityInfo.color, 'border-current/20'] : 'bg-surface-100 text-surface-400 border-surface-200 hover:bg-surface-200 hover:text-surface-600'"
            >
              <AppIcon name="flag" size="13" stroke="1.5"/><span>{{ priorityInfo ? priorityInfo.label : '优先级' }}</span>
              <button v-if="todo.priority" @click.stop="clearPriority" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-surface-400 hover:text-red-400 ml-0.5 relative z-20"><svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 18L18 6M6 6l12 12"/></svg></button>
            </button>
            <div v-if="priorityOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[110px]" @click.stop>
              <button v-for="opt in PRIORITY_OPTIONS" :key="opt.key" @click="setPriority(opt.key)" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left transition-colors" :class="todo.priority===opt.key ? 'bg-blue-50 text-blue-600 font-medium' : 'text-surface-600 hover:bg-surface-50'"><span class="w-2 h-2 rounded-full flex-shrink-0" :style="{background: opt.dot}"></span><span>{{ opt.label }}</span></button>
              <hr class="border-surface-100 my-0.5"/>
              <button @click="clearPriority" class="w-full px-3 py-1.5 text-xs text-left text-surface-400 hover:bg-surface-50">无</button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <hr class="mx-4 border-surface-100 mt-4 mb-2" />

    <div class="flex-1 overflow-y-auto px-4">
      <EditorContent :editor="editor" />
    </div>

  </div>

  <AtMentionSelector
    v-if="atOpen"
    :projects="props.projects"
    @select-project="insertProjectLink($event)"
    @select-file="insertFileLink($event)"
    @close="atOpen = false"
  />

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

</template>

<style>
.tiptap-editor { min-height: 100%; padding: 0 0 16px; font-size: 14px; line-height: 1.55; color: #334155; outline: none; }
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
.tiptap-editor ul { list-style-type: disc; padding-left: 1.5em; margin: 0.5em 0; }
.tiptap-editor ol { list-style-type: decimal; padding-left: 1.5em; margin: 0.5em 0; }
.tiptap-editor li { margin: 0.2em 0; }
.tiptap-editor li p { margin: 0; }
.tiptap-editor blockquote { border-left: 3px solid #e2e8f0; padding-left: 1em; margin: 0.5em 0; color: #64748b; }
.tiptap-editor pre { background: #f1f5f9; border-radius: 6px; padding: 12px 16px; font-family: ui-monospace, monospace; font-size: 13px; margin: 0.5em 0; overflow-x: auto; }
.tiptap-editor code { background: #f1f5f9; border-radius: 4px; padding: 1px 5px; font-size: 0.9em; }
.tiptap-editor pre code { background: none; padding: 0; }
.tiptap-editor table { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
.tiptap-editor table td, .tiptap-editor table th { border: 1px solid #e2e8f0; padding: 6px 10px; font-size: 13px; min-width: 60px; }
.tiptap-editor table th { background: #f8fafc; font-weight: 600; }
.no-spinner::-webkit-outer-spin-button,.no-spinner::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.no-spinner { -moz-appearance: textfield; }

.project-link, .file-link { display: inline-flex; align-items: center; gap: 2px; padding: 1px 6px; border-radius: 4px; font-size: 13px; cursor: pointer; white-space: nowrap; transition: background 0.15s; }
.project-link { background: #eff6ff; border: 1px solid #bfdbfe; color: #1d4ed8; }
.project-link:hover { background: #dbeafe; }
.file-link { background: #f0fdf4; border: 1px solid #bbf7d0; color: #15803d; }
.file-link:hover { background: #dcfce7; }
.link-svg { flex-shrink: 0; display: inline; }
</style>
