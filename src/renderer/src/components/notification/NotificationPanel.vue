<script setup lang="ts">
import { ref, watch, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { notificationList, notificationMarkRead, notificationSetState, notificationDelete, notificationBatchDelete, emailActionConfirm, emailActionIgnore, memoryCreateFromNotification, profileFactSave, profileFactMerge, notificationCreateProjectLog, notificationLinkFile, notificationLinkFiles, notificationUpdateObject, type FileLinkTarget } from '@/api/notification-client'
import type { Notification, NotificationAction, SuggestionMeta } from '@/api/notification-client'
import { profileIdentityRevert, createTodo, createProject, createProduct, createPartner, createPartnerContact, notesV2Create } from '@/api/db-client'
import { sanitizeHtml } from '@/utils/sanitize'
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
// 处置视图：pending=未处置（默认流）| handled=已办结/已忽略（可恢复）
const viewFilter = ref<'pending' | 'handled'>('pending')

async function renderContent(n: Notification) {
  if (renderedContent.value[n.id]) return
  renderedContent.value[n.id] = await renderMd(stripUuids(n.content))
}

async function loadNotifications() {
  loading.value = true
  try {
    const r = await notificationList(props.unread, viewFilter.value)
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

// 用户处置：done/dismissed 出当前流；'' 恢复。行保留在库中作为分析师"不再提醒"的教学证据
async function setNotifState(id: string, state: '' | 'done' | 'dismissed') {
  await notificationSetState(id, state)
  if (viewFilter.value === 'pending' && state !== '') {
    notifications.value = notifications.value.filter(n => n.id !== id)
    expandedId.value = expandedId.value === id ? null : expandedId.value
  } else if (viewFilter.value === 'handled' && state === '') {
    notifications.value = notifications.value.filter(n => n.id !== id)
    expandedId.value = null
  } else {
    const n = notifications.value.find(n => n.id === id)
    if (n) n.user_state = state
  }
}

function switchView(v: 'pending' | 'handled') {
  if (viewFilter.value === v) return
  viewFilter.value = v
  expandedId.value = null
  exitMultiSelect()
  loadNotifications()
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
  link_file: 'save',
  update_object: 'edit',
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
        await router.push({ path: '/contacts', query: { action: 'create', name: p.name as string || '', type: p.type as string || 'CUSTOMER', phone: p.phone as string || '', address: p.address as string || '', remark: p.remark as string || '', contact_person: p.contact_person as string || '', contact_phone: p.contact_phone as string || '' } })
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
      case 'link_file': {
        const res = await runFileLink(p, n.id)
        toast({ message: res.msg, type: res.ok ? 'success' : 'error' })
        if (res.ok) {
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        }
        break
      }
      case 'update_object': {
        const et = (p.entity_type as string || '').trim()
        const eid = (p.entity_id as string || '').trim()
        const ups = (p.updates && typeof p.updates === 'object' && !Array.isArray(p.updates)) ? p.updates as Record<string, unknown> : null
        if (!et || !eid || !ups || Object.keys(ups).length === 0) { toast({ message: '该通知缺少完整更新信息', type: 'info' }); break }
        const lines = Object.entries(ups).map(([k, v]) => `${UPD_FIELD_ZH[k] || k}：→ ${UPD_STATUS_ZH[String(v)] || String(v)}`)
        const evid = (p.evidence as string || '').trim()
        const ok = await dangerConfirm({
          title: '按 AI 洞察更新对象信息',
          message: `${UPD_ENTITY_ZH[et] || et}\n${lines.join('\n')}${evid ? `\n依据：${evid}` : ''}`,
          confirmText: '更新',
        })
        if (!ok) break
        try {
          await notificationUpdateObject(et, eid, ups, n.id)
          toast({ message: '已更新', type: 'success' })
          await notificationMarkRead(n.id)
          notifications.value = notifications.value.filter(x => x.id !== n.id)
        } catch (e: any) {
          toast({ message: `更新失败：${e?.message || e}`, type: 'error' })
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

// link_file 统一执行：单目标(entity_type/entity_id)或批量(params.links[])——一个文件一个按钮
async function runFileLink(p: Record<string, unknown>, notificationId?: string): Promise<{ ok: boolean; msg: string }> {
  const fileId = (p.file_id as string || '').trim()
  if (!fileId) return { ok: false, msg: '该通知缺少完整关联信息' }
  const links = Array.isArray(p.links) ? (p.links as FileLinkTarget[]) : null
  try {
    if (links && links.length) {
      const r = await notificationLinkFiles(fileId, links, notificationId)
      const parts: string[] = []
      if (r?.linked) parts.push(`新增 ${r.linked} 条关联`)
      if (r?.already) parts.push(`${r.already} 条已存在`)
      if (r?.failed?.length) parts.push(`${r.failed.length} 条失败`)
      const msg = parts.join('，')
      return { ok: !!r?.success, msg: msg || '关联失败' }
    }
    const r = await notificationLinkFile(fileId, (p.entity_type as string || '').trim(), (p.entity_id as string || '').trim(), notificationId)
    return { ok: true, msg: r?.already ? '该关联已存在' : '已关联文件' }
  } catch {
    return { ok: false, msg: '关联失败' }
  }
}

// update_object 确认弹窗的中文映射（字段/实体/阶段值）
const UPD_FIELD_ZH: Record<string, string> = {
  name: '名称', description: '描述', budget: '预算', bid_time: '投标时间', expected_order_date: '预计下单时间',
  owner: '负责人', status: '阶段', full_specs: '参数规格', phone: '电话', address: '地址', remark: '备注',
  contact_person: '联系人', contact_phone: '联系电话',
}
const UPD_ENTITY_ZH: Record<string, string> = { PROJECT: '项目', PRODUCT: '产品', CHANNEL: '渠道', VENDOR: '厂商', CUSTOMER: '客户' }
const UPD_STATUS_ZH: Record<string, string> = {
  Lead: '线索', Qualification: '立项', Bidding: '投标中', Won: '已中标', Lost: '已失标', Ordered: '已下单', Archived: '已归档',
}

// ── 一键创建确认弹窗 ──
const createAllDialogVisible = ref(false)
const createAllTarget = ref<Notification | null>(null)
const createAllItems = ref<Array<{ type: string; label: string; preview: string; params: Record<string, unknown> }>>([])

function showCreateAllConfirm(n: Notification) {
  const creates = parseActions(n).filter(a => (a.type.startsWith('create_') && a.type !== 'create_all') || a.type === 'link_file')
  if (creates.length === 0) {
    toast({ message: '没有可执行的操作', type: 'info' })
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
  link_file: '关联',
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
    toast({ message: `已完成 ${successCount} 项操作${failCount > 0 ? `，${failCount} 项失败` : ''}`, type: 'success' })
  } else {
    toast({ message: '执行失败', type: 'error' })
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
      const r: any = await createPartner({ name: p.name || '新建合作伙伴', type: p.type || 'CUSTOMER', phone: p.phone || '', address: p.address || '', remark: p.remark || '', contact_person: p.contact_person || '', contact_phone: p.contact_phone || '' })
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
    case 'link_file': {
      // 批量场景不逐条标记已读（executeCreateAll 循环后统一处理）
      const res = await runFileLink(p)
      if (!res.ok) throw new Error(res.msg)
      return null
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
</script>

<template>
  <div class="flex flex-col h-full select-text">
    <!-- 处置视图切换 -->
    <div class="flex items-center gap-1 px-3 pt-2 pb-1 flex-shrink-0">
      <button @click="switchView('pending')" class="px-2.5 py-1 text-[11px] rounded-md transition-colors"
        :class="viewFilter === 'pending' ? 'bg-blue-50 text-blue-600 font-medium' : 'text-slate-500 hover:bg-slate-50'">待处理</button>
      <button @click="switchView('handled')" class="px-2.5 py-1 text-[11px] rounded-md transition-colors"
        :class="viewFilter === 'handled' ? 'bg-blue-50 text-blue-600 font-medium' : 'text-slate-500 hover:bg-slate-50'">已处理</button>
    </div>
    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading" class="p-6 text-center text-sm text-surface-400">加载中...</div>
      <div v-else-if="notifications.length === 0" class="p-6 text-center text-sm text-surface-400">{{ viewFilter === 'handled' ? '没有已处理的通知' : '暂无通知' }}</div>
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
                <div v-if="n.read === 0 && !n.user_state" class="w-2 h-2 rounded-full flex-shrink-0" :class="priorityDot[n.priority] || priorityDot.medium"></div>
                <span v-tooltip.auto="stripUuids(n.title)"
                  class="text-[13px] font-medium text-slate-800 truncate cursor-default select-text"
                  :class="{ 'font-normal text-slate-500': n.read === 1 || n.user_state }">{{ stripUuids(n.title) }}</span>
                <span v-if="n.user_state === 'done'" class="text-[10px] px-1.5 py-0.5 rounded bg-emerald-50 text-emerald-600 flex-shrink-0 leading-none">已办结</span>
                <span v-else-if="n.user_state === 'dismissed'" class="text-[10px] px-1.5 py-0.5 rounded bg-slate-100 text-slate-400 flex-shrink-0 leading-none">已忽略</span>
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
              <template v-if="viewFilter === 'pending'">
                <button @click.stop="setNotifState(n.id, 'done')" class="text-[11px] text-emerald-600 hover:underline">办结</button>
                <button @click.stop="setNotifState(n.id, 'dismissed')" class="text-[11px] text-slate-400 hover:text-slate-600 hover:underline">忽略</button>
                <button v-if="n.read === 0" @click.stop="markRead(n.id)" class="text-[11px] text-blue-600 hover:underline">标记已读</button>
                <button v-if="n.entity_id && !parseActions(n).length" @click.stop="goToEntity(n)" class="text-[11px] text-blue-600 hover:underline">查看详情</button>
                <button @click.stop="removeNotification(n.id)" class="text-[11px] text-red-500 hover:underline ml-auto">删除</button>
              </template>
              <template v-else>
                <button @click.stop="setNotifState(n.id, '')" class="text-[11px] text-blue-600 hover:underline">回到待处理</button>
                <button @click.stop="removeNotification(n.id)" class="text-[11px] text-red-500 hover:underline ml-auto">删除</button>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Create All Confirmation Dialog -->
    <Teleport to="body">
      <Transition name="modal-fade">
        <div v-if="createAllDialogVisible" class="fixed inset-0 z-[110] bg-black/30 flex items-center justify-center" @click.self="createAllDialogVisible = false">
          <div class="bg-white rounded-2xl shadow-2xl w-[400px] max-w-[90vw] p-6">
            <h3 class="text-[14px] font-semibold text-surface-800 mb-3">确认执行以下操作</h3>
            <div class="space-y-2 mb-5 max-h-[50vh] overflow-y-auto pr-1">
              <div v-for="(item, idx) in createAllItems" :key="item.type + idx" class="flex items-center gap-2 px-3 py-2 rounded-lg bg-surface-50">
                <span class="text-[10px] px-1.5 py-0.5 rounded font-medium bg-surface-200 text-surface-600">
                  {{ CREATE_KIND_LABELS[item.type] || item.type }}
                </span>
                <span class="text-[12px] text-surface-700 truncate flex-1">{{ item.label }}</span>
              </div>
            </div>
            <div class="flex justify-end gap-2">
              <button @click="createAllDialogVisible = false" class="px-4 py-1.5 text-[12px] rounded-lg border border-surface-200 text-surface-600 hover:bg-surface-50">取消</button>
              <button @click="executeCreateAll" class="px-4 py-1.5 text-[12px] rounded-lg bg-brand-600 text-white hover:bg-brand-700">确认执行</button>
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
