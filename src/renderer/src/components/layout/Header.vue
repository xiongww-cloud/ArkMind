<script setup lang="ts">
import { computed, ref, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useProjectStore } from '@/stores/project'
import { invoke } from '@tauri-apps/api/core'
import AppIcon from '@/components/common/AppIcon.vue'
import { useSelectionStore } from '@/stores/selection'
import { useNavigation } from '@/composables/useNavigation'
import { notesV2Create, notesV2Update, notesV2ListDaily, notesV2Delete, fetchTodos, kbMemoryObjects, searchBusiness, searchSourceData, lookupObjectNames } from '@/api/db-client'
import { toast } from '@/composables/useToast'

defineProps({})

const emit = defineEmits(['toggle-sidebar'])

const route = useRoute()
const router = useRouter()
const projectStore = useProjectStore()
const selectionStore = useSelectionStore()
const { navigateTo } = useNavigation()

// ═══════════════════════════════════════════════════════════════════
// Platform detection
// ═══════════════════════════════════════════════════════════════════
const isMac = typeof navigator !== 'undefined' && (
  navigator.platform.toUpperCase().indexOf('MAC') >= 0 ||
  navigator.userAgent.toUpperCase().indexOf('MAC') >= 0
)

// ═══════════════════════════════════════════════════════════════════
// Window controls
// ═══════════════════════════════════════════════════════════════════
async function minimizeWindow() { await invoke('plugin:window|minimize') }
async function toggleMaximize() {
  const isMax = await invoke<boolean>('plugin:window|is_maximized')
  if (isMax) { await invoke('plugin:window|unmaximize') }
  else { await invoke('plugin:window|maximize') }
}
async function closeWindow() { await invoke('plugin:window|close') }

// ═══════════════════════════════════════════════════════════════════
// Page title
// ═══════════════════════════════════════════════════════════════════
const pageTitle = computed(() => {
  if (route.name === 'ProjectDetail') {
    const project = projectStore.getProjectById(route.params.id as string)
    return `项目 / ${project?.name || '...'}`
  }
  const meta = route.meta as Record<string, string>
  if (meta.parent) return `${meta.parent} / ${meta.title}`
  const title = meta.title || '总览'
  return title.replace(/库$/, '')
})

// ═══════════════════════════════════════════════════════════════════
// Global search
// ═══════════════════════════════════════════════════════════════════
const searchQuery = ref('')
const searchInputRef = ref<HTMLInputElement | null>(null)
const searchResults = ref<Array<{source_type:string;source_id:string;title:string}>>([])
const searchOpen = ref(false)
const searchLoading = ref(false)
const searchFilter = ref<string | null>(null)  // null=全部, 或 type 值
let searchTimer: ReturnType<typeof setTimeout> | null = null

const filteredResults = computed(() => {
  if (!searchFilter.value) return searchResults.value
  return searchResults.value.filter(r => r.source_type === searchFilter.value)
})

function onSearchInput() {
  if (searchTimer) clearTimeout(searchTimer)
  const q = searchQuery.value.trim()
  if (!q) { searchResults.value = []; searchOpen.value = false; return }
  searchLoading.value = true
  searchOpen.value = true
  searchTimer = setTimeout(async () => {
    try {
      const [business, sourceData] = await Promise.all([
        searchBusiness(q),
        searchSourceData(q)
      ])
      const seen = new Set<string>()
      const merged: any[] = []
      // 业务表结果优先（带有 title）
      for (const obj of (business?.objects || [])) {
        const key = `${obj.source_type}-${obj.source_id}`
        if (seen.has(key)) continue
        seen.add(key)
        merged.push(obj)
      }
      // source_data 结果补充（去重，需要查名称）
      const lookupIds: Array<{source_type:string;source_id:string}> = []
      for (const obj of (sourceData?.objects || [])) {
        const key = `${obj.source_type}-${obj.source_id}`
        if (seen.has(key)) continue
        seen.add(key)
        merged.push(obj)
        lookupIds.push({ source_type: obj.source_type, source_id: obj.source_id })
      }
      // 批量查询业务表名称
      if (lookupIds.length > 0) {
        try {
          const { names } = await lookupObjectNames(lookupIds)
          for (const obj of merged) {
            if (!obj.title) {
              const key = `${obj.source_type}-${obj.source_id}`
              obj.title = names?.[key] || ''
            }
          }
        } catch {}
      }
      searchResults.value = merged.slice(0, 30)
    } catch { searchResults.value = [] }
    finally { searchLoading.value = false }
  }, 300)
}

function closeSearch() {
  searchFilter.value = null
  setTimeout(() => { searchOpen.value = false }, 200)
}

function selectSearchResult(obj: {source_type:string;source_id:string;title:string;project_id?:string;project_name?:string}) {
  searchOpen.value = false
  searchInputRef.value?.blur()
  if (obj.project_id) {
    const map: Record<string, import('@/stores/selection').SelectionType> = {
      PROJECT: 'project', PRODUCT: 'product', NOTE: 'note', TODO: 'todo',
    }
    const t = map[obj.source_type]
    const tab = t === 'note' ? 'notes' as const : t === 'todo' ? 'todos' as const : 'overview' as const
    selectionStore.jumpTo('project', obj.project_id, { tab, targetType: t, targetId: obj.source_id })
    router.push(`/projects/${obj.project_id}`)
    return
  }
  const map: Record<string, import('@/stores/selection').SelectionType> = {
    PROJECT: 'project', PRODUCT: 'product', NOTE: 'note', TODO: 'todo',
  }
  const t = map[obj.source_type]
  if (t) { navigateTo(t, obj.source_id) }
  else if (obj.source_type === 'FILE') { invoke('file_open', { versionId: obj.source_id }).catch(() => {}) }
}

const typeLabelMap: Record<string,string> = { FILE:'文件', NOTE:'笔记', PRODUCT:'产品', PROJECT:'项目', TODO:'待办' }
const typeBadgeMap: Record<string,string> = {
  FILE:'bg-blue-50 text-blue-600', NOTE:'bg-amber-50 text-amber-600',
  PRODUCT:'bg-emerald-50 text-emerald-600', PROJECT:'bg-violet-50 text-violet-600',
  TODO:'bg-slate-100 text-slate-500',
}

// ═══════════════════════════════════════════════════════════════════
// Quick todo
// ═══════════════════════════════════════════════════════════════════
const quickTodoOpen = ref(false)
const quickTodoList = ref<any[]>([])
const pendingTodoCount = ref(0)
const todoPreview = ref<any>(null)

function showTodoPreview(t: any) { todoPreview.value = t }

function goToTodos() {
  quickTodoOpen.value = false
  router.push('/todos?create=true')
}

async function loadQuickTodos() {
  try {
    const r = await fetchTodos()
    const all = (r?.todos || []).map((t: any) => ({ ...t, done: t.done === 1 || t.done === true }))
    // Auto-uncomplete overdue repeating todos
    const now = new Date()
    for (const t of all) {
      if (t.done && t.repeat && t.due_date) {
        const due = new Date(t.due_date)
        if (!isNaN(due.getTime()) && due <= now) {
          try {
            const { updateTodo } = await import('@/api/db-client')
            await updateTodo(t.id, { done: false })
            t.done = false
          } catch (e) { console.error('[Header] toggleTodo failed:', e) }
        }
      }
    }
    // 只显示未完成的待办
    const undone = all.filter((t: any) => !t.done)
    // 排序：有日期的按日期升序（越近越靠前），无日期的按优先级 P1→P2→P3→无
    const priorityRank: Record<string, number> = { 'P1': 1, 'P2': 2, 'P3': 3 }
    const withDate = undone.filter((t: any) => t.due_date).sort((a: any, b: any) => a.due_date.localeCompare(b.due_date))
    const withoutDate = undone.filter((t: any) => !t.due_date).sort((a: any, b: any) => (priorityRank[a.priority] || 999) - (priorityRank[b.priority] || 999))
    quickTodoList.value = [...withDate, ...withoutDate]
    pendingTodoCount.value = all.filter((t: any) => !t.done).length
  } catch { quickTodoList.value = []; pendingTodoCount.value = 0 }
}

function toggleQuickTodo() {
  quickTodoOpen.value = !quickTodoOpen.value
  if (quickTodoOpen.value) loadQuickTodos()
}

async function toggleQuickTodoDone(t: any) {
  try {
    const { toggleTodo } = await import('@/api/db-client')
    await toggleTodo(t.id)
    await loadQuickTodos()
  } catch (e) { console.error('[Header] toggleQuickTodo failed:', e) }
}

async function deleteQuickTodo(t: any) {
  try {
    const { deleteTodo } = await import('@/api/db-client')
    await deleteTodo(t.id)
    await loadQuickTodos()
    toast({ message: '待办已删除', type: 'success' })
  } catch { toast({ message: '删除失败', type: 'error' }) }
}

// ═══════════════════════════════════════════════════════════════════
// Quick note (闪记)
// ═══════════════════════════════════════════════════════════════════
const quickNoteOpen = ref(false)
const quickNoteText = ref('')
const quickNoteRef = ref<HTMLTextAreaElement | null>(null)
const todayNoteId = ref<string | null>(null)
const isSaving = ref(false)

// ── Open: ensure today's daily note exists ──
async function openQuickNote() {
  quickNoteOpen.value = true
  quickNoteText.value = ''
  todayNoteId.value = null
  isSaving.value = true
  try {
    const now = new Date()
    const y = now.getFullYear()
    const m = String(now.getMonth() + 1).padStart(2, '0')
    const d = String(now.getDate()).padStart(2, '0')
    const today = `${y}-${m}-${d}`
    const r: any = await notesV2ListDaily()
    const dailyNotes = r?.notes || []
    const todayNote = dailyNotes.find((n: any) => n.title === today)
    if (todayNote) {
      todayNoteId.value = todayNote.note_id
    } else {
      const cr: any = await notesV2Create({ title: today, content: '', isDaily: true })
      todayNoteId.value = cr?.noteId || null
    }
  } catch { /* ignore */ }
  isSaving.value = false
  await nextTick()
  quickNoteRef.value?.focus()
}

// ── Save: prepend block to today's daily note ──
async function saveQuickNote() {
  const text = quickNoteText.value.trim()
  if (!text) return false
  isSaving.value = true
  try {
    const now = new Date()
    const y = now.getFullYear()
    const m = String(now.getMonth() + 1).padStart(2, '0')
    const d = String(now.getDate()).padStart(2, '0')
    const today = `${y}-${m}-${d}`
    const nowTime = now.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })

    const block = `<p><br></p>\n<blockquote class="is-flash"><p style="color:#94a3b8;font-size:11px;margin-bottom:4px"><span class="time-tag">${nowTime}</span></p><p>${text.replace(/\n/g, '<br>')}</p></blockquote>`

    if (todayNoteId.value) {
      // Append new block to existing content
      const r: any = await notesV2ListDaily()
      const notes = r?.notes || []
      const existing = notes.find((n: any) => n.note_id === todayNoteId.value)
      const oldContent = existing?.content || ''
      const newContent = oldContent ? `${block}${oldContent}` : `${block}`
      await notesV2Update({ noteId: todayNoteId.value, title: today, content: newContent })
    } else {
      // Create new daily note (fallback)
      const r: any = await notesV2Create({
        title: today,
        content: block,
        isDaily: true,
      })
      todayNoteId.value = r?.noteId || null
    }
  } catch (e: any) { /* silent save */ }
  isSaving.value = false
  return true
}

// ── Close with save ──
async function closeWithSave() {
  const saved = await saveQuickNote()
  quickNoteOpen.value = false
  quickNoteText.value = ''
  if (saved) toast({ message: '已保存至今日闪记', type: 'success' })
}

// ── Discard: delete ALL today's note content and close ──
async function discardQuickNote() {
  if (todayNoteId.value) {
    try {
      await notesV2Delete(todayNoteId.value)
    } catch { /* ignore */ }
    todayNoteId.value = null
  }
  quickNoteOpen.value = false
  quickNoteText.value = ''
}

// ── Auto-save on blur (with delay to allow close button to work) ──
let blurTimer: ReturnType<typeof setTimeout> | null = null
function onQuickBlur() {
  blurTimer = setTimeout(() => {
    if (quickNoteOpen.value && quickNoteText.value.trim()) {
      saveQuickNote()
      toast({ message: '已自动保存', type: 'success' })
    }
  }, 200)
}
function onQuickFocus() {
  if (blurTimer) { clearTimeout(blurTimer); blurTimer = null }
}

// ── Cleanup ──
let todoPollTimer: ReturnType<typeof setInterval> | null = null
const quickTodoWrap = ref<HTMLElement | null>(null)
function onDocumentMouseDown(e: MouseEvent) {
  if (quickTodoOpen.value && quickTodoWrap.value && !quickTodoWrap.value.contains(e.target as Node)) {
    quickTodoOpen.value = false
  }
}
onMounted(() => {
  loadQuickTodos()
  todoPollTimer = setInterval(loadQuickTodos, 30 * 1000)
  document.addEventListener('mousedown', onDocumentMouseDown, true)
})
// 路由变化时刷新待办计数
watch(() => route.path, () => loadQuickTodos())
onUnmounted(() => {
  if (blurTimer) clearTimeout(blurTimer)
  if (todoPollTimer) clearInterval(todoPollTimer)
  document.removeEventListener('mousedown', onDocumentMouseDown, true)
})
</script>

<template>
  <header data-tauri-drag-region class="titlebar">
    <button class="titlebar-btn titlebar-btn--menu" @mousedown.stop @click="emit('toggle-sidebar')">
      <AppIcon name="menu" class="h-5 w-5" />
    </button>

    <span class="text-[12px] text-surface-400 font-medium ml-2 select-none" data-tauri-drag-region>{{ pageTitle }}</span>

    <div class="flex-1" />

    <!-- Quick todo button -->
    <div class="relative" ref="quickTodoWrap">
      <button class="titlebar-btn titlebar-btn--todo" @mousedown.stop @click="toggleQuickTodo" title="快速待办">
        <AppIcon name="list" size="14" />
        <span>待办</span>
        <span v-if="pendingTodoCount > 0" class="ml-1 px-1.5 py-0.5 rounded-full bg-red-500 text-white text-[9px] font-medium leading-none">{{ pendingTodoCount > 99 ? '99+' : pendingTodoCount }}</span>
      </button>
      <!-- Quick todo dropdown -->
      <div v-if="quickTodoOpen" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-2xl shadow-xl border border-surface-100 w-[360px]" @click.stop>
        <!-- Todo list -->
        <div class="p-3">
          <div class="flex items-center justify-between mb-2">
            <span class="text-xs font-semibold text-surface-700">待办清单</span>
            <button @click="goToTodos" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建待办">
              <AppIcon name="plus" size="12" stroke="2.5"/>
            </button>
          </div>
          <div class="space-y-1.5 max-h-[360px] overflow-y-auto pr-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
            <div v-for="t in quickTodoList" :key="t.id" @click.stop="showTodoPreview(t)"
              class="flex items-center gap-2.5 px-3 py-2 text-xs rounded-lg cursor-pointer group border-l-[3px]"
              :class="[t.done ? 'opacity-50' : '', t.priority === 'P1' ? 'border-red-500 bg-red-50/40 hover:bg-red-50' : t.priority === 'P2' ? 'border-amber-500 bg-amber-50/40 hover:bg-amber-50' : t.priority === 'P3' ? 'border-blue-500 bg-blue-50/40 hover:bg-blue-50' : 'border-gray-300 bg-gray-50/40 hover:bg-gray-50']">
              <button @click.stop="toggleQuickTodoDone(t)" class="w-4 h-4 rounded-full border-2 flex items-center justify-center shrink-0 transition-all"
                :class="t.done ? 'bg-gray-400 border-gray-400' : t.priority === 'P1' ? 'border-red-500' : t.priority === 'P2' ? 'border-amber-500' : t.priority === 'P3' ? 'border-blue-500' : 'border-gray-300'">
                <AppIcon v-if="t.done" name="check" class="w-2.5 h-2.5 text-white" />
              </button>
              <div class="flex-1 min-w-0">
                <p class="truncate" :class="t.done ? 'text-surface-400 line-through' : 'text-surface-700'">{{ t.title }}</p>
                <div class="text-[9px] text-surface-400 mt-0.5">{{ t.project_name || '无' }}</div>
                <div class="flex items-center gap-1.5 mt-0.5">
                  <span class="text-[9px] text-surface-400">日期：{{ t.due_date ? t.due_date.slice(0,10) + (t.due_date.includes('T') ? ' ' + t.due_date.slice(11,16) : '') : '无' }}</span>
                  <span v-if="t.repeat" class="inline-flex items-center gap-0.5 px-1 py-0.5 rounded text-[9px] font-medium" :class="t.repeat==='daily'?'bg-blue-50 text-blue-600':t.repeat==='weekly'?'bg-purple-50 text-purple-600':'bg-green-50 text-green-600'">
                    <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
                    <span>{{ t.repeat==='daily'?'每天':t.repeat==='weekly'?'每周':'每月' }}</span>
                  </span>
                </div>
              </div>
              <button @click.stop="deleteQuickTodo(t)" class="shrink-0 text-surface-300 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-all" title="删除">
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/></svg>
              </button>
            </div>
            <div v-if="quickTodoList.length === 0" class="text-center py-4 text-surface-400 text-[11px]">暂无待办</div>
          </div>
        </div>
      </div>

      <!-- Todo preview popover -->
      <div v-if="todoPreview" class="fixed inset-0 z-[100] flex items-center justify-center bg-black/20" @click.self="todoPreview = null">
        <div class="bg-white rounded-2xl shadow-2xl w-full max-w-md mx-4 p-5">
          <div class="flex items-center justify-between mb-3">
            <h3 class="text-sm font-semibold text-surface-800">{{ todoPreview.title }}</h3>
            <button @click="todoPreview = null" class="text-surface-400 hover:text-surface-600"><svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M6 6l12 12M6 18L18 6"/></svg></button>
          </div>
          <div class="flex flex-wrap items-center gap-4 mb-3 text-[10px] text-surface-500">
            <div class="flex items-center gap-1">
              <span class="text-surface-400">项目：</span>
              <span :class="todoPreview.project_name ? 'text-surface-600' : 'text-surface-300'">{{ todoPreview.project_name || '无' }}</span>
            </div>
            <div class="flex items-center gap-1">
              <span class="text-surface-400">日期：</span>
              <span :class="todoPreview.due_date ? 'text-surface-600' : 'text-surface-300'">{{ todoPreview.due_date ? todoPreview.due_date.slice(0,10) + (todoPreview.due_date.includes('T') ? ' ' + todoPreview.due_date.slice(11,16) : '') : '无' }}</span>
            </div>
            <div class="flex items-center gap-1">
              <span class="text-surface-400">优先级：</span>
              <span v-if="todoPreview.priority" class="px-1.5 py-0.5 rounded text-[10px] font-medium" :class="todoPreview.priority==='P1'?'bg-red-50 text-red-600':todoPreview.priority==='P2'?'bg-amber-50 text-amber-600':'bg-blue-50 text-blue-600'">{{ {P1:'P1 紧急',P2:'P2 重要',P3:'P3 普通'}[todoPreview.priority] }}</span>
              <span v-else class="text-surface-300">无</span>
            </div>
          </div>
          <div class="text-xs text-surface-600 leading-relaxed whitespace-pre-wrap max-h-[200px] overflow-y-auto">{{ todoPreview.content?.replace(/<[^>]*>/g,'') || '（无内容）' }}</div>
        </div>
      </div>
    </div>

    <!-- Global search bar — absolutely centered in header -->
    <div class="absolute left-1/2 -translate-x-1/2 ml-[-40px] top-1/2 -translate-y-1/2 w-[320px] z-[40]" style="-webkit-app-region: no-drag;">
      <div class="relative">
        <AppIcon name="search" size="13" class="absolute left-2.5 top-1/2 -translate-y-1/2 text-surface-300 pointer-events-none" />
        <input
          ref="searchInputRef"
          v-model="searchQuery"
          @input="onSearchInput"
          @focus="onSearchInput"
          @blur="closeSearch"
          type="text"
          placeholder="搜索项目、笔记、产品、文件..."
          class="w-full h-7 pl-7 pr-3 rounded-lg border border-surface-200 bg-surface-50 text-[12px] text-surface-700 outline-none focus:border-brand-400 focus:bg-white transition-colors placeholder:text-surface-400"
        />
        <div v-if="searchOpen && (searchResults.length > 0 || searchLoading)"
          class="absolute top-full left-1/2 -translate-x-1/2 mt-1 z-[50] bg-white rounded-lg shadow-lg border border-surface-200 max-h-[400px] overflow-y-auto w-[480px]">
          <!-- Filter buttons -->
          <div class="flex flex-wrap gap-1.5 px-3 pt-2.5 pb-2 border-b border-surface-100">
            <button @mousedown.prevent="searchFilter=null"
              class="text-[10px] px-2 py-1 rounded-full transition-colors"
              :class="searchFilter===null ? 'bg-brand-600 text-white' : 'bg-surface-100 text-surface-500 hover:bg-surface-200'">
              全部 {{ searchResults.length }}
            </button>
            <button v-for="(label,type) in typeLabelMap" :key="type"
              @mousedown.prevent="searchFilter=searchFilter===type?null:type"
              class="text-[10px] px-2 py-1 rounded-full transition-colors"
              :class="searchFilter===type ? 'bg-brand-600 text-white' : 'bg-surface-100 text-surface-500 hover:bg-surface-200'">
              {{ label }} {{ searchResults.filter(r=>r.source_type===type).length }}
            </button>
          </div>
          <div v-if="searchLoading" class="px-3 py-4 text-center text-[11px] text-surface-400">搜索中...</div>
          <div v-else-if="filteredResults.length === 0" class="px-3 py-4 text-center text-[11px] text-surface-400">无匹配结果</div>
          <div v-else>
            <div v-for="obj in filteredResults" :key="obj.source_type + obj.source_id"
              @mousedown.prevent="selectSearchResult(obj)"
              class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer transition-colors border-b border-surface-50 last:border-b-0">
              <span class="text-[9px] px-1.5 py-0.5 rounded font-medium shrink-0" :class="typeBadgeMap[obj.source_type] || 'bg-surface-100 text-surface-500'">{{ typeLabelMap[obj.source_type] || obj.source_type }}</span>
              <span class="text-[12px] text-surface-700 truncate" :title="obj.title || '无标题'">{{ obj.title || '无标题' }}</span>
              <span v-if="obj.project_name" class="text-[10px] text-violet-500 truncate max-w-[120px]" :title="obj.project_name">{{ obj.project_name }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Quick note (闪记) button -->
    <button class="titlebar-btn titlebar-btn--quicknote" @mousedown.stop @click="openQuickNote" title="快速闪记">
      <AppIcon name="bolt" size="14" />
      <span>闪记</span>
    </button>

    <!-- Window controls (macOS 使用原生红绿灯，隐藏自定义按钮) -->
    <div v-if="!isMac" class="titlebar-controls">
      <button class="titlebar-btn titlebar-btn--win" @click="minimizeWindow" title="最小化">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 12h16"/></svg>
      </button>
      <button class="titlebar-btn titlebar-btn--win" @click="toggleMaximize" title="最大化/还原">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 4h16v16H4z"/></svg>
      </button>
      <button class="titlebar-btn titlebar-btn--win titlebar-btn--close" @click="closeWindow" title="关闭">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M6 6l12 12M6 18L18 6"/></svg>
      </button>
    </div>

    <!-- Quick note bubble — memo style -->
    <div v-if="quickNoteOpen" class="fixed inset-0 z-50" @click.self="closeWithSave">
      <div class="absolute w-[340px]" :style="{ top: '50px', right: 'calc(72px + 0.5rem)' }">
        <!-- Memo card body -->
        <div class="quicknote-card">
          <!-- Rotated square arrow — fuses with card body -->
          <div class="quicknote-arrow" />
          <!-- Close X button -->
          <button
            @mousedown.prevent @click.stop="discardQuickNote"
            class="absolute top-2.5 right-2.5 w-6 h-6 flex items-center justify-center rounded-md text-amber-400/70 hover:text-amber-700 hover:bg-amber-100/40 transition-colors z-10"
            title="丢弃并关闭"
          >
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 6l12 12M6 18L18 6"/></svg>
          </button>

          <!-- Text area -->
          <textarea
            ref="quickNoteRef"
            v-model="quickNoteText"
            class="quicknote-textarea"
            placeholder="随手记下想法、待办、灵感..."
            @blur="onQuickBlur"
            @focus="onQuickFocus"
          ></textarea>

          <!-- Footer hint -->
          <div class="quicknote-footer">
            <span class="text-[10px] text-amber-400/60">✍ 失焦自动保存 · 右上角 × 丢弃</span>
          </div>
        </div>
      </div>
    </div>
  </header>
</template>

<!-- non-scoped: webkit-app-region MUST be in unscoped style -->
<style>
.titlebar {
  -webkit-app-region: drag;
}
.titlebar-btn {
  -webkit-app-region: no-drag;
}
</style>

<style scoped>
/* ─── Titlebar ─── */
.titlebar {
  display: flex;
  align-items: center;
  gap: 16px;
  height: 46px;
  border-bottom: 1px solid #e5e7eb;
  background: #fff;
  padding-left: 16px;
  padding-right: 4px;
  flex-shrink: 0;
  user-select: none;
  position: relative;
}

.titlebar-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  transition: background 0.15s, color 0.15s;
  cursor: pointer;
}

.titlebar-btn--menu {
  padding: 6px;
  color: #94a3b8;
}
.titlebar-btn--menu:hover {
  background: #f8fafc;
  color: #475569;
}
@media (min-width: 1024px) {
  .titlebar-btn--menu { display: none; }
}

.titlebar-btn--quicknote {
  gap: 6px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 500;
  color: #d97706;
  background: #fffbeb;
}
.titlebar-btn--quicknote:hover {
  background: #fef3c7;
}

.titlebar-btn--todo {
  gap: 6px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 500;
  color: #3b82f6;
  background: #eff6ff;
}
.titlebar-btn--todo:hover {
  background: #dbeafe;
}

.titlebar-title {
  display: flex;
  align-items: center;
  gap: 8px;
}

.titlebar-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: 8px;
}

.titlebar-btn--win {
  width: 32px;
  height: 32px;
  color: #94a3b8;
}
.titlebar-btn--win:hover {
  background: #f1f5f9;
  color: #475569;
}

.titlebar-btn--close:hover {
  background: #fef2f2;
  color: #ef4444;
}

/* ─── Quick note bubble ─── */

/* Memo card — must be relative for the rotated arrow to anchor on */
.quicknote-card {
  position: relative;
  background: #fffbeb;
  border: 1px solid #fde68a;
  border-radius: 14px;
  box-shadow:
    0 12px 32px rgba(217, 119, 6, 0.1),
    0 4px 12px rgba(217, 119, 6, 0.04);
  overflow: visible;
}

/* Rotated square arrow — fuses with card */
.quicknote-arrow {
  position: absolute;
  top: -8px;
  right: 74px;
  width: 16px;
  height: 16px;
  transform: rotate(45deg);
  background: #fffbeb;
  border-top: 1px solid #fde68a;
  border-left: 1px solid #fde68a;
  z-index: 2;
}

/* Textarea */
.quicknote-textarea {
  width: 100%;
  min-height: 200px;
  padding: 18px 20px 14px;
  font-size: 13px;
  line-height: 1.8;
  color: #92400e;
  background: transparent;
  border: none;
  outline: none;
  resize: none;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Microsoft YaHei", sans-serif;
}
.quicknote-textarea::placeholder {
  color: #d97706;
  opacity: 0.4;
  font-style: italic;
}

/* Footer hint */
.quicknote-footer {
  padding: 6px 16px 10px;
  text-align: right;
}
</style>
