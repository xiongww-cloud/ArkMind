<script setup lang="ts">
import { ref, computed, watch, nextTick, onActivated } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import { getTiptapExtensions } from '@/composables/useTiptapConfig'
import { useProjectStore } from '@/stores/project'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { formatDateTime, nowLocalDb, tsMillis } from '@/utils/time'
import { richContentToHtml } from '@/utils/rtContent'
import { fetchProjectNotes, createNote, updateNote, deleteNote, fetchProjects, notesV2Get } from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import RichTextToolbar from '@/components/common/RichTextToolbar.vue'
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

// 子 tab 用 v-if 切换会销毁重建组件，onActivated 不触发，需挂载即加载
watch(() => props.projectId, () => { loadProjectNotes() }, { immediate: true })

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
    const fullContent = await richContentToHtml(r?.note?.content || '')
    // 防止快速切换时旧响应覆盖新选中的笔记
    if (selectedNote.value?.note_id !== targetId) return
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
  } catch (e: any) {
    // 自动保存失败需可见：lastSavedContent 未更新，下次编辑会再试；提示用户勿立即关页
    console.warn('保存笔记内容失败', e)
    toast({ message: `项目笔记保存失败：${e?.message || e}（请重试或手动复制备份）`, type: 'error' })
  }
}

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
</script>

<template>
  <div class="h-full flex overflow-hidden bg-white">
    <div class="w-[280px] flex-shrink-0 border-r border-surface-200 flex flex-col bg-white/50">
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">笔记</span>
        <button @click="addProjectNote" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" v-tooltip="'新建笔记'"><AppIcon name="plus" size="12" stroke="2.5"/></button>
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
        <RichTextToolbar :editor="noteEditor" :at-active="atOpen" @at="openAtSelector" />
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

<style>
.tiptap-editor a { color: #3b82f6; text-decoration: underline; }
.tiptap-editor .time-tag {
  background: #f1f5f9;
  border-radius: 5px;
  padding: 1px 7px;
  font-size: 12px;
  color: #64748b;
  white-space: nowrap;
}
</style>
