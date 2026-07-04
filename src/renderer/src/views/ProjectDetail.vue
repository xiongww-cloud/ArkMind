
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import { getTiptapExtensions } from '@/composables/useTiptapConfig'
import { useRoute, useRouter } from 'vue-router'
import { useProjectStore } from '@/stores/project'
import { useSelectionStore } from '@/stores/selection'
import { useAuthStore } from '@/stores/auth'
import { alert, dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { formatDateTime } from '@/utils/time'
import { deleteProjectBackend, fetchPartners, createPartner, updatePartner, deletePartner, createPartnerContact, updatePartnerContact, deletePartnerContact, partnerLinkProject, partnerUnlinkProject, partnerUpdateRemark, contactLinkProject, contactUnlinkProject, fetchChannels, createChannel, updateChannel, deleteChannel, createChannelContact, updateChannelContact, deleteChannelContact, channelLinkProject, channelUnlinkProject, fetchVendors, createVendor, updateVendor, deleteVendor, createVendorContact, updateVendorContact, deleteVendorContact, vendorLinkProject, vendorUnlinkProject, fetchProjectNotes, createNote, updateNote, deleteNote, fetchTodos, createTodo, updateTodo, toggleTodo, deleteTodo, fetchProjects, fetchFiles, previewFile, updateFileMeta, fetchFileTree, contactsListAll, contactsListChannels, contactsListVendors } from '@/api/db-client'
import Timeline from '@/components/project/Timeline.vue'
import FileManager from '@/components/project/FileManager.vue'
import SolutionConfigurator from '@/components/project/SolutionConfigurator.vue'
import ProjectDetailCustomer from '@/components/project/ProjectDetailCustomer.vue'
import TodoDetail from '@/views/TodoDetail.vue'
import AtMentionSelector from '@/components/common/AtMentionSelector.vue'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'

const props = defineProps<{ id?: string; embedded?: boolean }>()

const route = useRoute()
const router = useRouter()
const projectStore = useProjectStore()
const selectionStore = useSelectionStore()
const authStore = useAuthStore()

const projectId = computed(() => props.id || route.params.id as string)
const project = computed(() => projectStore.getProjectById(projectId.value))

// ── Isolated dynamics — NEVER sourced from project.timeline to avoid state pollution ──
const dynamics = ref([])

const activeTab = ref('detail')

const tabs = [
  { id: 'detail', label: '详情', icon: 'info' },
  { id: 'files', label: '文件', icon: 'document' },
  { id: 'notes', label: '笔记', icon: 'file-doc' },
  { id: 'todos', label: '待办', icon: 'list' },
  { id: 'configurator', label: '方案', icon: 'cog' },
  { id: 'overview', label: '日志', icon: 'clock' }
]

const statusLabels = {
  Lead: '线索',
  SolutionDesign: '方案报价',
  PoC: '测试',
  Bidding: '投标',
  Won: '中标',
  Ordered: '下单',
  Lost: '丢单',
  OnHold: '搁置'
}

const statusOptions = ['Lead', 'SolutionDesign', 'PoC', 'Bidding', 'Won', 'Ordered', 'Lost', 'OnHold']

const statusColors = {
  Lead: 'bg-blue-500',
  SolutionDesign: 'bg-cyan-500',
  PoC: 'bg-purple-500',
  Bidding: 'bg-orange-500',
  Won: 'bg-emerald-500',
  Ordered: 'bg-indigo-500',
  Lost: 'bg-red-500',
  OnHold: 'bg-gray-500'
}

const showStatusDropdown = ref(false)
const statusDropdownRef = ref(null)
const showMoreMenu = ref(false)
const moreMenuRef = ref(null)
const isArchiving = ref(false)

function toggleStatusDropdown() { showStatusDropdown.value = !showStatusDropdown.value }
const bidDaysRemaining = computed(() => {
  const bt = project.value?.bidTime || project.value?.bid_time
  if (!bt) return Infinity
  const now = new Date()
  const bidDate = new Date(bt)
  if (isNaN(bidDate.getTime())) return Infinity
  return Math.ceil((bidDate.getTime() - now.getTime()) / (1000 * 60 * 60 * 24))
})
const isBidOverdue = computed(() => bidDaysRemaining.value <= 0)

async function selectStatus(status) {
  if (status === project.value?.status) { showStatusDropdown.value = false; return }
  const oldLabel = statusLabels[project.value.status] || project.value.status
  project.value.status = status
  projectStore.updateProjectInList(projectId.value, { status })
  await projectStore.updateProjectStatus(projectId.value, status)
  const newLabel = statusLabels[status] || status
  await projectStore.addProjectDynamic(projectId.value, `将项目阶段从「${oldLabel}」变更为「${newLabel}」`)
  showStatusDropdown.value = false
  dynamics.value = projectStore.getProjectDynamics(projectId.value)
  // 切换到投标中时弹出投标时间选择
  if (status === 'Bidding') {
    nextTick(() => startEditBidTime())
  }
}
function toggleMoreMenu() { showMoreMenu.value = !showMoreMenu.value }
async function handleExportProject() {
  showMoreMenu.value = false
  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const savePath = await save({
      defaultPath: `${project.value?.name || 'project'}_export.ark`,
      filters: [{ name: 'ArkMind 项目包', extensions: ['ark'] }],
    })
    if (!savePath) return
    const { exportProject } = await import('@/api/db-client')
    const result = await exportProject(projectId.value, savePath)
    if (!result.success) {
      toast.error(result.error || '导出失败')
      return
    }
    toast.success(`项目已导出至：${result.exportPath || savePath}`)
  } catch (e) {
    toast.error((e && e.message) || '导出失败')
  }
}

async function handleDeleteProject() {
  showMoreMenu.value = false
  const ok = await dangerConfirm({
    title: '删除项目',
    message: `确定要删除项目「${project.value?.name}」吗？此操作不可撤销，所有关联的方案、版本和文件将被永久删除。`
  })
  if (!ok) return
  // 调用 store 的统一删除方法（含持久化黑名单 + 后端级联 + 本地移除）
  await projectStore.deleteProject(projectId.value)
  await router.replace({ name: 'Projects' })
  toast.success('项目已永久删除')
}
async function handleArchiveProject() {
  showMoreMenu.value = false
  const ok = await dangerConfirm({
    title: '归档项目',
    message: '确定归档该项目？归档后项目将进入只读模式，不可再修改配置。',
    confirmText: '归档'
  })
  if (!ok) return
  isArchiving.value = true
  try {
    await projectStore.archiveProject(projectId.value)
  } finally {
    isArchiving.value = false
  }
}
function handleClickOutside(e) {
  if (statusDropdownRef.value && !statusDropdownRef.value.contains(e.target)) showStatusDropdown.value = false
  if (moreMenuRef.value && !moreMenuRef.value.contains(e.target)) showMoreMenu.value = false
  if (bidCalendarRef.value && !bidCalendarRef.value.contains(e.target)) showBidCalendar.value = false
}

const isEditingBudget = ref(false)
const budgetEditValue = ref('')
const budgetInputRef = ref(null)
// Strip ¥ and commas for editing — works for both "¥4,200,000" and "4200000"
const budgetDisplay = computed(() => String(project.value?.budget || '').replace(/[^0-9.]/g, ''))
// 金额以"万"为单位，原始值即万数："420" → "420.00 万"
const budgetFormatted = computed(() => {
  const raw = String(project.value?.budget || '')
  const num = raw.replace(/[^0-9.]/g, '')
  if (!num || isNaN(Number(num))) return '-'
  return Number(num).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) + ' 万'
})
function startEditBudget() { budgetEditValue.value = budgetDisplay.value; isEditingBudget.value = true; nextTick(() => budgetInputRef.value?.focus()) }
async function saveBudget() {
  const v = budgetEditValue.value.trim()
  if (v && project.value) {
    const clean = v.replace(/[^0-9.]/g, '') || '0'
    if (clean === budgetDisplay.value) { isEditingBudget.value = false; return }
    project.value.budget = clean
    projectStore.updateProjectInList(projectId.value, { budget: clean })
    try {
      await projectStore.updateProject(projectId.value, { budget: clean })
    } catch (e) { console.warn('保存金额失败', e) }
    await projectStore.addProjectDynamic(projectId.value, `将项目金额更新为 ¥${clean} 万`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
  }
  isEditingBudget.value = false
}
function cancelEditBudget() { isEditingBudget.value = false }

const isEditingName = ref(false)
const nameEditValue = ref('')
const nameInputRef = ref(null)
const fileManagerKey = ref(0)
function startEditName() { nameEditValue.value = project.value?.name || ''; isEditingName.value = true; nextTick(() => nameInputRef.value?.focus()) }
async function saveName() {
  const v = nameEditValue.value.trim()
  if (v && project.value) {
    if (v === project.value.name) { isEditingName.value = false; return }
    const q = v.toLowerCase()
    const duplicate = projectStore.projects.some((p: any) => p.id !== projectId.value && p.name && p.name.toLowerCase() === q)
    if (duplicate) { toast({ message: '已存在同名项目，项目名称相同代表同一项目', type: 'error' }); return }
    const oldName = project.value.name
    project.value.name = v
    projectStore.updateProjectInList(projectId.value, { name: v })
    try {
      await projectStore.updateProject(projectId.value, { name: v })
    } catch (e) { console.warn('保存名称失败', e) }
    await projectStore.addProjectDynamic(projectId.value, `将项目名称从「${oldName}」更新为「${v}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    fileManagerKey.value++
  }
  isEditingName.value = false
}
function cancelEditName() { isEditingName.value = false }

const isEditingCustomer = ref(false)
const customerEditValue = ref('')
const customerInputRef = ref(null)
function startEditCustomer() { customerEditValue.value = project.value?.customer || ''; isEditingCustomer.value = true; nextTick(() => customerInputRef.value?.focus()) }
async function saveCustomer() {
  const v = customerEditValue.value.trim()
  if (project.value) {
    const current = project.value.customer || ''
    if (v === current) { isEditingCustomer.value = false; return }
    project.value.customer = v || null
    projectStore.updateProjectInList(projectId.value, { customer: v || null })
    await projectStore.updateProject(projectId.value, { customer: v || null })
    const msg = v ? `客户从「${current}」更新为「${v}」` : '清除了客户信息'
    await projectStore.addProjectDynamic(projectId.value, msg)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    await projectStore.fetchProjectDetail(projectId.value)
  }
  isEditingCustomer.value = false
}
function cancelEditCustomer() { isEditingCustomer.value = false }

const isEditingDesc = ref(false)
const descEditValue = ref('')
const descOriginalValue = ref('')
const descInputRef = ref(null)
function startEditDesc() { descOriginalValue.value = project.value?.description || ''; descEditValue.value = descOriginalValue.value; isEditingDesc.value = true; nextTick(() => { descInputRef.value?.focus(); autoResizeDesc() }) }
function autoResizeDesc() { const el = descInputRef.value; if (el) { el.style.height = 'auto'; el.style.height = Math.max(el.scrollHeight, 60) + 'px' } }
function cancelDescEdit() { descEditValue.value = descOriginalValue.value; isEditingDesc.value = false }
async function handleAutoSave() {
  if (!isEditingDesc.value) return
  const v = descEditValue.value.trim()
  if (v === descOriginalValue.value) { isEditingDesc.value = false; return }
  isEditingDesc.value = false
  if (project.value) {
    project.value.description = v
    projectStore.updateProjectInList(projectId.value, { description: v })
    try {
      await projectStore.updateProject(projectId.value, { description: v })
      await projectStore.addProjectDynamic(projectId.value, `项目描述从「${descOriginalValue.value}」更新为「${v}」`)
      dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
      await projectStore.fetchProjectDetail(projectId.value)
    } catch (e) { console.warn('保存描述失败', e) }
  }
}

// ─── 渠道管理 ──────────────────────────────────────────────────────────
const channels = ref([])
const channelEditNameId = ref('')    // which channel's name is being edited
const channelEditRemarkId = ref('')  // which channel's remark is being edited
const channelEditName = ref('')
const channelEditRemark = ref('')
const expandedChannelId = ref(null)  // current expanded channel card

// 渠道选择弹窗
const showChannelSelectModal = ref(false)
const allChannels = ref<any[]>([])
const selectedChannelId = ref('')
const selectedChannelContactId = ref('')
const channelRemark = ref('')  // 项目级备注

const selectedChannel = computed(() => {
  return allChannels.value.find(c => c.id === selectedChannelId.value) || null
})

const channelContacts = computed(() => {
  return selectedChannel.value?.contacts || []
})

async function confirmChannelSelect() {
  if (!selectedChannel.value) return
  
  const channel = selectedChannel.value
  const channelName = channel.name
  const selectedContactId = selectedChannelContactId.value || null
  const remark = channelRemark.value || null
  
  // 关闭弹窗
  showChannelSelectModal.value = false
  selectedChannelId.value = ''
  selectedChannelContactId.value = ''
  channelRemark.value = ''
  
  try {
    // 关联已有渠道到项目，传入选中的联系人 ID 和备注
    await partnerLinkProject(channel.id, projectId.value, selectedContactId, remark)
    
    // 重新加载渠道列表
    channels.value = await fetchPartners(projectId.value, 'CHANNEL') || []
    
    await projectStore.addProjectDynamic(projectId.value, `添加了渠道「${channelName}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: `已添加渠道「${channelName}」`, type: 'success' })
  } catch (e) {
    console.warn('添加渠道失败', e)
    toast({ message: '添加渠道失败', type: 'error' })
  }
}

function toggleChannel(channelId) {
  expandedChannelId.value = expandedChannelId.value === channelId ? null : channelId
}

async function loadChannels() {
  try { channels.value = await fetchPartners(projectId.value, 'CHANNEL') } catch { channels.value = [] }
}

// ── channel name inline edit ──
function startEditChannelName(ch) {
  channelEditNameId.value = ch.id
  channelEditName.value = ch.name || ''
}
async function cancelEditChannelName() {
  const chId = channelEditNameId.value
  const ch = channels.value.find(c => c.id === chId)
  channelEditNameId.value = ''
  if (ch && !ch.name) {
    try {
      await deletePartner(chId)
      channels.value = channels.value.filter(c => c.id !== chId)
    } catch (e) { console.warn('取消添加渠道', e) }
  }
}
async function saveChannelName(channelId) {
  // 防重入：blur 和 enter 可能先后触发
  if (channelEditNameId.value !== channelId) return
  const n = channelEditName.value.trim()
  const ch = channels.value.find(c => c.id === channelId)
  if (ch && n === (ch.name || '')) { channelEditNameId.value = ''; return }
  channelEditNameId.value = ''
  if (!n && ch && !ch.name) {
    try {
      await deletePartner(channelId)
      channels.value = channels.value.filter(c => c.id !== channelId)
    } catch (e) { console.warn('取消添加渠道', e) }
    return
  }
  try {
    const oldName = ch?.name || '未命名'
    await updatePartner({ id: channelId, name: n || '' })
    if (ch) ch.name = n
    await projectStore.addProjectDynamic(projectId.value, `渠道「${oldName}」更名为「${n || '未命名'}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
  } catch (e) { console.warn('更新渠道名称失败', e) }
}

// ── channel remark inline edit ──
function startEditChannelRemark(ch) {
  channelEditRemarkId.value = ch.id
  channelEditRemark.value = ch.project_remark || ch.remark || ''
}
function cancelEditChannelRemark() { channelEditRemarkId.value = '' }
async function saveChannelRemark(channelId) {
  const r = channelEditRemark.value.trim()
  const ch = channels.value.find(c => c.id === channelId)
  if (ch && r === (ch.project_remark || ch.remark || '')) { channelEditRemarkId.value = ''; return }
  channelEditRemarkId.value = ''
  try {
    await partnerUpdateRemark(channelId, projectId.value, r || null)
    if (ch) ch.project_remark = r
  } catch (e) { console.warn('更新渠道备注失败', e) }
}

async function addChannel() {
  // 打开渠道选择弹窗
  showChannelSelectModal.value = true
  try {
    allChannels.value = await contactsListAll('CHANNEL') || []
  } catch (e) { console.warn('加载渠道列表失败', e) }
}
async function removeChannel(channelId) {
  try {
    await partnerUnlinkProject(channelId, projectId.value)
    channels.value = channels.value.filter(c => c.id !== channelId)
    await projectStore.addProjectDynamic(projectId.value, '移除了一个渠道')
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: '已移除渠道', type: 'success' })
  } catch (e) {
    console.warn('移除渠道失败', e)
    toast({ message: '移除失败', type: 'error' })
  }
}

// ─── 渠道联系人（每字段独立编辑） ──────────────────────────────────────────
const editingContactNameOf = ref('')    // contact id whose name is being edited
const editingContactPhoneOf = ref('')   // contact id whose phone is being edited
const editingContactEmailOf = ref('')   // contact id whose email is being edited
const editingContactPositionOf = ref('') // contact id whose position is being edited
const addingContactToChannelId = ref('') // channel id for new contact form
const contactEditName = ref('')
const contactEditPhone = ref('')
const contactEditEmail = ref('')
const contactEditPosition = ref('')

async function startAddContact(channelId) {
  // 打开联系人选择弹窗
  contactSelectChannelId.value = channelId
  // 先加载渠道列表（如果未加载）
  if (allChannels.value.length === 0) {
    try { allChannels.value = await contactsListChannels() || [] } catch {}
  }
  // 从 allChannels 中查找当前渠道的联系人
  const ch = channels.value.find(c => c.id === channelId)
  const systemChannel = allChannels.value.find(sc => sc.name === ch?.name)
  availableContacts.value = systemChannel?.contacts || []
  showContactSelectModal.value = true
}

// ─── 联系人选择弹窗状态 ──────────────────────────────────────────
const showContactSelectModal = ref(false)
const contactSelectChannelId = ref('')
const selectedContactId = ref('')
const availableContacts = ref<any[]>([])

async function confirmContactSelect() {
  if (!selectedContactId.value || !contactSelectChannelId.value) return
  const contact = availableContacts.value.find(c => c.id === selectedContactId.value)
  if (!contact) return
  
  const channelId = contactSelectChannelId.value
  showContactSelectModal.value = false
  selectedContactId.value = ''
  
  try {
    // 关联已有联系人到项目
    await contactLinkProject(contact.id, projectId.value)
    
    // 在 UI 中显示联系人（关联到渠道）
    const ch = channels.value.find(c => c.id === channelId)
    if (ch) {
      if (!Array.isArray(ch.contacts)) ch.contacts = []
      if (!ch.contacts.find(c => c.id === contact.id)) {
        ch.contacts.push({ id: contact.id, name: contact.name, phone: contact.phone, email: contact.email, position: contact.position })
      }
    }
    await projectStore.addProjectDynamic(projectId.value, `添加了联系人「${contact.name}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: `已添加联系人「${contact.name}」`, type: 'success' })
  } catch (e) {
    console.warn('添加联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

// ── contact name inline edit ──
function startEditContactName(ct) {
  editingContactNameOf.value = ct.id
  contactEditName.value = ct.name || ''
}
function cancelEditContactName() { editingContactNameOf.value = '' }
async function saveContactName(channelId, contactId) {
  if (editingContactNameOf.value !== contactId) return
  const n = contactEditName.value.trim()
  editingContactNameOf.value = ''
  if (!n) return
  try {
    await updatePartnerContact({ id: contactId, name: n })
    const ch = channels.value.find(c => c.id === channelId)
    const ct = ch?.contacts?.find(c => c.id === contactId)
    const oldName = ct?.name || ''
    if (ct) ct.name = n
    await projectStore.addProjectDynamic(projectId.value, `联系人姓名从「${oldName}」改为「${n}」`)
  } catch (e) { console.warn('更新联系人姓名失败', e) }
}

// ── contact phone inline edit ──
function startEditContactPhone(ct) {
  editingContactPhoneOf.value = ct.id
  contactEditPhone.value = ct.phone || ''
}
function cancelEditContactPhone() { editingContactPhoneOf.value = '' }
async function saveContactPhone(channelId, contactId) {
  if (editingContactPhoneOf.value !== contactId) return
  const p = contactEditPhone.value.trim()
  editingContactPhoneOf.value = ''
  try {
    await updatePartnerContact({ id: contactId, phone: p || '' })
    const ch = channels.value.find(c => c.id === channelId)
    const ct = ch?.contacts?.find(c => c.id === contactId)
    const op = ct?.phone || ''
    if (ct) ct.phone = p
    const cname = ct?.name || '未命名'
    await projectStore.addProjectDynamic(projectId.value, `联系人「${cname}」电话从「${op}」改为「${p}」`)
  } catch (e) { console.warn('更新联系人电话失败', e) }
}

function onNewContactFocusOut(event, channelId) {
  const container = event.currentTarget
  if (container.contains(event.relatedTarget)) return
  saveNewContact(channelId)
}

// ── new contact: save all fields from the add form ──
async function saveNewContact(channelId) {
  if (!addingContactToChannelId.value || addingContactToChannelId.value !== 'new-' + channelId) return
  const n = contactEditName.value.trim()
  const p = contactEditPhone.value.trim()
  const e = contactEditEmail.value.trim()
  const pos = contactEditPosition.value.trim()
  addingContactToChannelId.value = ''
  if (!n && !p && !e && !pos) return
  try {
    const ct = await createChannelContact({ channelId, name: n, phone: p || '', email: e || '', position: pos || '' })
    const ch = channels.value.find(c => c.id === channelId)
    if (ch) {
      if (!Array.isArray(ch.contacts)) ch.contacts = []
      ch.contacts.push({ id: ct.id, name: ct.name || n, phone: ct.phone || p, email: ct.email || e, position: ct.position || pos })
    }
    // 关联联系人到项目
    await contactLinkProject(ct.id, projectId.value)
    await projectStore.addProjectDynamic(projectId.value, `添加了联系人「${n}」`)
  } catch (e) { console.warn('添加联系人失败', e) }
}

async function removeContact(channelId, contactId) {
  try {
    // 从项目中取消关联联系人
    await contactUnlinkProject(contactId, projectId.value)
    // 在 UI 中移除
    const ch = channels.value.find(c => c.id === channelId)
    if (ch?.contacts) { ch.contacts = ch.contacts.filter(c => c.id !== contactId) }
    await projectStore.addProjectDynamic(projectId.value, '移除了一个联系人')
    toast({ message: '已移除联系人', type: 'success' })
  } catch (e) { console.warn('移除联系人失败', e) }
}

// ─── 厂商管理 ──────────────────────────────────────────────────────────
const vendors = ref([])
const vendorEditNameId = ref('')
const vendorEditRemarkId = ref('')
const vendorEditName = ref('')
const vendorEditRemark = ref('')
const expandedVendorId = ref(null)

// 厂商选择弹窗
const showVendorSelectModal = ref(false)
const allVendors = ref<any[]>([])
const selectedVendorId = ref('')
const selectedVendorContactId = ref('')
const vendorRemark = ref('')  // 项目级备注

const selectedVendor = computed(() => {
  return allVendors.value.find(v => v.id === selectedVendorId.value) || null
})

const vendorContacts = computed(() => {
  return selectedVendor.value?.contacts || []
})

async function confirmVendorSelect() {
  if (!selectedVendor.value) return
  
  const vendor = selectedVendor.value
  const vendorName = vendor.name
  const selectedContactId = selectedVendorContactId.value || null
  const remark = vendorRemark.value || null
  
  // 关闭弹窗
  showVendorSelectModal.value = false
  selectedVendorId.value = ''
  selectedVendorContactId.value = ''
  vendorRemark.value = ''
  
  try {
    // 关联已有厂商到项目，传入选中的联系人 ID 和备注
    await partnerLinkProject(vendor.id, projectId.value, selectedContactId, remark)
    
    // 重新加载厂商列表
    vendors.value = await fetchPartners(projectId.value, 'VENDOR') || []
    
    await projectStore.addProjectDynamic(projectId.value, `添加了厂商「${vendorName}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: `已添加厂商「${vendorName}」`, type: 'success' })
  } catch (e) {
    console.warn('添加厂商失败', e)
    toast({ message: '添加厂商失败', type: 'error' })
  }
}

function toggleVendor(vendorId) {
  expandedVendorId.value = expandedVendorId.value === vendorId ? null : vendorId
}

async function loadVendors() {
  try { vendors.value = await fetchPartners(projectId.value, 'VENDOR') } catch { vendors.value = [] }
}

function startEditVendorName(vd) {
  vendorEditNameId.value = vd.id
  vendorEditName.value = vd.name || ''
}
async function cancelEditVendorName() {
  const vdId = vendorEditNameId.value
  const vd = vendors.value.find(v => v.id === vdId)
  vendorEditNameId.value = ''
  if (vd && !vd.name) {
    try {
      await deletePartner(vdId)
      vendors.value = vendors.value.filter(v => v.id !== vdId)
    } catch (e) { console.warn('取消添加厂商', e) }
  }
}
async function saveVendorName(vendorId) {
  if (vendorEditNameId.value !== vendorId) return
  const n = vendorEditName.value.trim()
  const vd = vendors.value.find(v => v.id === vendorId)
  if (vd && n === (vd.name || '')) { vendorEditNameId.value = ''; return }
  vendorEditNameId.value = ''
  if (!n && vd && !vd.name) {
    try {
      await deletePartner(vendorId)
      vendors.value = vendors.value.filter(v => v.id !== vendorId)
    } catch (e) { console.warn('取消添加厂商', e) }
    return
  }
  try {
    const oldName = vd?.name || '未命名'
    await updatePartner({ id: vendorId, name: n || '' })
    if (vd) vd.name = n
    await projectStore.addProjectDynamic(projectId.value, `厂商「${oldName}」更名为「${n || '未命名'}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
  } catch (e) { console.warn('更新厂商名称失败', e) }
}

function startEditVendorRemark(vd) {
  vendorEditRemarkId.value = vd.id
  vendorEditRemark.value = vd.project_remark || vd.remark || ''
}
function cancelEditVendorRemark() { vendorEditRemarkId.value = '' }
async function saveVendorRemark(vendorId) {
  const r = vendorEditRemark.value.trim()
  const vd = vendors.value.find(v => v.id === vendorId)
  if (vd && r === (vd.project_remark || vd.remark || '')) { vendorEditRemarkId.value = ''; return }
  vendorEditRemarkId.value = ''
  try {
    await partnerUpdateRemark(vendorId, projectId.value, r || null)
    if (vd) vd.project_remark = r
  } catch (e) { console.warn('更新厂商备注失败', e) }
}

async function addVendor() {
  // 打开厂商选择弹窗
  showVendorSelectModal.value = true
  try {
    allVendors.value = await contactsListAll('VENDOR') || []
  } catch (e) { console.warn('加载厂商列表失败', e) }
}
async function removeVendor(vendorId) {
  try {
    await partnerUnlinkProject(vendorId, projectId.value)
    vendors.value = vendors.value.filter(v => v.id !== vendorId)
    await projectStore.addProjectDynamic(projectId.value, '移除了一个厂商')
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: '已移除厂商', type: 'success' })
  } catch (e) {
    console.warn('移除厂商失败', e)
    toast({ message: '移除失败', type: 'error' })
  }
}

// ─── 厂商联系人 ────────────────────────────────────────────────────────
const editingVendorContactNameOf = ref('')
const editingVendorContactPhoneOf = ref('')
const editingVendorContactEmailOf = ref('')
const editingVendorContactPositionOf = ref('')
const addingContactToVendorId = ref('')
const vendorContactEditName = ref('')
const vendorContactEditPhone = ref('')
const vendorContactEditEmail = ref('')
const vendorContactEditPosition = ref('')

async function startAddVendorContact(vendorId) {
  // 打开联系人选择弹窗
  vendorContactSelectVendorId.value = vendorId
  // 先加载厂商列表（如果未加载）
  if (allVendors.value.length === 0) {
    try { allVendors.value = await contactsListAll('VENDOR') || [] } catch {}
  }
  // 从 allVendors 中查找当前厂商的联系人
  const vd = vendors.value.find(v => v.id === vendorId)
  const systemVendor = allVendors.value.find(sv => sv.name === vd?.name)
  availableVendorContacts.value = systemVendor?.contacts || []
  showVendorContactSelectModal.value = true
}

// ─── 厂商联系人选择弹窗状态 ──────────────────────────────────────────
const showVendorContactSelectModal = ref(false)
const vendorContactSelectVendorId = ref('')
const selectedVendorContactToAdd = ref('')
const availableVendorContacts = ref<any[]>([])

async function confirmVendorContactSelect() {
  if (!selectedVendorContactToAdd.value || !vendorContactSelectVendorId.value) return
  const contact = availableVendorContacts.value.find(c => c.id === selectedVendorContactToAdd.value)
  if (!contact) return
  
  const vendorId = vendorContactSelectVendorId.value
  showVendorContactSelectModal.value = false
  selectedVendorContactToAdd.value = ''
  
  try {
    // 关联已有联系人到项目
    await contactLinkProject(contact.id, projectId.value)
    
    // 在 UI 中显示联系人（关联到厂商）
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      if (!vd.contacts.find(c => c.id === contact.id)) {
        vd.contacts.push({ id: contact.id, name: contact.name, phone: contact.phone, email: contact.email, position: contact.position })
      }
    }
    await projectStore.addProjectDynamic(projectId.value, `添加了厂商联系人「${contact.name}」`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
    toast({ message: `已添加联系人「${contact.name}」`, type: 'success' })
  } catch (e) {
    console.warn('添加厂商联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

function startEditVendorContactName(ct) {
  editingVendorContactNameOf.value = ct.id
  vendorContactEditName.value = ct.name || ''
}
function cancelEditVendorContactName() { editingVendorContactNameOf.value = '' }
async function saveVendorContactName(vendorId, contactId) {
  if (editingVendorContactNameOf.value !== contactId) return
  const n = vendorContactEditName.value.trim()
  editingVendorContactNameOf.value = ''
  if (!n) return
  try {
    await updatePartnerContact({ id: contactId, name: n })
    const vd = vendors.value.find(v => v.id === vendorId)
    const ct = vd?.contacts?.find(c => c.id === contactId)
    const oldName = ct?.name || ''
    if (ct) ct.name = n
    await projectStore.addProjectDynamic(projectId.value, `厂商联系人姓名从「${oldName}」改为「${n}」`)
  } catch (e) { console.warn('更新厂商联系人姓名失败', e) }
}

function startEditVendorContactPhone(ct) {
  editingVendorContactPhoneOf.value = ct.id
  vendorContactEditPhone.value = ct.phone || ''
}
function cancelEditVendorContactPhone() { editingVendorContactPhoneOf.value = '' }
async function saveVendorContactPhone(vendorId, contactId) {
  if (editingVendorContactPhoneOf.value !== contactId) return
  const p = vendorContactEditPhone.value.trim()
  editingVendorContactPhoneOf.value = ''
  try {
    await updatePartnerContact({ id: contactId, phone: p || '' })
    const vd = vendors.value.find(v => v.id === vendorId)
    const ct = vd?.contacts?.find(c => c.id === contactId)
    const op = ct?.phone || ''
    if (ct) ct.phone = p
    const cname = ct?.name || '未命名'
    await projectStore.addProjectDynamic(projectId.value, `厂商联系人「${cname}」电话从「${op}」改为「${p}」`)
  } catch (e) { console.warn('更新厂商联系人电话失败', e) }
}

function onNewVendorContactFocusOut(event, vendorId) {
  const container = event.currentTarget
  if (container.contains(event.relatedTarget)) return
  saveNewVendorContact(vendorId)
}

async function saveNewVendorContact(vendorId) {
  if (!addingContactToVendorId.value || addingContactToVendorId.value !== 'new-' + vendorId) return
  const n = vendorContactEditName.value.trim()
  const p = vendorContactEditPhone.value.trim()
  const e = vendorContactEditEmail.value.trim()
  const pos = vendorContactEditPosition.value.trim()
  addingContactToVendorId.value = ''
  if (!n && !p && !e && !pos) return
  try {
    const ct = await createPartnerContact({
      partnerId: vendorId,
      name: n, phone: p || '', email: e || '', position: pos || ''
    })
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      vd.contacts.push({ id: ct.id, name: ct.name || n, phone: ct.phone || p, email: ct.email || e, position: ct.position || pos })
    }
    // 关联联系人到项目
    await contactLinkProject(ct.id, projectId.value)
    await projectStore.addProjectDynamic(projectId.value, `添加了厂商联系人「${n}」`)
  } catch (e) { console.warn('添加厂商联系人失败', e) }
}

async function removeVendorContact(vendorId, contactId) {
  try {
    // 从项目中取消关联联系人
    await contactUnlinkProject(contactId, projectId.value)
    // 在 UI 中移除
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd?.contacts) { vd.contacts = vd.contacts.filter(c => c.id !== contactId) }
    await projectStore.addProjectDynamic(projectId.value, '移除了一个厂商联系人')
    toast({ message: '已移除联系人', type: 'success' })
  } catch (e) { console.warn('移除厂商联系人失败', e) }
}

// ─── 客户信息 — 已拆分到 ProjectDetailCustomer.vue ──────────────────────
async function handleCustomerUpdate() {
  dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
}

// ─── 投标时间日历下拉 ────────────────────────────────────────────────────
const showBidCalendar = ref(false)
const bidCalendarRef = ref(null)
const bidCalendarYear = ref(new Date().getFullYear())
const bidCalendarMonth = ref(new Date().getMonth())

function formatBidTime(v) {
  if (!v) return ''
  try {
    const d = new Date(v)
    if (isNaN(d.getTime())) return v
    return d.getFullYear() + '-' +
      String(d.getMonth() + 1).padStart(2, '0') + '-' +
      String(d.getDate()).padStart(2, '0')
  } catch { return v }
}

function startEditBidTime() {
  const raw = (project.value?.bidTime || project.value?.bid_time || '')
  if (raw) {
    const d = new Date(raw)
    if (!isNaN(d.getTime())) {
      bidCalendarYear.value = d.getFullYear()
      bidCalendarMonth.value = d.getMonth()
    }
  } else {
    const now = new Date()
    bidCalendarYear.value = now.getFullYear()
    bidCalendarMonth.value = now.getMonth()
  }
  showBidCalendar.value = !showBidCalendar.value
}

function prevMonth() {
  if (bidCalendarMonth.value === 0) { bidCalendarMonth.value = 11; bidCalendarYear.value-- }
  else { bidCalendarMonth.value-- }
}

function nextMonth() {
  if (bidCalendarMonth.value === 11) { bidCalendarMonth.value = 0; bidCalendarYear.value++ }
  else { bidCalendarMonth.value++ }
}

const calendarDays = computed(() => {
  const year = bidCalendarYear.value
  const month = bidCalendarMonth.value
  const firstDay = new Date(year, month, 1).getDay()
  const daysInMonth = new Date(year, month + 1, 0).getDate()
  const days = []
  for (let i = 0; i < firstDay; i++) days.push(null)
  for (let d = 1; d <= daysInMonth; d++) days.push(d)
  return days
})

const todayStr = computed(() => {
  const now = new Date()
  return now.getFullYear() + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
})

function isSelectedDay(day) {
  const y = bidCalendarYear.value
  const m = String(bidCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  return (project.value?.bidTime || project.value?.bid_time || '').startsWith(`${y}-${m}-${d}`)
}

function closeBidCalendar() { showBidCalendar.value = false }

async function selectBidDay(day) {
  const y = bidCalendarYear.value
  const m = String(bidCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  const displayTime = `${y}-${m}-${d}`
  showBidCalendar.value = false
  if (project.value) {
    project.value.bidTime = displayTime
    project.value.bid_time = displayTime
  }
  projectStore.updateProjectInList(projectId.value, { bid_time: displayTime })
  try {
    await projectStore.updateProject(projectId.value, { bid_time: displayTime })
    await projectStore.addProjectDynamic(projectId.value, `将投标时间更新为 ${displayTime}`)
    dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
  } catch (e) { console.warn('保存投标时间失败', e) }
}

async function clearBidTime() {
  showBidCalendar.value = false
  const displayTime = ''
  if (project.value) {
    project.value.bidTime = displayTime
    project.value.bid_time = displayTime
  }
  projectStore.updateProjectInList(projectId.value, { bid_time: '' })
  try {
    await projectStore.updateProject(projectId.value, { bid_time: '' })
  } catch (e) { console.warn('清除投标时间失败', e) }
}

const logFilter = ref('all')
const showEventForm = ref(false)
const newEvent = ref({ content: '', operator: authStore.currentUser.name })
const isAddingDynamic = ref(false)

onMounted(async () => {
  document.addEventListener('click', handleClickOutside)
  document.addEventListener('click', closeNotePopups)
  await loadProjectData()
})

// ★ 切换项目时重新加载所有数据（onMounted 在路由参数变化时不会重复触发）
watch(() => projectId.value, async () => {
  // 清空所有临时数据，防止闪旧项目缓存
  channels.value = []
  vendors.value = []
  dynamics.value = []
  projectNotes.value = []
  projectTodos.value = []
  selectedNoteId.value = null
  selectedTodoId.value = null
  projFilesPanel.value = []
  activeTab.value = 'detail'
  // 重置编辑状态
  isEditingName.value = false
  isEditingBudget.value = false
  isEditingCustomer.value = false
  isEditingDesc.value = false
  showStatusDropdown.value = false
  showMoreMenu.value = false
  showBidCalendar.value = false
  await loadProjectData()
})

async function loadProjectData() {
  // Step 1: Refresh/create project from backend — handles both local and backend-driven IDs
  const result = await projectStore.fetchProjectDetail(projectId.value)
  if (!result) { router.push('/projects'); return }
  // Step 2: Fetch dynamics into isolated ref — never touches project.timeline
  dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
  // Step 3: Load channel and vendor data
  loadChannels()
  loadVendors()
}

// 切换 tab 时加载对应数据
watch(activeTab, (tab) => {
  if (tab === 'notes') {
    loadProjectNotes()
    loadAllProjects()
    loadProjFilesPanel()
  } else if (tab === 'todos') {
    loadProjectTodos()
    loadProjFilesPanel()
  }
})
onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
  document.removeEventListener('click', closeNotePopups)
})

// ─── 项目笔记 ──────────────────────────────────────────────────────────────
const projectNotes = ref<any[]>([])
const selectedNoteId = ref<string | null>(null)

const selectedNote = computed(() => projectNotes.value.find(n => n.note_id === selectedNoteId.value))

const editingNoteTitle = ref('')

watch(selectedNote, (note) => {
  editingNoteTitle.value = note?.title || ''
})

async function saveNoteTitle() {
  if (!selectedNote.value || editingNoteTitle.value === selectedNote.value.title) return
  try {
    await updateNote({ noteId: selectedNote.value.note_id, title: editingNoteTitle.value })
    await loadProjectNotes()
  } catch (e) { console.warn('保存笔记标题失败', e) }
}

async function loadProjectNotes() {
  try {
    const backendId = await projectStore.syncProjectToBackend(projectId.value)
    const r = await fetchProjectNotes(backendId)
    projectNotes.value = Array.isArray(r) ? r : (r?.notes || [])
  } catch { projectNotes.value = [] }
}

async function addProjectNote() {
  try {
    const backendId = await projectStore.syncProjectToBackend(projectId.value)
    const r: any = await createNote({ title: '', content: '', isDaily: false, projectIds: [backendId] })
    const newNote = { note_id: r?.noteId || '', id: r?.noteId || '', title: '', content: '',
      updated_at: new Date().toISOString(), created_at: new Date().toISOString() }
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

// ── Tiptap editor for project notes (replica of NotesPage) ──
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

// Load projects for @ mention selector
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

watch(selectedNote, (note) => {
  editingNoteTitle.value = note?.title || ''
  nextTick(() => {
    noteEditor.value?.commands.setContent(note?.content || '', false)
  })
  if (note) loadProjFilesPanel()
})

async function saveNoteContent(html?: string) {
  const content = html ?? noteEditor.value?.getHTML() ?? ''
  if (!selectedNote.value || content === selectedNote.value.content) return
  try {
    await updateNote({ noteId: selectedNote.value.note_id, content })
    selectedNote.value.content = content
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
    case 'align': if (val) ed.setTextAlign(val as any); alignLabel.value = val === 'right' ? '右对齐' : val === 'center' ? '居中' : '左对齐'; break
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

// ─── 项目文件面板（笔记和待办右侧通用）──────────────────────────────
const projFilesPanel = ref<any[]>([])
const projFilesFolders = ref<any[]>([])
const projFilesLoading = ref(false)
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

async function loadProjFilesPanel() {
  projFilesLoading.value = true
  try {
    const backendId = await projectStore.syncProjectToBackend(projectId.value)
    const [filesData, treeData] = await Promise.all([
      fetchFiles(backendId),
      fetchFileTree(backendId)
    ])
    projFilesPanel.value = Array.isArray(filesData) ? filesData : []
    projFilesFolders.value = treeData?.folders || []
  } catch { projFilesPanel.value = []; projFilesFolders.value = [] }
  finally { projFilesLoading.value = false }
}

const panelEditingFileId = ref<string | null>(null)
const panelEditingFileName = ref('')

function panelFileDisplayName(f: any): string {
  return f.custom_name || f.displayName || f.name || f.lib_cn || f.lib_gn || '未命名'
}

function panelFileTypeClass(ext: string) {
  const map: Record<string, string> = {
    docx: 'bg-blue-50 text-blue-600', doc: 'bg-blue-50 text-blue-600',
    pdf: 'bg-red-50 text-red-600',
    xlsx: 'bg-emerald-50 text-emerald-600', xls: 'bg-emerald-50 text-emerald-600',
    pptx: 'bg-orange-50 text-orange-600', ppt: 'bg-orange-50 text-orange-600',
    png: 'bg-purple-50 text-purple-600', jpg: 'bg-purple-50 text-purple-600',
  }
  return map[ext.toLowerCase()] || 'bg-slate-50 text-slate-500'
}

function panelFileSize(bytes: number | undefined): string {
  if (!bytes) return ''
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

async function panelPreviewFile(fileId: string) {
  try { await previewFile(fileId) } catch (e) { console.error('[Project] previewFile failed:', e) }
}

function panelStartRename(f: any) {
  panelEditingFileId.value = f.id
  panelEditingFileName.value = f.custom_name || ''
  nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-panel-rename-input]')
    el?.focus(); el?.select()
  })
}

async function panelConfirmRename(f: any) {
  const n = panelEditingFileName.value.trim()
  if (n) { try { await updateFileMeta(f.id, { customName: n }); await loadProjFilesPanel() } catch (e) { console.error('[Project] renameFile failed:', e) } }
  panelEditingFileId.value = null
}

function panelCancelRename() { panelEditingFileId.value = null }


function noteInsertFileToEditor(f: any) {
  if (!noteEditor.value) return
  const fname = panelFileDisplayName(f)
  const fileId = f.id
  noteEditor.value.chain().focus().insertContent({
    type: 'fileMention',
    attrs: { fileId, name: fname },
  }).insertContent(' ').run()
}

// ─── 项目待办 ──────────────────────────────────────────────────────────────
const projectTodos = ref<any[]>([])
const selectedTodoId = ref<string | null>(null)

const selectedTodo = computed(() => projectTodos.value.find(t => t.id === selectedTodoId.value))

const todoDetailRef = ref<any>(null)

watch(selectedTodo, (todo) => {
  if (todo) loadProjFilesPanel()
})

function todoInsertFileToEditor(f: any) {
  const fname = panelFileDisplayName(f)
  if (todoDetailRef.value?.insertFileAtCursor) {
    todoDetailRef.value?.insertFileAtCursor(f.id, fname)
  }
}

async function loadProjectTodos() {
  try {
    const r = await fetchTodos()
    const all = Array.isArray(r) ? r : (r?.todos || [])
    const backendId = await projectStore.syncProjectToBackend(projectId.value)
    projectTodos.value = all.filter((t: any) => t.project_id === backendId).map((t: any) => ({
      id: t.id, title: t.title || '', content: t.content || '',
      done: t.done === 1 || t.done === true,
      createdAt: t.created_at || new Date().toISOString(),
      dueDate: t.due_date || undefined,
      priority: t.priority || undefined,
      projectId: t.project_id || undefined,
      projectName: t.project_name || '',
    }))
  } catch { projectTodos.value = [] }
}

const projectTodoDoneOpen = ref(false)
const projectUndoneTodos = computed(() => projectTodos.value.filter(t => !t.done))
const projectDoneTodos = computed(() => projectTodos.value.filter(t => t.done))

async function addProjectTodo() {
  try {
    const backendId = await projectStore.syncProjectToBackend(projectId.value)
    const r = await createTodo({ title: '新待办', projectId: backendId })
    if (r?.todo) {
      const t = r.todo
      projectTodos.value.unshift({
        id: t.id, title: t.title || '新待办', content: t.content || '',
        done: false, createdAt: t.created_at || new Date().toISOString(),
        dueDate: t.due_date || undefined, priority: t.priority || undefined,
        projectId: t.project_id || undefined, projectName: t.project_name || '',
      })
      selectedTodoId.value = t.id
    } else {
      toast({ message: '创建待办失败', type: 'error' })
    }
  } catch (e) {
    console.warn('创建待办失败', e)
    toast({ message: '创建待办失败', type: 'error' })
  }
}

async function toggleProjectTodo(todoId: string) {
  try {
    await toggleTodo(todoId)
    await loadProjectTodos()
  } catch (e) { console.warn('切换待办状态失败', e) }
}

async function removeProjectTodo(todoId: string) {
  try {
    await deleteTodo(todoId)
    selectedTodoId.value = null
    await loadProjectTodos()
  } catch (e) { console.warn('删除待办失败', e) }
}

async function handleTodoUpdate(todoId: string, data: Record<string, any>) {
  try {
    await updateTodo(todoId, data)
  } catch (e) { console.warn('更新待办失败', e) }
}

// 实时同步 dynamicsMap 变更（来自 FileManager、SolutionConfigurator 等子组件）
watch(() => projectStore.dynamicsMap[projectId.value], (val) => {
  if (val) dynamics.value = val
}, { immediate: false })

function goBack() { router.push('/projects') }

async function addTimelineEvent() {
  if (!newEvent.value.content.trim() || isAddingDynamic.value) return
  isAddingDynamic.value = true
  try {
    await projectStore.addProjectDynamic(projectId.value, newEvent.value.content, newEvent.value.operator, 'note')
    // Refresh isolated dynamics from store's dynamicsMap
    dynamics.value = projectStore.getProjectDynamics(projectId.value)
    newEvent.value.content = ''
    showEventForm.value = false
  } catch (e) {
    alert({ title: '保存失败', message: e.message || '网络异常' })
  } finally {
    isAddingDynamic.value = false
  }
}

// ── Datetime formatter: ISO → "YYYY-MM-DD HH:mm" (minute precision) ─────
const tabIcons = {
  info: 'info',
  clock: 'clock',
  document: 'file',
  cog: 'settings',
  'file-doc': 'file-doc',
  list: 'list'
}

// ★ 响应全局搜索 / Header 跳转：切 tab + 定位 note/todo
const pendingTarget = ref<{ tab: string; targetId: string } | null>(null)
watch(
  () => selectionStore.current.seq,
  () => {
    if (selectionStore.current.type !== 'project') return
    if (route.params.id !== selectionStore.current.id) return
    const extra = selectionStore.current.extra
    if (!extra?.tab) return
    pendingTarget.value = { tab: extra.tab, targetId: extra.targetId || '' }
    activeTab.value = extra.tab as any
  },
  { immediate: true }
)
// 数据加载后再定位
watch([projectNotes, projectTodos], () => {
  const pt = pendingTarget.value
  if (!pt) return
  if (activeTab.value === 'notes' && pt.targetId) {
    if (projectNotes.value.some((n: any) => n.note_id === pt.targetId)) {
      selectedNoteId.value = pt.targetId
      pendingTarget.value = null
    }
  } else if (activeTab.value === 'todos' && pt.targetId) {
    if (projectTodos.value.some((t: any) => t.id === pt.targetId)) {
      selectedTodoId.value = pt.targetId
      pendingTarget.value = null
    }
  }
})
</script>

<template>
  <div :class="[props.embedded ? 'flex flex-col h-full' : 'h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden']">
    <div v-if="project" class="flex flex-col h-full overflow-hidden">
    <!-- Archiving progress banner -->
    <div v-if="isArchiving" class="w-full bg-amber-50 border border-amber-200 text-amber-600 text-sm py-2 px-4 rounded-md flex items-center">
      <AppIcon name="spinner" class="mr-2 w-4 h-4 text-amber-500 animate-spin flex-shrink-0" />
      正在封存文件...
    </div>

    <!-- Archive Banner -->
    <div v-if="project.isArchived && !isArchiving" class="w-full bg-slate-50 border border-slate-200 text-slate-500 text-sm py-2 px-4 rounded-md flex items-center">
      <AppIcon name="lock" class="mr-2 w-4 h-4 text-slate-400 flex-shrink-0" />
      此项目已归档，处于只读模式。仅支持查看和下载文件。
    </div>

    <!-- Tabs -->
    <div :class="['sticky top-0 z-10 bg-surface-50 pt-1.5 px-3', props.embedded ? '' : 'border-b border-slate-200']">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-0">
          <button v-if="!props.embedded" @click="goBack" class="btn-ghost !p-1 mr-2 flex-shrink-0" title="返回项目列表">
            <AppIcon name="arrow-left" class="h-4 w-4 text-slate-400" />
          </button>
          <nav class="flex gap-0 -mb-px">
            <button v-for="tab in tabs" :key="tab.id" @click="activeTab = tab.id" class="tab" :class="activeTab === tab.id ? 'tab-active' : ''">
              <span class="flex items-center gap-2">
                <AppIcon :name="tabIcons[tab.icon]" class="h-4 w-4" />
                {{ tab.label }}
              </span>
            </button>
          </nav>
        </div>
        <div v-if="!project.isArchived" ref="moreMenuRef" class="relative flex-shrink-0">
          <button @click="toggleMoreMenu" class="btn-ghost !p-1">
            <AppIcon name="more-horizontal" class="h-4 w-4 text-slate-400" />
          </button>
          <Transition name="dropdown">
            <div v-if="showMoreMenu" class="absolute right-0 top-full mt-1 z-[80] bg-white rounded-lg shadow-lg border border-slate-100 py-1 min-w-[140px]">
              <button @click="handleExportProject" class="w-full flex items-center gap-2.5 px-3 py-1.5 text-sm text-left transition-colors hover:bg-slate-50 text-slate-600">
                <AppIcon name="download" class="w-3.5 h-3.5 flex-shrink-0" />
                <span>导出项目</span>
              </button>
              <button v-if="(project.status === 'Won' || project.status === 'Ordered') && !project.isArchived" @click="handleArchiveProject" class="w-full flex items-center gap-2.5 px-3 py-1.5 text-sm text-left transition-colors hover:bg-slate-50 text-slate-600">
                <AppIcon name="archive" class="w-3.5 h-3.5 flex-shrink-0" />
                <span>归档项目</span>
              </button>
              <button @click="handleDeleteProject" class="w-full flex items-center gap-2.5 px-3 py-1.5 text-sm text-left transition-colors hover:bg-red-50 text-red-600">
                <AppIcon name="trash" class="w-3.5 h-3.5 flex-shrink-0" />
                <span>删除项目</span>
              </button>
            </div>
          </Transition>
        </div>
      </div>
    </div>

    <!-- Tab Content -->
    <div class="flex-1 overflow-hidden min-h-0 w-full">
      <div v-if="activeTab === 'detail'" class="h-full">
        <div class="bg-white rounded-xl p-6 h-full overflow-y-auto [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full hover:[&::-webkit-scrollbar-thumb]:bg-gray-300">
          <!-- 项目名称 & 阶段 -->
          <div class="flex items-center gap-4 mb-5 flex-wrap">
            <template v-if="isEditingName">
              <input ref="nameInputRef" v-model="nameEditValue" type="text"
                @blur="saveName" @keyup.enter="saveName" @keyup.escape="cancelEditName"
                class="text-lg font-semibold text-surface-800 bg-white border border-blue-400 rounded-lg px-2 py-1 outline-none focus:ring-2 focus:ring-blue-200 w-full max-w-md" />
            </template>
            <template v-else>
              <div class="flex items-center gap-2 group/name">
                <h2 class="text-lg font-semibold text-surface-800 border border-transparent rounded-lg px-2 py-1">{{ project.name }}</h2>
                <button v-if="!project.isArchived" @click="startEditName" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/name:opacity-100" title="编辑项目名称">
                  <AppIcon name="edit" class="w-4 h-4" />
                </button>
              </div>
            </template>
            <div v-if="!project.isArchived" ref="statusDropdownRef" class="relative">
              <button @click="toggleStatusDropdown"
                class="group/stage inline-flex items-center gap-1 cursor-pointer h-6 px-1.5 rounded transition-colors border border-transparent hover:bg-slate-100/50 hover:border-slate-100 outline-none">
                <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusColors[project.status]" />
                <span class="text-sm font-medium text-slate-600">{{ statusLabels[project.status] || project.status }}</span>
                <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform" :class="showStatusDropdown ? 'rotate-180' : ''" />
                <span v-if="project.status === 'Bidding'" ref="bidCalendarRef" class="relative flex items-center gap-0.5 group/bid-inline ml-1">
                  <span @click.stop="startEditBidTime" class="text-xs cursor-pointer" :class="isBidOverdue ? 'text-purple-500 font-medium' : bidDaysRemaining <= 3 ? 'text-red-500 font-medium' : 'text-slate-400'">
                    {{ formatBidTime(project.bidTime || project.bid_time) || '点击设置投标时间' }}
                  </span>
                  <button v-if="!project.isArchived" @click.stop="startEditBidTime" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/bid-inline:opacity-100 flex-shrink-0" title="编辑投标时间">
                    <AppIcon name="edit" class="w-2.5 h-2.5" />
                  </button>
                  <!-- Calendar Dropdown -->
                  <Transition name="dropdown">
                    <div v-if="showBidCalendar" @click.stop class="absolute top-full left-1/2 -translate-x-1/2 mt-2 z-[80] bg-white/85 backdrop-blur-xl rounded-xl shadow-2xl border border-white/50 p-4 w-[260px]">
                      <!-- Month navigation -->
                      <div class="flex items-center justify-between mb-3">
                        <button @click="prevMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-left" class="w-4 h-4 text-slate-500" />
                        </button>
                        <span class="text-sm font-medium text-slate-700">{{ bidCalendarYear }} 年 {{ bidCalendarMonth + 1 }} 月</span>
                        <button @click="nextMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-right" class="w-4 h-4 text-slate-500" />
                        </button>
                      </div>
                      <!-- Weekday headers -->
                      <div class="grid grid-cols-7 mb-1">
                        <span v-for="w in ['日','一','二','三','四','五','六']" :key="w" class="text-center text-[11px] text-slate-400 py-1">{{ w }}</span>
                      </div>
                      <!-- Day grid -->
                      <div class="grid grid-cols-7 gap-0.5">
                        <div v-for="(day, idx) in calendarDays" :key="idx" class="aspect-square flex items-center justify-center">
                          <button
                            v-if="day !== null"
                            @click="selectBidDay(day)"
                            class="w-8 h-8 rounded-full text-xs transition-colors"
                            :class="[
                              isSelectedDay(day)
                                ? 'bg-blue-600 text-white font-medium'
                                : todayStr === (bidCalendarYear + '-' + String(bidCalendarMonth + 1).padStart(2, '0') + '-' + String(day).padStart(2, '0'))
                                  ? 'text-blue-600 font-medium hover:bg-blue-50'
                                  : 'text-slate-600 hover:bg-slate-100'
                            ]"
                          >{{ day }}</button>
                        </div>
                      </div>
                      <!-- Clear button -->
                      <div class="mt-3 pt-3 border-t border-slate-100 flex justify-between">
                        <button @click="clearBidTime" class="text-[11px] text-slate-400 hover:text-red-500 transition-colors">清除</button>
                        <button @click="closeBidCalendar" class="text-[11px] text-slate-400 hover:text-slate-600 transition-colors">关闭</button>
                      </div>
                    </div>
                  </Transition>
                </span>
              </button>
              <Transition name="dropdown">
                <div v-if="showStatusDropdown" class="absolute top-full left-0 mt-1 z-[80] bg-white rounded-lg shadow-lg border border-slate-100 py-1 min-w-[140px]">
                  <button v-for="status in statusOptions" :key="status" @click="selectStatus(status)"
                    class="w-full flex items-center gap-2.5 px-3 py-1.5 text-sm text-left transition-colors hover:bg-slate-50"
                    :class="project.status === status ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600'">
                    <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusColors[status]" />
                    <span class="flex-1">{{ statusLabels[status] }}</span>
                    <AppIcon v-if="project.status === status" name="check" class="w-3.5 h-3.5 text-blue-600 flex-shrink-0" />
                  </button>
                </div>
              </Transition>
            </div>
          </div>

          <!-- 项目描述 -->
          <div class="mt-4 border-t border-surface-100 pt-4 group/desc">
            <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide mb-2">项目描述</h3>
            <template v-if="isEditingDesc">
              <div>
                <textarea ref="descInputRef" v-model="descEditValue" rows="2"
                  @blur="handleAutoSave"
                  @keydown.escape.prevent="cancelDescEdit"
                  @keyup.ctrl.enter="$event.target.blur()"
                  @input="autoResizeDesc"
                  class="w-full text-sm leading-relaxed text-slate-700 bg-white border border-blue-400 rounded-md px-2 py-1.5 outline-none focus:ring-2 focus:ring-blue-500/10 resize-none placeholder:text-slate-400 min-h-[3.75rem]"
                  placeholder="输入项目描述" />
              </div>
            </template>
            <template v-else>
              <p v-if="project.isArchived" class="text-sm leading-relaxed rounded px-2 py-1.5 border border-transparent min-h-[3.75rem] whitespace-pre-wrap" :class="project.description ? 'text-slate-700' : 'text-slate-400 italic'">
                {{ project.description || '暂未添加项目描述' }}
              </p>
              <p v-else @click="startEditDesc" class="text-sm leading-relaxed cursor-text rounded px-2 py-1.5 border border-transparent min-h-[3.75rem] whitespace-pre-wrap hover:bg-gray-100/60 transition-colors" :class="project.description ? 'text-slate-700' : 'text-slate-400 italic'">
                {{ project.description || '暂未添加项目描述...' }}
              </p>
            </template>
          </div>

          <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide mb-3">基本信息</h3>
          <div class="grid grid-cols-3 gap-x-4">
            <div class="group/budget flex flex-col gap-1">
              <p class="text-xs text-slate-400">预计金额</p>
              <div class="flex items-center gap-0">
                <span class="text-xs text-slate-400 select-none">¥</span>
                <template v-if="isEditingBudget">
                  <input ref="budgetInputRef" v-model="budgetEditValue" type="text"
                    @blur="saveBudget" @keyup.enter="saveBudget" @keyup.escape="cancelEditBudget"
                    :style="{ width: (budgetEditValue.length + 2) + 'ch' }"
                    class="text-sm font-normal text-slate-600 bg-transparent border border-transparent rounded px-0.5 py-0 outline-none focus:border-blue-400 focus:ring-1 focus:ring-blue-200 placeholder:text-slate-300 min-w-[3ch]" />
                </template>
                <template v-else>
                  <span class="text-sm font-normal text-slate-600 rounded px-0.5 py-0 border border-transparent">{{ budgetFormatted || '-' }}</span>
                  <button v-if="!project.isArchived" @click="startEditBudget" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/budget:opacity-100 flex items-center justify-center" title="编辑金额">
                    <AppIcon name="edit" class="w-3.5 h-3.5" />
                  </button>
                </template>
              </div>
            </div>
            <div class="flex flex-col gap-1">
              <p class="text-xs text-slate-400">更新</p>
              <p class="text-sm font-normal text-slate-600">{{ formatDateTime(project.updatedAt || project.createdAt) }}</p>
            </div>
            <div class="flex flex-col gap-1">
              <p class="text-xs text-slate-400">立项</p>
              <p class="text-sm font-normal text-slate-600">{{ formatDateTime(project.createdAt) }}</p>
            </div>
          </div>

          <!-- 客户信息 -->
          <ProjectDetailCustomer :project="project" :is-archived="project.isArchived" :project-id="projectId" @update="handleCustomerUpdate" />

          <!-- 渠道信息 -->
          <div class="mt-4 border-t border-surface-100 pt-4">
            <div class="flex items-center justify-between mb-4">
              <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">渠道信息</h3>
              <button v-if="!project.isArchived" @click="addChannel" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
                <AppIcon name="plus" size="10" stroke="2.5" /> 添加渠道
              </button>
            </div>
            <div v-if="channels.length === 0" class="text-sm text-slate-400 italic py-2">暂未添加渠道</div>
            <div v-for="ch in channels" :key="ch.id" class="card !p-0 overflow-hidden mb-3">
              <!-- Card header -->
              <div @click="toggleChannel(ch.id)" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/ch-card select-none">
                <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 bg-blue-50 text-blue-500">
                  <AppIcon name="building" class="h-4 w-4" />
                </div>
                <div class="flex-1 min-w-0 flex items-center gap-3">
                  <div class="flex items-center gap-1 flex-shrink-0">
                    <p class="text-xs font-medium text-surface-600 whitespace-nowrap">{{ ch.name || '未命名' }}</p>
                  </div>
                </div>
                <div class="flex items-center gap-0.5 flex-shrink-0">
                  <span class="text-[11px] text-slate-400">{{ (ch.contacts || []).length }} 个联系人</span>
                  <button v-if="!project.isArchived" @click.stop="removeChannel(ch.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" title="删除渠道">
                    <AppIcon name="trash" class="w-3 h-3" />
                  </button>
                  <AppIcon name="chevron-right" class="h-3.5 w-3.5 text-surface-300 transition-transform duration-200" :class="expandedChannelId === ch.id ? 'rotate-90' : ''" />
                </div>
              </div>

              <!-- Collapsible contacts section -->
              <div v-show="expandedChannelId === ch.id" class="border-t border-surface-100 bg-gray-50/50">
                <div class="p-4">
                  <div v-if="ch.contacts && ch.contacts.length > 0">
                    <table class="w-full text-xs">
                      <thead>
                        <tr class="text-slate-400 border-b border-slate-200">
                          <th class="text-left font-medium py-2 w-[20%]">联系人</th>
                          <th class="text-left font-medium py-2 w-[18%]">电话</th>
                          <th class="text-left font-medium py-2 w-[18%]">职位</th>
                          <th class="text-left font-medium py-2">邮箱</th>
                        </tr>
                      </thead>
                      <tbody>
                        <tr v-for="ct in (ch.contacts || [])" :key="ct.id" class="border-b border-slate-100 hover:bg-white/60 transition-colors">
                          <td class="py-2 pr-2"><span class="text-xs text-slate-600">{{ ct.name || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.phone || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.position || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.email || '-' }}</span></td>
                        </tr>
                      </tbody>
                    </table>
                  </div>
                  <p v-else class="text-xs text-slate-400 italic">暂无联系人</p>
                  <button v-if="!project.isArchived" @click="startAddContact(ch.id)" class="mt-3 btn-secondary">
                    <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
                  </button>
                </div>
              </div>
            </div>
          </div>

          <!-- 厂商信息 -->
          <div class="mt-4 border-t border-surface-100 pt-4">
            <div class="flex items-center justify-between mb-4">
              <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">厂商信息</h3>
              <button v-if="!project.isArchived" @click="addVendor" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
                <AppIcon name="plus" size="10" stroke="2.5" /> 添加厂商
              </button>
            </div>
            <div v-if="vendors.length === 0" class="text-sm text-slate-400 italic py-2">暂未添加厂商</div>
            <div v-for="vd in vendors" :key="vd.id" class="card !p-0 overflow-hidden mb-3">
              <!-- Card header -->
              <div @click="toggleVendor(vd.id)" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/vd-card select-none">
                <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 bg-amber-50 text-amber-500">
                  <AppIcon name="building" class="h-4 w-4" />
                </div>
                <div class="flex-1 min-w-0 flex items-center gap-3">
                  <div class="flex items-center gap-1 flex-shrink-0">
                    <p class="text-xs font-medium text-surface-600 whitespace-nowrap">{{ vd.name || '未命名' }}</p>
                  </div>
                </div>
                <div class="flex items-center gap-0.5 flex-shrink-0">
                  <span class="text-[11px] text-slate-400">{{ (vd.contacts || []).length }} 个联系人</span>
                  <button v-if="!project.isArchived" @click.stop="removeVendor(vd.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" title="删除厂商">
                    <AppIcon name="trash" class="w-3 h-3" />
                  </button>
                  <AppIcon name="chevron-right" class="h-4 w-4 text-surface-300 transition-transform duration-200" :class="expandedVendorId === vd.id ? 'rotate-90' : ''" />
                </div>
              </div>

              <!-- Collapsible contacts section -->
              <div v-show="expandedVendorId === vd.id" class="border-t border-surface-100 bg-gray-50/50">
                <div class="p-4">
                  <div v-if="vd.contacts && vd.contacts.length > 0">
                    <table class="w-full text-xs">
                      <thead>
                        <tr class="text-slate-400 border-b border-slate-200">
                          <th class="text-left font-medium py-2 w-[20%]">联系人</th>
                          <th class="text-left font-medium py-2 w-[18%]">电话</th>
                          <th class="text-left font-medium py-2 w-[18%]">职位</th>
                          <th class="text-left font-medium py-2">邮箱</th>
                        </tr>
                      </thead>
                      <tbody>
                        <tr v-for="ct in (vd.contacts || [])" :key="ct.id" class="border-b border-slate-100 hover:bg-white/60 transition-colors">
                          <td class="py-2 pr-2"><span class="text-xs text-slate-600">{{ ct.name || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.phone || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.position || '-' }}</span></td>
                          <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.email || '-' }}</span></td>
                        </tr>
                      </tbody>
                    </table>
                  </div>
                  <p v-else class="text-xs text-slate-400 italic">暂无联系人</p>
                  <button v-if="!project.isArchived" @click="startAddVendorContact(vd.id)" class="mt-3 btn-secondary">
                    <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
                  </button>
                </div>
              </div>
            </div>
          </div>

        </div>
      </div>

      <div v-if="activeTab === 'overview'" class="h-full">
        <div class="bg-white rounded-xl p-6 h-full flex flex-col min-h-0">
          <div class="flex items-center justify-between mb-4 shrink-0">
            <div class="flex items-center gap-3">
              <button class="flex items-center gap-1 h-6 px-2 rounded-md text-xs bg-blue-600 text-white hover:bg-blue-700 transition-colors" @click="showEventForm = !showEventForm">
                <AppIcon name="plus" class="w-3 h-3" />
                记录日志
              </button>
            </div>
            <div class="flex items-center gap-1.5">
              <button @click="logFilter = 'all'" class="h-6 px-2 rounded-md text-[11px] font-medium transition-colors" :class="logFilter === 'all' ? 'bg-blue-50 text-blue-600' : 'text-surface-400 hover:text-surface-600 hover:bg-surface-50'">全部</button>
              <button @click="logFilter = 'system'" class="h-6 px-2 rounded-md text-[11px] font-medium transition-colors" :class="logFilter === 'system' ? 'bg-blue-50 text-blue-600' : 'text-surface-400 hover:text-surface-600 hover:bg-surface-50'">系统日志</button>
              <button @click="logFilter = 'note'" class="h-6 px-2 rounded-md text-[11px] font-medium transition-colors" :class="logFilter === 'note' ? 'bg-blue-50 text-blue-600' : 'text-surface-400 hover:text-surface-600 hover:bg-surface-50'">用户日志</button>
            </div>
          </div>

          <div v-if="showEventForm" class="mb-4 p-4 rounded-lg bg-surface-50 border border-surface-200 shrink-0">
            <textarea v-model="newEvent.content" rows="2" class="input-field mb-2" placeholder="发生了什么？例如：完成客户拜访、POC环境部署..." />
            <div class="flex gap-2">
              <button class="flex items-center gap-1 h-6 px-2 rounded-md text-xs bg-blue-600 text-white hover:bg-blue-700 transition-colors disabled:opacity-50" :disabled="isAddingDynamic" @click="addTimelineEvent">{{ isAddingDynamic ? '保存中...' : '保存' }}</button>
              <button class="btn-ghost" @click="showEventForm = false">取消</button>
            </div>
          </div>

          <div class="flex-1 overflow-y-auto min-h-0 pr-2 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full hover:[&::-webkit-scrollbar-thumb]:bg-gray-300">
            <Timeline :events="dynamics" :project-id="projectId" :log-filter="logFilter" />
          </div>
        </div>
      </div>

      <div v-if="activeTab === 'notes'" class="h-full">
        <div class="h-full flex overflow-hidden bg-white">
          <!-- Note list (left) - match NotesPage middle column style -->
          <div class="w-[280px] flex-shrink-0 border-r border-surface-200 flex flex-col bg-white/50">
            <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
              <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">笔记</span>
              <button @click="addProjectNote" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建笔记"><AppIcon name="plus" size="12" stroke="2.5"/></button>
            </div>
            <div class="flex-1 overflow-y-auto px-2 py-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
              <div v-for="note in projectNotes" :key="note.note_id" @click="selectedNoteId = note.note_id"
                class="flex items-center px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden hover:scale-[1.02] hover:z-10 select-none mb-1"
                :class="selectedNoteId === note.note_id ? 'bg-blue-50/70 border-brand-200 shadow-none scale-100' : 'border-surface-100 hover:border-surface-300 hover:shadow-md'">
                <div v-if="selectedNoteId === note.note_id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] font-medium text-surface-800 truncate">{{ note.title || '未命名笔记' }}</p>
                  <p class="text-[10px] text-surface-400 line-clamp-1 mt-0.5">{{ (note.content || '').replace(/<[^>]*>/g,'').slice(0,40) || '暂无内容' }}</p>
                  <p class="text-[9px] text-surface-300 mt-0.5">{{ formatDateTime(note.updated_at || note.created_at || '') }}</p>
                </div>
                <button @click.stop="removeProjectNote(note.note_id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0"><AppIcon name="trash" size="13"/></button>
              </div>
              <div v-if="projectNotes.length === 0" class="text-center py-10 text-surface-400 text-xs">暂无笔记</div>
            </div>
          </div>
          <!-- Note content + file panel -->
          <div class="flex-1 flex min-w-0" v-if="selectedNote">
            <!-- Editor column -->
            <div class="flex-1 flex flex-col bg-white min-w-0">
              <!-- Toolbar (match NotesPage) -->
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
              <!-- @ mention button -->
              <div class="relative">
                <button data-at-btn @mousedown.prevent @click.stop="openAtSelector()" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100" :class="atOpen?'bg-blue-50 text-blue-600':''" title="插入链接">
                  <svg class="w-[15px] h-[15px]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 12v1a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
                </button>
              </div>
              <div class="mx-0.5 w-px h-[14px] bg-[#E5E7EB]"></div>
              <!-- Table button -->
              <div class="relative">
                <button @mousedown.prevent @click.stop="tableMenuOpen=!tableMenuOpen; headingOpen=false; listOpen=false; alignOpen=false; atOpen=false" class="w-7 h-7 flex items-center justify-center rounded transition-colors duration-200 relative group" :class="noteEditor?.isActive('table')?'bg-blue-50 text-blue-600':'text-surface-500 hover:bg-surface-100'">
                  <AppIcon name="table-grid" size="15"/>
                  <svg class="w-2 h-2 ml-0.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
                </button>
                <div v-if="tableMenuOpen" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-lg shadow-sm border border-surface-100 p-1 w-[92px]" @click.stop>
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
            <!-- Title -->
            <div class="px-4 pt-3 pb-1 shrink-0">
              <input v-model="editingNoteTitle" @blur="saveNoteTitle" @keyup.enter="saveNoteTitle" class="w-full text-lg font-semibold text-surface-800 bg-transparent outline-none" placeholder="笔记标题" />
              <hr class="border-surface-200 mt-1.5" />
            </div>
            <!-- Content editor -->
            <div class="flex-1 overflow-y-auto min-h-0 px-4 py-2 prose prose-sm max-w-none">
              <EditorContent :editor="noteEditor" />
            </div>
          </div>
          <!-- File panel column (right) — 分组折叠项目文件 -->
          <div class="flex-shrink-0 w-[300px] border-l border-surface-200 flex flex-col bg-surface-50/20">
            <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
              <span class="text-[10px] font-semibold text-surface-400 uppercase tracking-wider">项目文件</span>
              <span class="text-[9px] text-surface-300">{{ projFilesPanel.length }}</span>
            </div>
            <div class="flex-1 overflow-y-auto p-2">
              <div v-if="projFilesLoading" class="text-center py-4"><p class="text-xs text-surface-400">加载中...</p></div>
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
                      <div class="w-6 h-6 flex items-center justify-center rounded font-bold text-[7px] tracking-wider shrink-0 select-none" :class="panelFileTypeClass(f.file_ext || '')">{{ (f.file_ext || 'file').toUpperCase() }}</div>
                      <div class="ml-1.5 flex-1 min-w-0 flex items-center gap-1">
                        <template v-if="panelEditingFileId === f.id">
                          <input v-model="panelEditingFileName" data-panel-rename-input class="w-full text-[10px] font-medium text-surface-700 bg-transparent outline-none border-b border-brand-400 pb-0.5" @keyup.enter="panelConfirmRename(f)" @blur="panelConfirmRename(f)" @click.stop @keyup.escape="panelCancelRename()" />
                        </template>
                        <template v-else>
                          <p class="text-[10px] font-medium text-surface-700 truncate leading-tight cursor-pointer hover:text-blue-600 flex-1 min-w-0" :title="panelFileDisplayName(f)" @click="panelPreviewFile(f.id)">{{ panelFileDisplayName(f) }}</p>
                        </template>
                        <button @click.stop="noteInsertFileToEditor(f)" class="shrink-0 w-4.5 h-4.5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100 ml-1" title="插入到笔记"><AppIcon name="file-plus" size="12"/></button>
                      </div>
                    </div>
                  </div>
                </div>
              </template>
              <div v-else class="text-center py-8"><p class="text-xs text-surface-400">暂无项目文件</p><p class="text-[10px] text-surface-300 mt-1">前往「文件」tab 上传</p></div>
            </div>
          </div>
          </div>
          <div v-else class="flex-1 flex items-center justify-center text-surface-300 text-xs bg-white min-w-0">从左侧选择一个笔记</div>
        </div>
      </div>

      <div v-if="activeTab === 'todos'" class="h-full">
        <div class="h-full flex overflow-hidden bg-white">
          <!-- Todo list (left) - match TodoPage left column style -->
          <div class="w-[280px] flex-shrink-0 border-r border-surface-200 flex flex-col bg-white/50">
            <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
              <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">待办</span>
              <div class="flex items-center gap-2">
                <span class="text-[10px] text-surface-400 font-medium">{{ projectUndoneTodos.length }}</span>
                <button @click="addProjectTodo" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" title="新建待办"><AppIcon name="plus" size="12" stroke="2.5"/></button>
              </div>
            </div>
            <div style="display:flex;flex-direction:column;flex:1;min-height:0">
              <div class="flex-1 overflow-y-auto px-2 py-1 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
                <div v-for="todo in projectUndoneTodos" :key="todo.id" @click="selectedTodoId = todo.id"
                  class="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden select-none mb-1"
                  :class="['border-surface-100 hover:border-surface-300 hover:shadow-md', selectedTodoId === todo.id ? 'bg-blue-50/70 border-brand-200 shadow-none scale-100' : 'hover:scale-[1.02] hover:z-10']">
                  <div v-if="selectedTodoId === todo.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
                  <button @click.stop="toggleProjectTodo(todo.id)" class="w-5 h-5 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-200 hover:shadow-md"
                    :style="{ borderColor: todo.priority ? ({P1:'#ef4444',P2:'#f59e0b',P3:'#3b82f6'} as any)[todo.priority] : '#d1d5db' }">
                    <AppIcon v-if="todo.done" name="check" class="w-3 h-3 text-white" />
                  </button>
                  <div class="flex-1 min-w-0">
                    <p class="text-[12px] font-medium truncate text-surface-800">{{ todo.title }}</p>
                    <p class="text-[9px] mt-1 text-surface-400" v-if="todo.content">{{ (todo.content || '').replace(/<[^>]*>/g,'').slice(0,40) }}</p>
                    <p class="text-[9px] mt-0.5 text-surface-400" v-if="todo.dueDate">{{ todo.dueDate.slice(0,10) }}</p>
                  </div>
                  <button @click.stop="removeProjectTodo(todo.id)" class="w-4 h-4 flex items-center justify-center rounded-full text-surface-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除"><AppIcon name="trash" size="10"/></button>
                </div>
                <div v-if="projectUndoneTodos.length === 0 && projectDoneTodos.length === 0" class="text-center py-10 text-surface-400 text-xs">暂无待办</div>
              </div>
              <!-- Done drawer -->
              <div v-if="projectDoneTodos.length > 0" class="border-t border-surface-100 bg-white">
                <button @click="projectTodoDoneOpen = !projectTodoDoneOpen" class="w-full flex items-center gap-1.5 px-3 py-2 text-[10px] text-surface-400 hover:text-surface-600 transition-colors">
                  <svg class="w-3 h-3 transition-transform" :class="projectTodoDoneOpen ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 5l7 7-7 7"/></svg>
                  <span>已完成</span>
                  <span class="ml-auto text-surface-300">{{ projectDoneTodos.length }} 项</span>
                </button>
                <div v-show="projectTodoDoneOpen" class="px-2 pb-2 space-y-1 max-h-[200px] overflow-y-auto">
                  <div v-for="todo in projectDoneTodos" :key="todo.id" @click="selectedTodoId = todo.id"
                    class="flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer group border bg-gray-50/80 border-gray-200"
                    :class="selectedTodoId === todo.id ? 'bg-gray-100/70 border-gray-300' : ''">
                    <button @click.stop="toggleProjectTodo(todo.id)" class="w-4 h-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all"
                      style="border-color:#9ca3af;background:#9ca3af">
                      <AppIcon name="check" class="w-2.5 h-2.5 text-white" />
                    </button>
                    <div class="flex-1 min-w-0">
                      <p class="text-[11px] font-medium truncate text-gray-400 line-through">{{ todo.title }}</p>
                      <div class="flex items-center gap-1 mt-0.5">
                        <span class="text-[8px] text-gray-300" v-if="todo.dueDate">{{ todo.dueDate.slice(0,10) }}</span>
                      </div>
                    </div>
                    <button @click.stop="removeProjectTodo(todo.id)" class="w-3.5 h-3.5 flex items-center justify-center rounded-full text-gray-300 hover:text-red-500 hover:bg-red-50 opacity-0 group-hover:opacity-100 transition-all flex-shrink-0" title="删除"><AppIcon name="trash" size="8"/></button>
                  </div>
                </div>
              </div>
            </div>
          </div>
          <!-- Todo detail + file panel -->
          <div class="flex-1 flex min-w-0 overflow-hidden">
            <!-- Todo detail -->
            <div class="flex-1 flex flex-col bg-white min-w-0 overflow-hidden">
              <TodoDetail
                ref="todoDetailRef"
                v-if="selectedTodo"
                :todo="selectedTodo"
                :projects="[{id: project?._backendId || projectId, name: project?.name || ''}]"
                :hide-project-selector="true"
                @update="(id, data) => handleTodoUpdate(id, data)"
                @close="selectedTodoId = null"
              />
              <div v-else class="h-full flex items-center justify-center text-surface-300 text-xs">从左侧选择一个待办</div>
            </div>
            <!-- Todo file panel — 分组折叠项目文件 -->
            <div class="flex-shrink-0 w-[300px] border-l border-surface-200 flex flex-col bg-surface-50/20" v-if="selectedTodo">
              <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
                <span class="text-[10px] font-semibold text-surface-400 uppercase tracking-wider">项目文件</span>
                <span class="text-[9px] text-surface-300">{{ projFilesPanel.length }}</span>
              </div>
              <div class="flex-1 overflow-y-auto p-2">
                <div v-if="projFilesLoading" class="text-center py-4"><p class="text-xs text-surface-400">加载中...</p></div>
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
                        <div class="w-6 h-6 flex items-center justify-center rounded font-bold text-[7px] tracking-wider shrink-0 select-none" :class="panelFileTypeClass(f.file_ext || '')">{{ (f.file_ext || 'file').toUpperCase() }}</div>
                        <div class="ml-1.5 flex-1 min-w-0 flex items-center gap-1">
                          <p class="text-[10px] font-medium text-surface-700 truncate leading-tight cursor-pointer hover:text-blue-600 flex-1 min-w-0" :title="panelFileDisplayName(f)" @click="panelPreviewFile(f.id)">{{ panelFileDisplayName(f) }}</p>
                          <button @click.stop="todoInsertFileToEditor(f)" class="shrink-0 w-4.5 h-4.5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100" title="插入到待办"><AppIcon name="file-plus" size="12"/></button>
                        </div>
                      </div>
                    </div>
                  </div>
                </template>
                <div v-else class="text-center py-8"><p class="text-xs text-surface-400">暂无项目文件</p><p class="text-[10px] text-surface-300 mt-1">前往「文件」tab 上传</p></div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div v-if="activeTab === 'files'" class="h-full">
        <FileManager :project-id="projectId" :key="fileManagerKey" />
      </div>

      <div v-if="activeTab === 'configurator'" class="h-full">
        <SolutionConfigurator :project="project" />
      </div>
    </div>
  </div>

  <!-- Loading / Transition fallback -->
  <div v-else class="h-full flex items-center justify-center">
    <AppIcon name="spinner" class="w-8 h-8 text-surface-300 animate-spin" />
  </div>

</div>

<AtMentionSelector
  v-if="atOpen"
  :projects="allProjects"
  @select-project="insertProjectLink"
  @select-file="insertFileLink"
  @close="atOpen = false"
/>

<!-- 渠道选择弹窗 -->
<Teleport to="body">
  <Transition name="confirm-fade">
    <div v-if="showChannelSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
      <div class="absolute inset-0 bg-black/20" @click="showChannelSelectModal = false" />
      <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
        <h3 class="text-sm font-semibold text-surface-800 mb-4">选择渠道</h3>
        
        <div class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">渠道</label>
          <SelectWithSearch
            v-model="selectedChannelId"
            :options="allChannels.map(c => ({ id: c.id, name: c.name, address: c.address }))"
            placeholder="选择渠道..."
          />
        </div>

        <div v-if="selectedChannel && channelContacts.length > 0" class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
          <SelectWithSearch
            v-model="selectedChannelContactId"
            :options="channelContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
            placeholder="选择联系人..."
          />
        </div>

        <div class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
          <input v-model="channelRemark" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
        </div>

        <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
          <button @click="showChannelSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
          <button @click="confirmChannelSelect" :disabled="!selectedChannel" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

<!-- 厂商选择弹窗 -->
<Teleport to="body">
  <Transition name="confirm-fade">
    <div v-if="showVendorSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
      <div class="absolute inset-0 bg-black/20" @click="showVendorSelectModal = false" />
      <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
        <h3 class="text-sm font-semibold text-surface-800 mb-4">选择厂商</h3>
        
        <div class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">厂商</label>
          <SelectWithSearch
            v-model="selectedVendorId"
            :options="allVendors.map(v => ({ id: v.id, name: v.name, address: v.address }))"
            placeholder="选择厂商..."
          />
        </div>

        <div v-if="selectedVendor && vendorContacts.length > 0" class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
          <SelectWithSearch
            v-model="selectedVendorContactId"
            :options="vendorContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
            placeholder="选择联系人..."
          />
        </div>

        <div class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
          <input v-model="vendorRemark" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
        </div>

        <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
          <button @click="showVendorSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
          <button @click="confirmVendorSelect" :disabled="!selectedVendor" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

<!-- 渠道联系人选择弹窗 -->
<Teleport to="body">
  <Transition name="confirm-fade">
    <div v-if="showContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
      <div class="absolute inset-0 bg-black/20" @click="showContactSelectModal = false" />
      <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
        <h3 class="text-sm font-semibold text-surface-800 mb-4">选择联系人</h3>
        
        <div v-if="availableContacts.length > 0" class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
          <SelectWithSearch
            v-model="selectedContactId"
            :options="availableContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
            placeholder="选择联系人..."
          />
        </div>
        <div v-else class="text-[12px] text-surface-400 py-4 text-center">该渠道暂无联系人</div>

        <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
          <button @click="showContactSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
          <button @click="confirmContactSelect" :disabled="!selectedContactId" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

<!-- 厂商联系人选择弹窗 -->
<Teleport to="body">
  <Transition name="confirm-fade">
    <div v-if="showVendorContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
      <div class="absolute inset-0 bg-black/20" @click="showVendorContactSelectModal = false" />
      <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
        <h3 class="text-sm font-semibold text-surface-800 mb-4">选择联系人</h3>
        
        <div v-if="availableVendorContacts.length > 0" class="mb-4">
          <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
          <SelectWithSearch
            v-model="selectedVendorContactToAdd"
            :options="availableVendorContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
            placeholder="选择联系人..."
          />
        </div>
        <div v-else class="text-[12px] text-surface-400 py-4 text-center">该厂商暂无联系人</div>

        <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
          <button @click="showVendorContactSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
          <button @click="confirmVendorContactSelect" :disabled="!selectedVendorContactToAdd" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>
</template>

<style scoped>
.dropdown-enter-active {
  transition: all 0.15s ease-out;
}
.dropdown-leave-active {
  transition: all 0.1s ease-in;
}
.dropdown-enter-from {
  opacity: 0;
  transform: translateY(-4px) scale(0.97);
}
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.97);
}

</style>

<style>
/* Tiptap editor styles (non-scoped) */
.tiptap-editor { outline: none; min-height: 200px; }
.tiptap-editor p { margin: 0 0 0.5em; font-size: 14px; color: #374151; }
.tiptap-editor h1 { font-size: 1.6em; font-weight: 700; margin: 0.5em 0 0.3em; color: #1f2937; }
.tiptap-editor h2 { font-size: 1.3em; font-weight: 600; margin: 0.4em 0 0.25em; color: #1f2937; }
.tiptap-editor h3 { font-size: 1.1em; font-weight: 600; margin: 0.3em 0 0.2em; color: #1f2937; }
.tiptap-editor ul { list-style-type: disc; padding-left: 1.5em; }
.tiptap-editor ol { list-style-type: decimal; padding-left: 1.5em; }
.tiptap-editor li { margin: 0.2em 0; font-size: 14px; color: #374151; }
.tiptap-editor li p { margin: 0; }
.tiptap-editor ul[data-type="taskList"] { list-style: none; padding-left: 0; }
.tiptap-editor ul[data-type="taskList"] li { display: flex; gap: 0.5em; align-items: flex-start; }
.tiptap-editor ul[data-type="taskList"] li::marker { content: none; }
.tiptap-editor ul[data-type="taskList"] li label { margin-top: 0.15em; flex-shrink: 0; }
.tiptap-editor ul[data-type="taskList"] li div { flex: 1; }
.tiptap-editor ul[data-type="taskList"] li div p { margin: 0; }
.tiptap-editor blockquote { border-left: 3px solid #e2e8f0; padding-left: 1em; margin: 0.5em 0; color: #64748b; }
.tiptap-editor pre { background: #f1f5f9; border-radius: 6px; padding: 12px 16px; font-family: ui-monospace, monospace; font-size: 13px; margin: 0.5em 0; overflow-x: auto; }
.tiptap-editor code { background: #f1f5f9; border-radius: 4px; padding: 1px 5px; font-size: 0.9em; }
.tiptap-editor pre code { background: none; padding: 0; }
.tiptap-editor hr { border: none; border-top: 1px solid #e2e8f0; margin: 1em 0; }
.tiptap-editor table { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
.tiptap-editor table td, .tiptap-editor table th { border: 1px solid #e2e8f0; padding: 6px 10px; font-size: 13px; min-width: 60px; }
.tiptap-editor table th { background: #f8fafc; font-weight: 600; }
.tiptap-editor .selectedCell { background: #e0f2fe; }
.tiptap-editor p.is-editor-empty:first-child::before { content: attr(data-placeholder); float: left; color: #9ca3af; pointer-events: none; height: 0; }
</style>