<script setup lang="ts">
import { ref, computed, watch, nextTick, onActivated, onUnmounted } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import { getTiptapExtensions } from '@/composables/useTiptapConfig'
import { useProjectStore } from '@/stores/project'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { formatDateTime, nowLocalDb, tsMillis } from '@/utils/time'
import { fetchProjectNotes, createNote, updateNote, deleteNote, fetchProjects, notesV2Get } from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import AtMentionSelector from '@/components/common/AtMentionSelector.vue'
import ProjectFilePanel from '@/components/project/ProjectFilePanel.vue'

const props = defineProps<{ projectId: string }>()
const emit = defineEmits<{ update: [] }>()

const projectStore = useProjectStore()

const projectNotes = ref<any[]>([])
const selectedNoteId = ref<string | null>(null)
const selectedNote = computed(() => projectNotes.value.find(n => n.note_id === selectedNoteId.value))
// 按更新时间降序排序，确保自动保存后笔记实时重排到顶部
const sortedProjectNotes = computed(() =>
  [...projectNotes.value].sort((a, b) => tsMillis(b.updated_at || b.created_at) - tsMillis(a.updated_at || a.created_at))
)
const editingNoteTitle = ref('')
const lastSavedContent = ref('')

onActivated(async () => {
  await loadProjectNotes()
})

watch(selectedNote, (note) => { editingNoteTitle.value = note?.title || '' })

async function saveNoteTitle() {
  if (!selectedNote.value || editingNoteTitle.value === selectedNote.value.title) return
  try {
    await updateNote({ noteId: selectedNote.value.note_id, title: editingNoteTitle.value })
    await loadProjectNotes()
  } catch (e) { console.warn('保存笔记标题失败', e) }
}

async function loadProjectNotes() {
  try {
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    const r = await fetchProjectNotes(backendId)
    projectNotes.value = Array.isArray(r) ? r : (r?.notes || [])
  } catch { projectNotes.value = [] }
}

async function addProjectNote() {
  try {
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    const r: any = await createNote({ title: '', content: '', isDaily: false, projectIds: [backendId] })
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: '', excerpt: '',
      updated_at: nowLocalDb(), created_at: nowLocalDb() }
    projectNotes.value.unshift(newNote)
    selectedNoteId.value = newNote.note_id
    editingNoteTitle.value = ''
  } catch (e) {
    console.warn('创建笔记失败', e)
    toast({ message: '创建笔记失败', type: 'error' })
  }
}

async function removeProjectNote(noteId: string) {
  const ok = await dangerConfirm({ title: '删除笔记', message: '确定删除此笔记？将移入回收站。', confirmText: '删除' })
  if (!ok) return
  try {
    await deleteNote(noteId)
    await loadProjectNotes()
    toast({ message: '已移至回收站', type: 'success' })
  } catch (e) { console.warn('删除笔记失败', e) }
}

const headingOpen = ref(false)
const headingLabel = ref('正文')
const listOpen = ref(false)
const alignOpen = ref(false)
const alignLabel = ref('左对齐')
const tableMenuOpen = ref(false)
const tableRows = ref(3)
const tableCols = ref(3)
const atOpen = ref(false)
const allProjects = ref<any[]>([])

function openAtSelector() { atOpen.value = true }

watch(atOpen, (v) => { if (v) loadAllProjects() })

function insertProjectLink(p: any) {
  if (!noteEditor.value) return
  noteEditor.value.chain().focus().insertContent({
    type: 'projectMention',
    attrs: { projectId: p.id, name: p.name },
  }).insertContent(' ').run()
  atOpen.value = false
}

function insertFileLink(f: any) {
  if (!noteEditor.value) return
  const fname = f.lib_gn || f.lib_cn || f.proj_gn || f.prod_gn || '未命名'
  const fileId = f.id
  noteEditor.value.chain().focus().insertContent({
    type: 'fileMention',
    attrs: { fileId, name: fname },
  }).insertContent(' ').run()
  atOpen.value = false
}

async function loadAllProjects() {
  try {
    const raw = await fetchProjects()
    allProjects.value = Array.isArray(raw) ? raw : []
  } catch { allProjects.value = [] }
}

const noteEditor = useEditor({
  content: '',
  extensions: getTiptapExtensions({ placeholder: '开始记录...' }),
  onUpdate: ({ editor: ed }) => {
    if (selectedNote.value && debouncedNoteSave) debouncedNoteSave(ed.getHTML())
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
  editorProps: { attributes: { class: 'tiptap-editor' } },
})

let noteSaveTimer: any = null
function debouncedNoteSave(html: string) {
  if (noteSaveTimer) clearTimeout(noteSaveTimer)
  noteSaveTimer = setTimeout(() => saveNoteContent(html), 3000)
}

watch(selectedNote, async (note) => {
  // C1 修复：切换笔记前先落盘旧笔记的待保存内容，避免防抖超时后写入新笔记的 note_id
  if (noteSaveTimer) { clearTimeout(noteSaveTimer); noteSaveTimer = null; saveNoteContent() }
  editingNoteTitle.value = note?.title || ''
  if (!note) return
  try {
    const targetId = note.note_id
    const r = await notesV2Get(targetId)
    // 防止快速切换时旧响应覆盖新选中的笔记
    if (selectedNote.value?.note_id !== targetId) return
    const fullContent = r?.note?.content || ''
    lastSavedContent.value = fullContent
    nextTick(() => { noteEditor.value?.commands.setContent(fullContent, false) })
  } catch { nextTick(() => { noteEditor.value?.commands.setContent('', false) }) }
})

async function saveNoteContent(html?: string) {
  const content = html ?? noteEditor.value?.getHTML() ?? ''
  if (!selectedNote.value || content === lastSavedContent.value) return
  try {
    await updateNote({ noteId: selectedNote.value.note_id, content })
    lastSavedContent.value = content
    // Update local excerpt — Object.assign 原地修改，避免 spread 创建新对象触发 selectedNote watcher 重置编辑器光标
    const idx = projectNotes.value.findIndex(n => n.note_id === selectedNote.value?.note_id)
    if (idx !== -1) {
      Object.assign(projectNotes.value[idx], { excerpt: content.replace(/<[^>]*>/g,'').slice(0,300), updated_at: nowLocalDb() })
    }
  } catch (e) { console.warn('保存笔记内容失败', e) }
}

function noteExecFormat(cmd: string, val?: string) {
  if (!noteEditor.value) return
  const ed = noteEditor.value.chain().focus()
  switch (cmd) {
    case 'bold': ed.toggleBold().run(); break
    case 'italic': ed.toggleItalic().run(); break
    case 'underline': ed.toggleUnderline().run(); break
    case 'strike': ed.toggleStrike().run(); break
    case 'bulletList': ed.toggleBulletList().run(); break
    case 'orderedList': ed.toggleOrderedList().run(); break
    case 'taskList': ed.toggleTaskList().run(); break
    case 'blockquote': ed.toggleBlockquote().run(); break
    case 'codeBlock': ed.toggleCodeBlock().run(); break
    case 'horizontalRule': ed.setHorizontalRule().run(); break
    case 'formatBlock': if (val === 'p') ed.setParagraph().run(); break
    case 'heading': if (val) ed.toggleHeading({ level: val as any }).run(); break
    case 'align': if (val) ed.setTextAlign(val as any).run(); alignLabel.value = val === 'right' ? '右对齐' : val === 'center' ? '居中' : '左对齐'; break
    case 'insertTable': ed.insertTable({ rows: tableRows.value, cols: tableCols.value, withHeaderRow: true }).run(); tableMenuOpen.value = false; break
    case 'addColumnBefore': ed.addColumnBefore().run(); break
    case 'addColumnAfter': ed.addColumnAfter().run(); break
    case 'deleteColumn': ed.deleteColumn().run(); break
    case 'addRowBefore': ed.addRowBefore().run(); break
    case 'addRowAfter': ed.addRowAfter().run(); break
    case 'deleteRow': ed.deleteRow().run(); break
    case 'deleteTable': ed.deleteTable().run(); tableMenuOpen.value = false; break
  }
}

function closeNotePopups() { headingOpen.value = false; listOpen.value = false; alignOpen.value = false; tableMenuOpen.value = false }

function handleFilePanelInsert(f: any) {
  if (!noteEditor.value) return
  const fname = f.custom_name || f.displayName || f.name || f.lib_cn || f.lib_gn || '未命名'
  noteEditor.value.chain().focus().insertContent({
    type: 'fileMention',
    attrs: { fileId: f.id, name: fname },
  }).insertContent(' ').run()
}

const filePanelRef = ref<InstanceType<typeof ProjectFilePanel> | null>(null)
watch(selectedNote, () => {
  if (selectedNote.value) filePanelRef.value?.loadFiles()
})

onUnmounted(() => { document.removeEventListener('click', closeNotePopups) })

defineExpose({ closeNotePopups })
</script>

<template>
  <div class="h-full flex overflow-hidden bg-white">
    <div class="w-[280px] flex-shrink-0 border-r border-surface-200 flex flex-col bg-white/50">
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">笔记</span>
        <button @click="addProjectNote" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建笔记"><AppIcon name="plus" size="12" stroke="2.5"/></button>
      </div>
      <div class="flex-1 overflow-y-auto px-2 py-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
        <div v-for="note in sortedProjectNotes" :key="note.note_id" @click="selectedNoteId = note.note_id"
          class="flex items-center px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden hover:scale-[1.02] hover:z-10 select-none mb-1"
          :class="selectedNoteId === note.note_id ? 'bg-blue-50/70 border-brand-200 shadow-none scale-100' : 'border-surface-100 hover:border-surface-300 hover:shadow-md'">
          <div v-if="selectedNoteId === note.note_id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
          <div class="flex-1 min-w-0">
            <p class="text-[12px] font-medium text-surface-800 truncate">{{ note.title || '未命名笔记' }}</p>
            <p class="text-[10px] text-surface-400 line-clamp-1 mt-0.5">{{ (note.excerpt || '').replace(/<[^>]*>/g,'').slice(0,40) || '暂无内容' }}</p>
            <p class="text-[9px] text-surface-300 mt-0.5">{{ formatDateTime(note.updated_at || note.created_at || '') }}</p>
          </div>
          <button @click.stop="removeProjectNote(note.note_id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0"><AppIcon name="trash" size="13"/></button>
        </div>
        <div v-if="sortedProjectNotes.length === 0" class="text-center py-10 text-surface-400 text-xs">暂无笔记</div>
      </div>
    </div>

    <div class="flex-1 flex min-w-0" v-if="selectedNote">
      <div class="flex-1 flex flex-col bg-white min-w-0">
        <div class="flex items-center gap-0 px-2 h-10 border-b border-surface-100 shrink-0" @click="closeNotePopups">
          <div class="relative">
            <button @mousedown.prevent @click.stop="headingOpen=!headingOpen; listOpen=false; alignOpen=false" class="flex items-center gap-0.5 w-[54px] h-7 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
              <span class="truncate">{{ headingLabel }}</span>
              <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
            </button>
            <div v-if="headingOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[90px]">
              <button @mousedown.prevent="noteExecFormat('heading', 1); headingLabel='标题 1'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 1</button>
              <button @mousedown.prevent="noteExecFormat('heading', 2); headingLabel='标题 2'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 2</button>
              <button @mousedown.prevent="noteExecFormat('heading', 3); headingLabel='标题 3'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left font-bold text-surface-600 hover:bg-surface-50">标题 3</button>
              <button @mousedown.prevent="noteExecFormat('formatBlock','p'); headingLabel='正文'; headingOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50">正文</button>
            </div>
          </div>
          <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="flex items-center gap-0.5">
            <button @mousedown.prevent="noteExecFormat('bold')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('bold')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><b class="text-xs">B</b></button>
            <button @mousedown.prevent="noteExecFormat('italic')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('italic')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><i class="text-xs">I</i></button>
            <button @mousedown.prevent="noteExecFormat('underline')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('underline')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'"><u class="text-xs">U</u></button>
            <button @mousedown.prevent="noteExecFormat('strike')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100 line-through relative group" :class="noteEditor?.isActive('strike')?'bg-blue-50 text-blue-600':''"><span>S</span></button>
          </div>
          <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="relative">
            <button @mousedown.prevent @click.stop="listOpen=!listOpen; headingOpen=false; alignOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
              <AppIcon name="list" size="15"/>
              <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
            </button>
            <div v-if="listOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[110px]">
              <button @mousedown.prevent="noteExecFormat('bulletList'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list" size="14"/>无序列表</button>
              <button @mousedown.prevent="noteExecFormat('orderedList'); listOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list-ol" size="14"/>有序列表</button>
            </div>
          </div>
          <button @mousedown.prevent="noteExecFormat('taskList')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('taskList')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'" title="任务清单">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><path d="M8 12l3 3 5-6"/></svg>
          </button>
          <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="relative">
            <button @mousedown.prevent @click.stop="alignOpen=!alignOpen; headingOpen=false; listOpen=false" class="flex items-center gap-0.5 h-7 px-1.5 justify-center rounded transition-colors duration-200 text-xs text-surface-500 hover:bg-surface-100">
              <AppIcon :name="alignLabel==='右对齐'?'align-right':alignLabel==='居中'?'align-center':'align-left'" size="15"/>
              <svg class="w-2.5 h-2.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
            </button>
            <div v-if="alignOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[120px]">
              <button @mousedown.prevent="noteExecFormat('align','left'); alignOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-left" size="14"/>左对齐</button>
              <button @mousedown.prevent="noteExecFormat('align','center'); alignOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-center" size="14"/>居中</button>
              <button @mousedown.prevent="noteExecFormat('align','right'); alignOpen=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-right" size="14"/>右对齐</button>
            </div>
          </div>
          <div class="mx-1.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <button @mousedown.prevent="noteExecFormat('horizontalRule')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100"><span class="text-xs">—</span></button>
          <button @mousedown.prevent="noteExecFormat('codeBlock')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100"><AppIcon name="code-bracket" size="15"/></button>
          <button @mousedown.prevent="noteExecFormat('blockquote')" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100"><AppIcon name="quote" size="15"/></button>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="relative">
            <button data-at-btn @mousedown.prevent @click.stop="openAtSelector()" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100" :class="atOpen?'bg-blue-50 text-blue-600':''" title="插入链接">
              <svg class="w-[15px] h-[15px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 12v1a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
            </button>
          </div>
          <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
          <div class="relative">
            <button @mousedown.prevent @click.stop="tableMenuOpen=!tableMenuOpen; headingOpen=false; listOpen=false; alignOpen=false; atOpen=false" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('table')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'">
              <AppIcon name="table-grid" size="15"/>
              <svg class="w-2 h-2 ml-0.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
            </button>
            <div v-if="tableMenuOpen" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-lg shadow-sm border border-surface-100 p-1 w-[120px]" @click.stop>
              <div v-if="!noteEditor?.isActive('table')" class="space-y-1.5 mb-2">
                <div class="flex items-center gap-1.5">
                  <span class="text-[11px] text-surface-500 w-4">行</span>
                  <input v-model.number="tableRows" type="number" min="1" max="10" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner"/>
                </div>
                <div class="flex items-center gap-1.5">
                  <span class="text-[11px] text-surface-500 w-4">列</span>
                  <input v-model.number="tableCols" type="number" min="1" max="8" class="flex-1 border border-surface-200 rounded px-1.5 py-0.5 text-[11px] text-center outline-none focus:border-brand-400 no-spinner"/>
                </div>
              </div>
              <button v-if="!noteEditor?.isActive('table')" @mousedown.prevent="noteExecFormat('insertTable')" class="w-full py-1 text-[10px] bg-blue-600 text-white rounded hover:bg-blue-700 transition-colors flex items-center justify-center gap-1">+ 插入表格</button>
              <template v-if="noteEditor?.isActive('table')">
                <hr class="border-surface-100 my-2" />
                <button @mousedown.prevent="noteExecFormat('addColumnBefore'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">左侧插入列</button>
                <button @mousedown.prevent="noteExecFormat('addColumnAfter'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">右侧插入列</button>
                <button @mousedown.prevent="noteExecFormat('deleteColumn'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">删除列</button>
                <hr class="border-surface-100 my-1" />
                <button @mousedown.prevent="noteExecFormat('addRowBefore'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">上方插入行</button>
                <button @mousedown.prevent="noteExecFormat('addRowAfter'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">下方插入行</button>
                <button @mousedown.prevent="noteExecFormat('deleteRow'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 rounded">删除行</button>
                <hr class="border-surface-100 my-1" />
                <button @mousedown.prevent="noteExecFormat('deleteTable'); tableMenuOpen=false" class="w-full px-3 py-1.5 text-xs text-left text-red-500 hover:bg-red-50 rounded">删除表格</button>
              </template>
            </div>
          </div>
        </div>
        <div class="px-4 pt-3 pb-1 shrink-0">
          <input v-model="editingNoteTitle" @blur="saveNoteTitle" @keyup.enter="saveNoteTitle" class="w-full text-lg font-semibold text-surface-800 bg-transparent outline-none" placeholder="笔记标题" />
          <hr class="border-surface-200 mt-1.5" />
        </div>
        <div class="flex-1 overflow-y-auto min-h-0 px-4 py-2 prose prose-sm max-w-none">
          <EditorContent :editor="noteEditor" />
        </div>
      </div>
      <ProjectFilePanel ref="filePanelRef" :project-id="projectId" @insert-file="handleFilePanelInsert" />
    </div>
    <div v-else class="flex-1 flex items-center justify-center text-surface-300 text-xs bg-white min-w-0">从左侧选择一个笔记</div>
    <AtMentionSelector v-if="atOpen" :projects="allProjects" @select-project="insertProjectLink($event)" @select-file="insertFileLink($event)" @close="atOpen = false" />
  </div>
</template>
