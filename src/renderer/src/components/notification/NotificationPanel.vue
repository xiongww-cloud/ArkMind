<script setup lang="ts">
import { ref, watch, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { notificationList, notificationMarkRead, notificationDelete, notificationBatchDelete, emailActionConfirm, emailActionIgnore, memoryCreateFromNotification, profileFactSave, profileFactMerge, notificationCreateProjectLog } from '@/api/notification-client'
import type { Notification, NotificationAction, SuggestionMeta } from '@/api/notification-client'
import { profileIdentityRevert, createTodo, createProject, createProduct, createPartner, createPartnerContact, notesV2Create } from '@/api/db-client'
import { sanitizeHtml } from '@/utils/sanitize'
import { useTooltip } from '@/composables/useTooltip'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import AppIcon from '@/components/common/AppIcon.vue'
import ContactQuickCreateModal from '@/components/common/ContactQuickCreateModal.vue'
import { invoke } from '@tauri-apps/api/core'
import { emit as tauriEmit } from '@tauri-apps/api/event'

const props = defineProps<{
  visible?: boolean
  unread?: boolean
}>()

const emit = defineEmits<{
  close: []
  read: []
}>()

const router = useRouter()
const notifications = ref<Notification[]>([])
const loading = ref(true)
const expandedId = ref<string | null>(null)

// ── 多选模式 ──
const multiSelectMode = ref(false)
const selectedIds = ref<Set<string>>(new Set())
const selectedCount = computed(() => selectedIds.value.size)
const allSelected = computed(() => notifications.value.length > 0 && selectedIds.value.size === notifications.value.length)

function enterMultiSelect() {
  multiSelectMode.value = true
  selectedIds.value.clear()
  expandedId.value = null
}

function exitMultiSelect() {
  multiSelectMode.value = false
  selectedIds.value.clear()
}

function toggleSelect(id: string) {
  const s = new Set(selectedIds.value)
  if (s.has(id)) s.delete(id)
  else s.add(id)
  selectedIds.value = s
}

function toggleSelectAll() {
  if (allSelected.value) {
    selectedIds.value.clear()
  } else {
    selectedIds.value = new Set(notifications.value.map(n => n.id))
  }
}

async function batchDelete() {
  if (selectedIds.value.size === 0) return
  const ok = await dangerConfirm({ title: '批量删除', message: `确认删除选中的 ${selectedIds.value.size} 条通知？` })
  if (!ok) return
  const ids = Array.from(selectedIds.value)
  try {
    await notificationBatchDelete(ids)
    notifications.value = notifications.value.filter(n => !selectedIds.value.has(n.id))
    toast({ message: `已删除 ${ids.length} 条通知`, type: 'success' })
    exitMultiSelect()
    emit('read')
    tauriEmit('notification:updated')
  } catch (e) {
    console.error('[NotificationPanel] batch delete failed:', e)
    toast({ message: '批量删除失败', type: 'error' })
  }
}

async function renderMd(text: string): Promise<string> {
  if (!text) return ''
  try {
    const mod = await import('marked')
    const Marked = mod.Marked || (mod as any).default?.Marked || (mod as any).default
    if (Marked && typeof Marked === 'function') {
      const marked = new Marked({ gfm: true, breaks: true })
      return sanitizeHtml(marked.parse(text) || '')
    }
    const { marked } = mod as any
    return sanitizeHtml((marked?.parse || marked)(text) || '')
  } catch { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;') }
}

const UUID_RE = /[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/g

function stripUuids(text: string): string {
  return (text || '').replace(UUID_RE, '')
}

const renderedContent = ref<Record<string, string>>({})

async function renderContent(n: Notification) {
  if (renderedContent.value[n.id]) return
  renderedContent.value[n.id] = await renderMd(stripUuids(n.content))
}

async function loadNotifications() {
  loading.value = true
  try {
    const r = await notificationList(props.unread)
    notifications.value = r.notifications || []
    // Pre-render all content
    for (const n of notifications.value) {
      await renderContent(n)
    }
  } catch (e) {
    console.error('[NotificationPanel] load failed:', e)
  } finally {
    loading.value = false
  }
}

defineExpose({ reload: loadNotifications, enterMultiSelect, exitMultiSelect, toggleSelectAll, batchDelete, multiSelectMode, allSelected, selectedCount })

onMounted(loadNotifications)
watch(() => props.visible, (val) => { if (val) { loadNotifications(); exitMultiSelect() } })

function toggleExpand(id: string) {
  expandedId.value = expandedId.value === id ? null : id
}

async function markRead(id: string) {
  await notificationMarkRead(id)
  const n = notifications.value.find(n => n.id === id)
  if (n) n.read = 1
  emit('read')
  tauriEmit('notification:updated')
}

async function removeNotification(id: string) {
  const ok = await dangerConfirm({ title: '删除通知', message: '确认删除这条通知？' })
  if (!ok) return
  await notificationDelete(id)
  notifications.value = notifications.value.filter(n => n.id !== id)
  emit('read')
  tauriEmit('notification:updated')
}

function goToEntity(n: Notification) {
  if (n.entity_type === 'project' && n.entity_id) {
    router.push({ path: '/projects', query: { focus: n.entity_id } })
  } else if (n.entity_type === 'todo' && n.entity_id) {
    router.push({ path: '/todos', query: { focus: n.entity_id } })
  } else if (n.entity_type === 'email' && n.entity_id) {
    router.push({ path: '/email', query: { focus: n.entity_id } })
  } else if (n.entity_type === 'note' && n.entity_id) {
    router.push({ path: '/notes', query: { focus: n.entity_id } })
  } else if (n.entity_type === 'product' && n.entity_id) {
    router.push({ path: '/products', query: { focus: n.entity_id } })
  } else if (n.entity_type === 'contact' && n.entity_id) {
    router.push({ path: '/contacts', query: { focus: n.entity_id } })
  }
  emit('close')
}

// ── 通知操作按钮 ──
const ACTION_ICONS: Record<string, string> = {
  view_email: 'envelope',
  view_project: 'folder',
  view_todo: 'checkbox',
  view_note: 'edit',
  view_product: 'package',
  view_contact: 'user',
  view_partner: 'building',
  create_todo: 'plus',
  create_project: 'plus',
  create_note: 'plus',
  create_product: 'plus',
  create_contact: 'plus',
  create_partner: 'plus',
  create_all: 'check-circle',
  save_memory: 'save',
  save_profile: 'user',
  merge_profile: 'refresh',
  revert_profile: 'undo',
  create_project_log: 'file-text',
}

const ACTION_COLORS: Record<string, string> = {
  create_all: 'bg-brand-600 text-white hover:bg-brand-700',
}

function parseActions(n: Notification): NotificationAction[] {
  if (!n.actions) return []
  if (Array.isArray(n.actions)) return n.actions
  if (typeof n.actions === 'string') {
    try { return JSON.parse(n.actions) } catch { return [] }
  }
  return []
}

function actionColor(action: NotificationAction): string {
  return ACTION_COLORS[action.type] || 'bg-surface-100 text-surface-700 hover:bg-surface-200'
}

async function handleAction(n: Notification, action: NotificationAction) {
  const p = action.params || {}
  try {
    switch (action.type) {
      case 'view_email':
        await router.push({ path: '/email', query: { focus: p.email_id as string } })
        emit('close')
        break
      case 'view_project':
        await router.push({ path: '/projects', query: { focus: p.project_id as string } })
        emit('close')
        break
      case 'view_todo':
        await router.push({ path: '/todos', query: { focus: p.todo_id as string } })
        emit('close')
        break
      case 'view_note':
        await router.push({ path: '/notes', query: { focus: p.note_id as string } })
        emit('close')
        break
      case 'view_product':
        await router.push({ path: '/products', query: { focus: p.product_id as string } })
        emit('close')
        break
      case 'view_contact':
        await router.push({ path: '/contacts', query: { focus: p.contact_id as string } })
        emit('close')
        break
      case 'view_partner':
        await router.push({ path: '/contacts', query: { focus: p.partner_id as string } })
        emit('close')
        break
      case 'create_todo':
        await router.push({ path: '/todos', query: { action: 'create', title: p.title as string || '', priority: p.priority as string || '', due_date: p.due_date as string || '', content: p.content as string || '' } })
        emit('close')
        break
      case 'create_project':
        await router.push({ path: '/projects', query: { action: 'create', name: p.name as string || '' } })
        emit('close')
        break
      case 'create_note':
        await router.push({ path: '/notes', query: { action: 'create', title: p.title as string || '', content: p.content as string || '' } })
        emit('close')
        break
      case 'create_product':
        await router.push({ path: '/products', query: { action: 'create', name: p.name as string || '' } })
        emit('close')
        break
      case 'create_contact':
        quickContactPrefill.value = {
          name: (p.name as string) || '', position: (p.position as string) || '',
          email: (p.email as string) || '', phone: (p.phone as string) || '',
          partnerId: (p.partner_id as string) || '', partnerName: (p.partner_name as string) || '',
        }
        quickContactVisible.value = true
        break
      case 'create_partner':
        await router.push({ path: '/contacts', query: { action: 'create', name: p.name as string || '', type: p.type as string || 'CUSTOMER' } })
        emit('close')
        break
      case 'create_all':
        showCreateAllConfirm(n)
        break
      case 'save_memory': {
        const content = (p.content as string || '').trim()
        if (!content) { toast({ message: '该通知未提供可记忆的事实内容', type: 'info' }); break }
        const r = await memoryCreateFromNotification(content, n.id)
        if (r.success) {
          toast({ message: '已记住', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } else {
          toast({ message: '保存失败', type: 'error' })
        }
        break
      }
      case 'save_profile': {
        const content = (p.content as string || '').trim()
        if (!content) { toast({ message: '该通知未提供画像事实内容', type: 'info' }); break }
        const r = await profileFactSave(content, n.id)
        if (r.success) {
          toast({ message: '已记入你的画像', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } else {
          toast({ message: '保存失败', type: 'error' })
        }
        break
      }
      case 'merge_profile': {
        const content = (p.content as string || '').trim()
        const ids = Array.isArray(p.ids) ? (p.ids as string[]).filter(x => typeof x === 'string' && x.trim()) : []
        if (!content) { toast({ message: '该通知未提供整合结果', type: 'info' }); break }
        try {
          await profileFactMerge(ids, content, n.id)
          toast({ message: '画像已整合', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } catch {
          toast({ message: '整合失败', type: 'error' })
        }
        break
      }
      case 'create_project_log': {
        const projectId = (p.project_id as string || '').trim()
        const content = (p.content as string || '').trim()
        if (!projectId) { toast({ message: '缺少项目ID', type: 'info' }); break }
        if (!content) { toast({ message: '缺少日志内容', type: 'info' }); break }
        try {
          await notificationCreateProjectLog(projectId, content, n.id)
          toast({ message: '已记录到项目日志', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } catch {
          toast({ message: '记录失败', type: 'error' })
        }
        break
      }
      case 'revert_profile': {
        const before = (p.before as Record<string, string>) || {}
        const rejected = (p.revert_rejected as Record<string, string>) || {}
        try {
          await profileIdentityRevert(before, rejected)
          toast({ message: '档案已恢复，该值不会再被自动采用', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } catch {
          toast({ message: '撤销失败', type: 'error' })
        }
        break
      }
    }
  } catch (e) {
    console.error('[NotificationPanel] action failed:', e)
    toast({ message: '操作失败', type: 'error' })
  }
}

// ── 快捷建联系人：就地弹窗（不跳转页面） ──
const quickContactVisible = ref(false)
const quickContactPrefill = ref<{ name?: string; position?: string; email?: string; phone?: string; partnerId?: string; partnerName?: string }>({})

// ── 一键创建确认弹窗 ──
const createAllDialogVisible = ref(false)
const createAllTarget = ref<Notification | null>(null)
const createAllItems = ref<Array<{ type: string; label: string; preview: string; params: Record<string, unknown> }>>([])

function showCreateAllConfirm(n: Notification) {
  const creates = parseActions(n).filter(a => a.type.startsWith('create_') && a.type !== 'create_all')
  if (creates.length === 0) {
    toast({ message: '没有可创建的对象', type: 'info' })
    return
  }
  createAllItems.value = creates.map(a => ({
    type: a.type,
    label: a.label,
    preview: (a.params?.name || a.params?.title || '') as string,
    params: a.params || {},
  }))
  createAllTarget.value = n
  createAllDialogVisible.value = true
}

const CREATE_KIND_LABELS: Record<string, string> = {
  create_todo: '待办',
  create_project: '项目',
  create_note: '笔记',
  create_product: '产品',
  create_contact: '联系人',
  create_partner: '合作伙伴',
}

async function executeCreateAll() {
  if (!createAllTarget.value) return
  createAllDialogVisible.value = false
  let successCount = 0
  let failCount = 0
  // 两遍顺序：公司先建、联系人后建，新公司 id 按名称传给联系人做归属关联
  const ctx: Record<string, string> = {}
  const ordered = [...createAllItems.value].sort((a, b) => (a.type === 'create_contact' ? 1 : 0) - (b.type === 'create_contact' ? 1 : 0))
  for (const item of ordered) {
    try {
      await executeCreate(item.type, item.params, ctx)
      successCount++
    } catch (e) {
      console.error(`[createAll] failed for ${item.type}:`, e)
      failCount++
    }
  }
  if (successCount > 0) {
    toast({ message: `已创建 ${successCount} 个对象${failCount > 0 ? `，${failCount} 个失败` : ''}`, type: 'success' })
  } else {
    toast({ message: '创建失败', type: 'error' })
  }
  await markRead(createAllTarget.value.id)
  createAllTarget.value = null
}

async function executeCreate(type: string, params: Record<string, unknown>, ctx: Record<string, string> = {}) {
  const p = { ...params }
  // Remove empty string values
  for (const k of Object.keys(p)) {
    if (p[k] === '') delete p[k]
  }
  switch (type) {
    case 'create_todo': {
      const r: any = await createTodo({ title: p.title || '新建待办', content: p.content || '', priority: p.priority || 'P2', dueDate: p.due_date || '', projectId: p.project_id || '' })
      const id = r?.todo?.id
      if (!id) throw new Error('create todo failed')
      return id
    }
    case 'create_project': {
      const r: any = await createProject({ name: p.name || '新建项目', description: p.description || '' })
      if (!r?.id) throw new Error('create project failed')
      return r.id
    }
    case 'create_note': {
      const r: any = await notesV2Create({
        title: p.title || '新建笔记', content: p.content || '', notebookId: p.notebook_id || undefined
      })
      const nid = r?.noteId
      if (!nid) throw new Error('create note failed')
      return nid
    }
    case 'create_product': {
      const r: any = await createProduct({ name: p.name || '新建产品', description: p.description || '' })
      if (!r?.id) throw new Error('create product failed')
      return r.id
    }
    case 'create_partner': {
      const r: any = await createPartner({ name: p.name || '新建合作伙伴', type: p.type || 'CUSTOMER', phone: p.phone || '', address: p.address || '', remark: p.remark || '' })
      if (!r?.id) throw new Error('create partner failed')
      if (p.name) ctx[String(p.name).trim().toLowerCase()] = r.id
      return r.id
    }
    case 'create_contact': {
      let pid = (p.partner_id as string) || ''
      if (!pid && p.partner_name) pid = ctx[String(p.partner_name).trim().toLowerCase()] || ''
      if (!pid) throw new Error('缺少归属公司，请单独点击该按钮选择归属后创建')
      const r: any = await createPartnerContact({ partnerId: pid, name: p.name || '新建联系人', email: p.email || '', phone: p.phone || '', position: p.position || '', remark: p.remark || '' })
      if (!r?.id && !r?.success) throw new Error('create contact failed')
      return r?.id
    }
    default:
      throw new Error(`Unknown create type: ${type}`)
  }
}

// ── 邮件 AI 建议：逐条确认后才创建 ──
const kindColor: Record<string, string> = {
  todo: 'bg-blue-50 text-blue-600',
  project: 'bg-violet-50 text-violet-600',
  partner: 'bg-emerald-50 text-emerald-600',
}
const kindLabel: Record<string, string> = {
  todo: '待办',
  project: '项目',
  partner: '合作伙伴',
}

function parseSuggestions(n: Notification): SuggestionMeta[] {
  if (n.type !== 'email_actions' || !n.meta) return []
  try {
    const arr = JSON.parse(n.meta)
    if (Array.isArray(arr)) return arr as SuggestionMeta[]
  } catch {
    /* ignore */
  }
  return []
}

function suggestionTitle(s: SuggestionMeta): string {
  if (s.kind === 'todo') return String(s.title || '')
  return String(s.name || '')
}

function suggestionStatus(s: SuggestionMeta): 'pending' | 'done' | 'ignored' {
  return (s.status as 'pending' | 'done' | 'ignored') || 'pending'
}

async function confirmSuggestion(n: Notification, s: SuggestionMeta) {
  try {
    const res = await emailActionConfirm(n.id, s.id)
    if (res.items) {
      n.meta = JSON.stringify(res.items)
      if (res.items.every((i) => i.status !== 'pending')) {
        n.read = 1
        emit('read')
      }
    }
    if (res.object?.name) {
      toast({ message: `已创建${kindLabel[s.kind] || ''}：${res.object.name}`, type: 'success' })
    } else if (res.note && res.note !== '该建议已处理') {
      toast({ message: res.note, type: 'info' })
    }
  } catch (e) {
    console.error('[NotificationPanel] confirm failed:', e)
    toast({ message: '操作失败', type: 'error' })
  }
}

async function ignoreSuggestion(n: Notification, s: SuggestionMeta) {
  try {
    const res = await emailActionIgnore(n.id, s.id)
    if (res.items) {
      n.meta = JSON.stringify(res.items)
      if (res.items.every((i) => i.status !== 'pending')) {
        n.read = 1
        emit('read')
      }
    }
    toast({ message: '已忽略该建议', type: 'info' })
  } catch (e) {
    console.error('[NotificationPanel] ignore failed:', e)
    toast({ message: '操作失败', type: 'error' })
  }
}

function timeAgo(ts: string): string {
  if (!ts) return ''
  try {
    const norm = ts.includes('T') ? ts : ts.replace(' ', 'T')
    const diff = Date.now() - new Date(norm).getTime()
    if (isNaN(diff)) return ts.slice(0, 10)
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return '刚刚'
    if (mins < 60) return `${mins}分钟前`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}小时前`
    const days = Math.floor(hrs / 24)
    if (days < 30) return `${days}天前`
    return ts.slice(0, 10)
  } catch { return ts.slice(0, 10) }
}

const typeLabel: Record<string, string> = {
  todo_due: '待办到期',
  todo_today: '今日待办',
  project_stale: '项目停滞',
  bid_deadline: '投标截止',
  llm_insight: 'AI 洞察',
  email_actions: '邮件建议',
  email: '邮件提醒',
}

const typeLabelColor: Record<string, string> = {
  todo_due: 'bg-amber-50 text-amber-600',
  todo_today: 'bg-red-50 text-red-600',
  project_stale: 'bg-slate-100 text-slate-500',
  bid_deadline: 'bg-blue-50 text-blue-600',
  llm_insight: 'bg-violet-50 text-violet-600',
  email_actions: 'bg-indigo-50 text-indigo-600',
  email: 'bg-slate-100 text-slate-600',
}

const priorityDot: Record<string, string> = {
  high: 'bg-red-500',
  medium: 'bg-amber-400',
  low: 'bg-surface-300',
}

// ── Tooltip ──
const { tooltipText, tooltipX, tooltipY, tooltipVisible, show: showTip, hide: hideTip } = useTooltip()

function onTitleEnter(e: MouseEvent, title: string) {
  const el = e.currentTarget as HTMLElement
  if (el.scrollWidth > el.clientWidth || el.scrollHeight > el.clientHeight) {
    showTip(el, title)
  }
}
</script>

<template>
  <div class="flex flex-col h-full select-text">
    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading" class="p-6 text-center text-sm text-surface-400">加载中...</div>
      <div v-else-if="notifications.length === 0" class="p-6 text-center text-sm text-surface-400">暂无通知</div>
      <div v-else>
        <div v-for="n in notifications" :key="n.id"
          class="border-b border-surface-50">
          <div class="flex items-center">
            <!-- Checkbox (multi-select mode) -->
            <button v-if="multiSelectMode" @click.stop="toggleSelect(n.id)"
              class="w-8 h-8 flex items-center justify-center flex-shrink-0 ml-1">
              <span class="w-4 h-4 rounded border flex items-center justify-center" :class="selectedIds.has(n.id) ? 'bg-blue-600 border-blue-600' : 'border-slate-300'">
                <AppIcon v-if="selectedIds.has(n.id)" name="check" class="w-3 h-3 text-white" />
              </span>
            </button>
            <button @click="multiSelectMode ? toggleSelect(n.id) : toggleExpand(n.id)"
              class="flex-1 flex items-center gap-3 px-4 py-[5px] text-left transition-colors cursor-pointer min-w-0"
              :class="expandedId === n.id && !multiSelectMode ? 'bg-blue-50 hover:bg-blue-100 active:bg-blue-200' : 'hover:bg-slate-50 active:bg-slate-100'">
              <span v-if="typeLabel[n.type]"
                class="text-[10px] px-1.5 py-0.5 rounded font-medium flex-shrink-0 leading-none"
                :class="typeLabelColor[n.type] || 'bg-slate-100 text-slate-500'">
                {{ typeLabel[n.type] }}
              </span>
              <div class="flex-1 min-w-0 flex items-center gap-2">
                <div v-if="n.read === 0" class="w-2 h-2 rounded-full flex-shrink-0" :class="priorityDot[n.priority] || priorityDot.medium"></div>
                <span @mouseenter="(e) => onTitleEnter(e, stripUuids(n.title))" @mouseleave="hideTip"
                  class="text-[13px] font-medium text-slate-800 truncate cursor-default select-text"
                  :class="{ 'font-normal text-slate-500': n.read === 1 }">{{ stripUuids(n.title) }}</span>
              </div>
              <span class="text-[11px] text-slate-400 flex-shrink-0">{{ timeAgo(n.created_at) }}</span>
              <svg v-if="!multiSelectMode" class="w-3 h-3 flex-shrink-0 text-slate-400 transition-transform duration-200"
                   :class="{ 'rotate-90': expandedId === n.id }"
                   fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7"/>
              </svg>
            </button>
          </div>
          <!-- Expanded content -->
          <div v-if="expandedId === n.id && !multiSelectMode">
            <div v-if="renderedContent[n.id]" class="notif-md px-4 pb-2 pl-14 text-[13px] text-slate-600 leading-relaxed select-text" v-html="renderedContent[n.id]" />
            <!-- 操作按钮 -->
            <div v-if="parseActions(n).length" class="px-4 pb-2 pl-14 flex flex-wrap gap-1.5">
              <button
                v-for="action in parseActions(n)"
                :key="action.type"
                @click.stop="handleAction(n, action)"
                class="inline-flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium rounded-lg transition-colors"
                :class="actionColor(action)"
              >
                <AppIcon :name="ACTION_ICONS[action.type] || 'arrow-right'" size="12" />
                {{ action.label }}
              </button>
            </div>
            <!-- AI 建议逐条确认 -->
            <div v-if="parseSuggestions(n).length" class="px-4 pb-2 pl-14 space-y-1.5">
              <div
                v-for="s in parseSuggestions(n)"
                :key="s.id"
                class="flex items-center gap-2 rounded-lg border border-slate-100 px-2.5 py-1.5 select-text"
                :class="suggestionStatus(s) === 'pending' ? 'bg-white' : 'bg-slate-50'"
              >
                <span class="text-[10px] px-1.5 py-0.5 rounded font-medium flex-shrink-0 leading-none"
                  :class="kindColor[s.kind] || 'bg-slate-100 text-slate-500'">
                  {{ kindLabel[s.kind] || s.kind }}
                </span>
                <div class="flex-1 min-w-0">
                  <div class="text-[12px] text-slate-700 truncate select-text">{{ suggestionTitle(s) }}</div>
                  <div v-if="suggestionStatus(s) === 'done' && s.result?.name" class="text-[10px] text-emerald-600">
                    已创建：{{ s.result.name }}
                  </div>
                  <div v-if="s.note" class="text-[10px] text-slate-400">{{ s.note }}</div>
                </div>
                <template v-if="suggestionStatus(s) === 'pending'">
                  <button @click.stop="confirmSuggestion(n, s)"
                    class="flex-shrink-0 text-[11px] px-2 py-0.5 rounded bg-indigo-600 text-white hover:bg-indigo-700">确认创建</button>
                  <button @click.stop="ignoreSuggestion(n, s)"
                    class="flex-shrink-0 text-[11px] px-2 py-0.5 rounded text-slate-400 hover:text-slate-600 hover:bg-slate-100">忽略</button>
                </template>
                <span v-else-if="suggestionStatus(s) === 'ignored'" class="flex-shrink-0 text-[10px] text-slate-400">已忽略</span>
              </div>
            </div>
            <div class="flex items-center gap-3 px-4 pb-2 pl-14">
              <button v-if="n.read === 0" @click.stop="markRead(n.id)" class="text-[11px] text-blue-600 hover:underline">标记已读</button>
              <button v-if="n.entity_id && !parseActions(n).length" @click.stop="goToEntity(n)" class="text-[11px] text-blue-600 hover:underline">查看详情</button>
              <button @click.stop="removeNotification(n.id)" class="text-[11px] text-red-500 hover:underline ml-auto">删除</button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Tooltip (Teleport) -->
    <Teleport to="body">
      <div v-if="tooltipVisible"
           class="fixed pointer-events-none z-[9999] px-2 py-1 text-[11px] text-white bg-gray-800 rounded shadow-lg max-w-[320px]"
           :class="tooltipText.length > 40 ? 'break-words' : 'whitespace-nowrap'"
           :style="{ left: tooltipX + 'px', top: tooltipY + 'px', transform: 'translate(-50%, -100%)' }">
        {{ tooltipText }}
      </div>
    </Teleport>

    <!-- Create All Confirmation Dialog -->
    <Teleport to="body">
      <Transition name="modal-fade">
        <div v-if="createAllDialogVisible" class="fixed inset-0 z-[110] bg-black/30 flex items-center justify-center" @click.self="createAllDialogVisible = false">
          <div class="bg-white rounded-2xl shadow-2xl w-[400px] max-w-[90vw] p-6">
            <h3 class="text-[14px] font-semibold text-surface-800 mb-3">确认创建以下对象</h3>
            <div class="space-y-2 mb-5">
              <div v-for="item in createAllItems" :key="item.type" class="flex items-center gap-2 px-3 py-2 rounded-lg bg-surface-50">
                <span class="text-[10px] px-1.5 py-0.5 rounded font-medium bg-surface-200 text-surface-600">
                  {{ CREATE_KIND_LABELS[item.type] || item.type }}
                </span>
                <span class="text-[12px] text-surface-700 truncate flex-1">{{ item.label }}</span>
              </div>
            </div>
            <div class="flex justify-end gap-2">
              <button @click="createAllDialogVisible = false" class="px-4 py-1.5 text-[12px] rounded-lg border border-surface-200 text-surface-600 hover:bg-surface-50">取消</button>
              <button @click="executeCreateAll" class="px-4 py-1.5 text-[12px] rounded-lg bg-brand-600 text-white hover:bg-brand-700">确认创建</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- 快捷建联系人（就地弹窗） -->
    <ContactQuickCreateModal :visible="quickContactVisible" :prefill="quickContactPrefill" @close="quickContactVisible = false" />
  </div>
</template>

<style scoped>
.notif-md :deep(p) { margin: 0.3em 0; }
.notif-md :deep(strong) { font-weight: 600; color: #1e293b; }
.notif-md :deep(ul), .notif-md :deep(ol) { padding-left: 1.2em; margin: 0.2em 0; }
.notif-md :deep(li) { margin: 0.15em 0; }
.notif-md :deep(code) { background: #f1f5f9; color: #e11d48; padding: 1px 4px; border-radius: 3px; font-size: 0.9em; }
.notif-md :deep(pre) { background: #f8fafc; padding: 8px 10px; border-radius: 6px; overflow-x: auto; margin: 0.4em 0; border: 1px solid #e2e8f0; font-size: 12px; }
.notif-md :deep(pre code) { background: none; color: inherit; padding: 0; }
.notif-md :deep(blockquote) { border-left: 3px solid #3b82f6; padding: 4px 10px; margin: 0.4em 0; color: #475569; background: #f0f7ff; border-radius: 0 4px 4px 0; }
.notif-md :deep(a) { color: #2563eb; text-decoration: none; }
.notif-md :deep(a:hover) { text-decoration: underline; }
.notif-md :deep(table) { width: 100%; border-collapse: collapse; margin: 0.4em 0; font-size: 12px; }
.notif-md :deep(th), .notif-md :deep(td) { padding: 4px 8px; border: 1px solid #e2e8f0; text-align: left; }
.notif-md :deep(th) { background: #f8fafc; font-weight: 600; }
</style>
