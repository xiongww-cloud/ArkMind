<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useProjectStore } from '@/stores/project'
import { useSelectionStore } from '@/stores/selection'
import { useAuthStore } from '@/stores/auth'
import { alert, dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { STATUS_OPTIONS, STATUS_LABELS, STATUS_DOT_COLORS } from '@/composables/useProjectStatus'
import { formatDateTime } from '@/utils/time'
import Timeline from '@/components/project/Timeline.vue'
import FileManager from '@/components/project/FileManager.vue'
import SolutionConfigurator from '@/components/project/SolutionConfigurator.vue'
import ProjectDetailCustomer from '@/components/project/ProjectDetailCustomer.vue'
import ProjectChannels from '@/components/project/ProjectChannels.vue'
import ProjectVendors from '@/components/project/ProjectVendors.vue'
import ProjectNotes from '@/components/project/ProjectNotes.vue'
import ProjectTodos from '@/components/project/ProjectTodos.vue'
import AppIcon from '@/components/common/AppIcon.vue'

const props = defineProps<{ id?: string; embedded?: boolean }>()

const route = useRoute()
const router = useRouter()
const projectStore = useProjectStore()
const selectionStore = useSelectionStore()
const authStore = useAuthStore()

const projectId = computed(() => props.id || route.params.id as string)
const project = computed(() => projectStore.getProjectById(projectId.value))

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

const statusLabels = STATUS_LABELS
const statusOptions = STATUS_OPTIONS
const statusColors = STATUS_DOT_COLORS

const showStatusDropdown = ref(false)
const statusDropdownRef = ref(null)
const showMoreMenu = ref(false)
const moreMenuRef = ref(null)

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

async function selectStatus(status: string) {
  if (status === project.value?.status) { showStatusDropdown.value = false; return }
  const oldLabel = statusLabels[project.value.status] || project.value.status
  const ok = await projectStore.updateProjectStatus(projectId.value, status)
  if (!ok) { showStatusDropdown.value = false; return }
  const newLabel = statusLabels[status] || status
  await projectStore.addProjectDynamic(projectId.value, `将项目阶段从「${oldLabel}」变更为「${newLabel}」`)
  showStatusDropdown.value = false
  dynamics.value = projectStore.getProjectDynamics(projectId.value)
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
  const deleted = await projectStore.deleteProject(projectId.value)
  if (!deleted) return
  await router.replace({ name: 'Projects' })
  toast.success('项目已永久删除')
}
function handleClickOutside(e: MouseEvent) {
  if (statusDropdownRef.value && !statusDropdownRef.value.contains(e.target)) showStatusDropdown.value = false
  if (moreMenuRef.value && !moreMenuRef.value.contains(e.target)) showMoreMenu.value = false
  if (bidCalendarRef.value && !bidCalendarRef.value.contains(e.target)) showBidCalendar.value = false
  if (expectedCalendarRef.value && !expectedCalendarRef.value.contains(e.target)) showExpectedCalendar.value = false
}

const isEditingBudget = ref(false)
const budgetEditValue = ref('')
const budgetInputRef = ref(null)
const budgetDisplay = computed(() => String(project.value?.budget || '').replace(/[^0-9.]/g, ''))
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

const isEditingExpectedDate = ref(false)
const expectedDateEditValue = ref('')
const expectedDateInputRef = ref(null)

const showExpectedCalendar = ref(false)
const expectedCalendarRef = ref(null)
const expectedCalendarYear = ref(new Date().getFullYear())
const expectedCalendarMonth = ref(new Date().getMonth())
function startEditExpectedDate() { expectedDateEditValue.value = project.value?.expectedOrderDate || project.value?.expected_order_date || ''; isEditingExpectedDate.value = true; nextTick(() => expectedDateInputRef.value?.focus()) }
async function saveExpectedDate() {
  const v = expectedDateEditValue.value.trim()
  if (project.value) {
    if (v === (project.value.expectedOrderDate || project.value.expected_order_date || '')) { isEditingExpectedDate.value = false; return }
    project.value.expectedOrderDate = v
    project.value.expected_order_date = v
    projectStore.updateProjectInList(projectId.value, { expectedOrderDate: v })
    try {
      await projectStore.updateProject(projectId.value, { expectedOrderDate: v })
    } catch (e) { console.warn('保存预计下单时间失败', e) }
  }
  isEditingExpectedDate.value = false
}
function cancelEditExpectedDate() { isEditingExpectedDate.value = false }

function startEditExpectedDateCalendar() {
  const raw = (project.value?.expectedOrderDate || project.value?.expected_order_date || '')
  if (raw) {
    const d = new Date(raw)
    if (!isNaN(d.getTime())) {
      expectedCalendarYear.value = d.getFullYear()
      expectedCalendarMonth.value = d.getMonth()
    }
  } else {
    const now = new Date()
    expectedCalendarYear.value = now.getFullYear()
    expectedCalendarMonth.value = now.getMonth()
  }
  showExpectedCalendar.value = !showExpectedCalendar.value
}

function prevExpectedMonth() {
  if (expectedCalendarMonth.value === 0) { expectedCalendarMonth.value = 11; expectedCalendarYear.value-- }
  else { expectedCalendarMonth.value-- }
}

function nextExpectedMonth() {
  if (expectedCalendarMonth.value === 11) { expectedCalendarMonth.value = 0; expectedCalendarYear.value++ }
  else { expectedCalendarMonth.value++ }
}

const expectedCalendarDays = computed(() => {
  const year = expectedCalendarYear.value
  const month = expectedCalendarMonth.value
  const firstDay = new Date(year, month, 1).getDay()
  const daysInMonth = new Date(year, month + 1, 0).getDate()
  const days = []
  for (let i = 0; i < firstDay; i++) days.push(null)
  for (let d = 1; d <= daysInMonth; d++) days.push(d)
  return days
})

function isSelectedExpectedDay(day: number) {
  const y = expectedCalendarYear.value
  const m = String(expectedCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  return (project.value?.expectedOrderDate || project.value?.expected_order_date || '').startsWith(`${y}-${m}-${d}`)
}

function closeExpectedCalendar() { showExpectedCalendar.value = false }

async function selectExpectedDay(day: number) {
  const y = expectedCalendarYear.value
  const m = String(expectedCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  const displayTime = `${y}-${m}-${d}`
  showExpectedCalendar.value = false
  if (project.value) {
    project.value.expectedOrderDate = displayTime
    project.value.expected_order_date = displayTime
  }
  projectStore.updateProjectInList(projectId.value, { expectedOrderDate: displayTime })
  try {
    await projectStore.updateProject(projectId.value, { expectedOrderDate: displayTime })
  } catch (e) { console.warn('保存预计下单时间失败', e) }
}

async function clearExpectedDate() {
  showExpectedCalendar.value = false
  const displayTime = ''
  if (project.value) {
    project.value.expectedOrderDate = displayTime
    project.value.expected_order_date = displayTime
  }
  projectStore.updateProjectInList(projectId.value, { expectedOrderDate: '' })
  try {
    await projectStore.updateProject(projectId.value, { expectedOrderDate: '' })
  } catch (e) { console.warn('清除预计下单时间失败', e) }
}

const isEditingName = ref(false)
const nameEditValue = ref('')
const nameInputRef = ref(null)
const fileManagerKey = ref(0)
function startEditName() { nameEditValue.value = project.value?.name || ''; isEditingName.value = true; nextTick(() => nameInputRef.value?.focus()) }
async function saveName() {
  const v = nameEditValue.value.trim()
  if (v && project.value) {
    if (v.length > 30) { toast({ message: '项目名称不能超过 30 个字符', type: 'error' }); return }
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

// 客户信息现在通过合作伙伴系统管理（ProjectDetailCustomer.vue）

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

async function handleCustomerUpdate() {
  dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
}

const showBidCalendar = ref(false)
const bidCalendarRef = ref(null)
const bidCalendarYear = ref(new Date().getFullYear())
const bidCalendarMonth = ref(new Date().getMonth())

function formatBidTime(v: string) {
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

function isSelectedDay(day: number) {
  const y = bidCalendarYear.value
  const m = String(bidCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  return (project.value?.bidTime || project.value?.bid_time || '').startsWith(`${y}-${m}-${d}`)
}

function closeBidCalendar() { showBidCalendar.value = false }

async function selectBidDay(day: number) {
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
  await loadProjectData()
})

watch(() => projectId.value, async () => {
  dynamics.value = []
  activeTab.value = 'detail'
  isEditingName.value = false
  isEditingBudget.value = false
  isEditingDesc.value = false
  showStatusDropdown.value = false
  showMoreMenu.value = false
  showBidCalendar.value = false
  await loadProjectData()
})

async function loadProjectData() {
  const result = await projectStore.fetchProjectDetail(projectId.value)
  if (!result) { router.push('/projects'); return }
  dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
}

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
})

function goBack() { router.push('/projects') }

async function addTimelineEvent() {
  if (!newEvent.value.content.trim() || isAddingDynamic.value) return
  isAddingDynamic.value = true
  try {
    await projectStore.addProjectDynamic(projectId.value, newEvent.value.content, newEvent.value.operator, 'note')
    dynamics.value = projectStore.getProjectDynamics(projectId.value)
    newEvent.value.content = ''
    showEventForm.value = false
  } catch (e: any) {
    alert({ title: '保存失败', message: e.message || '网络异常' })
  } finally {
    isAddingDynamic.value = false
  }
}

async function refreshDynamics() {
  dynamics.value = await projectStore.fetchProjectDynamics(projectId.value)
}

const tabIcons: Record<string, string> = {
  info: 'info',
  clock: 'clock',
  document: 'file',
  cog: 'settings',
  'file-doc': 'file-doc',
  list: 'list'
}

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
</script>

<template>
  <div :class="[props.embedded ? 'flex flex-col h-full' : 'h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden']">
    <div v-if="project" class="flex flex-col h-full overflow-hidden">

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
        <div ref="moreMenuRef" class="relative flex-shrink-0">
          <button @click="toggleMoreMenu" class="btn-ghost !p-1">
            <AppIcon name="more-horizontal" class="h-4 w-4 text-slate-400" />
          </button>
          <Transition name="dropdown">
            <div v-if="showMoreMenu" class="absolute right-0 top-full mt-1 z-[80] bg-white rounded-lg shadow-lg border border-slate-100 py-1 min-w-[140px]">
              <button @click="handleExportProject" class="w-full flex items-center gap-2.5 px-3 py-1.5 text-sm text-left transition-colors hover:bg-slate-50 text-slate-600">
                <AppIcon name="download" class="w-3.5 h-3.5 flex-shrink-0" />
                <span>导出项目</span>
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

    <div class="flex-1 overflow-hidden min-h-0 w-full">
      <div v-if="activeTab === 'detail'" class="h-full">
        <div class="bg-white rounded-xl p-6 h-full overflow-y-auto [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full hover:[&::-webkit-scrollbar-thumb]:bg-gray-300">
          <div class="flex items-center gap-4 mb-5 flex-wrap">
            <template v-if="isEditingName">
              <input ref="nameInputRef" v-model="nameEditValue" type="text" maxlength="30"
                @blur="saveName" @keyup.enter="saveName" @keyup.escape="cancelEditName"
                class="text-lg font-semibold text-surface-800 bg-white border border-blue-400 rounded-lg px-2 py-1 outline-none focus:ring-2 focus:ring-blue-200 w-full max-w-md pr-12" />
              <span class="text-[10px] flex-shrink-0 ml-1" :class="nameEditValue.length >= 30 ? 'text-red-400' : 'text-surface-300'">{{ nameEditValue.length }}/30</span>
            </template>
            <template v-else>
              <div class="flex items-center gap-2 group/name">
                <h2 class="text-lg font-semibold text-surface-800 border border-transparent rounded-lg px-2 py-1">{{ project.name }}</h2>
                <button @click="startEditName" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/name:opacity-100" title="编辑项目名称">
                  <AppIcon name="edit" class="w-4 h-4" />
                </button>
              </div>
            </template>
            <div ref="statusDropdownRef" class="relative">
              <button @click="toggleStatusDropdown"
                class="group/stage inline-flex items-center gap-1 cursor-pointer h-6 px-1.5 rounded transition-colors border border-transparent hover:bg-slate-100/30 hover:border-slate-100 outline-none">
                <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusColors[project.status]" />
                <span class="text-sm font-medium text-slate-600">{{ statusLabels[project.status] || project.status }}</span>
                <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform" :class="showStatusDropdown ? 'rotate-180' : ''" />
                <span v-if="project.status === 'Bidding'" ref="bidCalendarRef" class="relative flex items-center gap-0.5 group/bid-inline ml-1">
                  <span @click.stop="startEditBidTime" class="text-xs cursor-pointer" :class="isBidOverdue ? 'text-purple-300 font-medium' : bidDaysRemaining <= 3 ? 'text-red-300 font-medium' : 'text-slate-400'">
                    {{ formatBidTime(project.bidTime || project.bid_time) || '点击设置投标时间' }}
                  </span>
                  <button @click.stop="startEditBidTime" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/bid-inline:opacity-100 flex-shrink-0" title="编辑投标时间">
                    <AppIcon name="edit" class="w-2.5 h-2.5" />
                  </button>
                  <div v-if="showBidCalendar" @click.stop class="absolute top-full left-1/2 -translate-x-1/2 mt-2 z-[80] bg-white/85 backdrop-blur-xl rounded-xl shadow-2xl border border-white/30 p-4 w-[260px]">
                      <div class="flex items-center justify-between mb-3">
                        <button @click="prevMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-left" class="w-4 h-4 text-slate-500" />
                        </button>
                        <span class="text-sm font-medium text-slate-700">{{ bidCalendarYear }} 年 {{ bidCalendarMonth + 1 }} 月</span>
                        <button @click="nextMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-right" class="w-4 h-4 text-slate-500" />
                        </button>
                      </div>
                      <div class="grid grid-cols-7 mb-1">
                        <span v-for="w in ['日','一','二','三','四','五','六']" :key="w" class="text-center text-[11px] text-slate-400 py-1">{{ w }}</span>
                      </div>
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
                      <div class="mt-3 pt-3 border-t border-slate-100 flex justify-between">
                        <button @click="clearBidTime" class="text-[11px] text-slate-400 hover:text-red-300 transition-colors">清除</button>
                        <button @click="closeBidCalendar" class="text-[11px] text-slate-400 hover:text-slate-600 transition-colors">关闭</button>
                      </div>
                    </div>
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

          <div class="mt-4 border-t border-surface-100 pt-4 group/desc">
            <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide mb-2">项目描述</h3>
            <template v-if="isEditingDesc">
              <div>
                <textarea ref="descInputRef" v-model="descEditValue" rows="2"
                  @blur="handleAutoSave"
                  @keydown.escape.prevent="cancelDescEdit"
                  @keyup.ctrl.enter="$event.target.blur()"
                  @input="autoResizeDesc"
                  class="w-full text-sm leading-relaxed text-slate-700 bg-white border border-blue-400 rounded-md px-2 py-1.5 outline-none focus:ring-2 focus:ring-blue-300/10 resize-none placeholder:text-slate-400 min-h-[3.75rem]"
                  placeholder="输入项目描述" />
              </div>
            </template>
            <template v-else>
              <p @click="startEditDesc" class="text-sm leading-relaxed cursor-text rounded px-2 py-1.5 border border-transparent min-h-[3.75rem] whitespace-pre-wrap hover:bg-gray-100/60 transition-colors" :class="project.description ? 'text-slate-700' : 'text-slate-400 italic'">
                {{ project.description || '暂未添加项目描述...' }}
              </p>
            </template>
          </div>

          <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide mb-3">基本信息</h3>
          <div class="grid grid-cols-4 gap-x-4">
            <div class="group/budget flex flex-col gap-1">
              <p class="text-xs text-slate-400">预计金额</p>
              <div class="flex items-center gap-0">
                <span class="text-xs text-slate-400 select-none">¥</span>
                <template v-if="isEditingBudget">
                  <input ref="budgetInputRef" v-model="budgetEditValue" type="text"
                    @blur="saveBudget" @keyup.enter="saveBudget" @keyup.escape="cancelEditBudget"
                    :style="{ width: (budgetEditValue.length + 2) + 'ch' }"
                    class="text-sm font-normal text-slate-600 bg-transparent border border-transparent rounded px-0.5 py-0 outline-none focus:border-blue-400 focus:ring-1 focus:ring-blue-200 placeholder:text-slate-500 min-w-[3ch]" />
                </template>
                <template v-else>
                  <span class="text-sm font-normal text-slate-600 rounded px-0.5 py-0 border border-transparent">{{ budgetFormatted || '-' }}</span>
                  <button @click="startEditBudget" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/budget:opacity-100 flex items-center justify-center" title="编辑金额">
                    <AppIcon name="edit" class="w-3.5 h-3.5" />
                  </button>
                </template>
              </div>
            </div>
            <div class="group/expectedDate flex flex-col gap-1">
              <p class="text-xs text-slate-400">预计下单</p>
              <div class="flex items-center gap-0">
                <span ref="expectedCalendarRef" class="relative flex items-center gap-0.5 group/expected-inline">
                  <span @click.stop="startEditExpectedDateCalendar" class="text-sm font-normal text-slate-600 rounded px-0.5 py-0 border border-transparent cursor-pointer">
                    {{ (project.expectedOrderDate || project.expected_order_date) || '-' }}
                  </span>
                  <button @click.stop="startEditExpectedDateCalendar" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/expectedDate:opacity-100 flex items-center justify-center" title="编辑预计下单时间">
                    <AppIcon name="edit" class="w-3.5 h-3.5" />
                  </button>
                  <div v-if="showExpectedCalendar" @click.stop class="absolute top-full left-1/2 -translate-x-1/2 mt-2 z-[80] bg-white/85 backdrop-blur-xl rounded-xl shadow-2xl border border-white/30 p-4 w-[260px]">
                      <div class="flex items-center justify-between mb-3">
                        <button @click="prevExpectedMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-left" class="w-4 h-4 text-slate-500" />
                        </button>
                        <span class="text-sm font-medium text-slate-700">{{ expectedCalendarYear }} 年 {{ expectedCalendarMonth + 1 }} 月</span>
                        <button @click="nextExpectedMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                          <AppIcon name="chevron-right" class="w-4 h-4 text-slate-500" />
                        </button>
                      </div>
                      <div class="grid grid-cols-7 mb-1">
                        <span v-for="w in ['日','一','二','三','四','五','六']" :key="w" class="text-center text-[11px] text-slate-400 py-1">{{ w }}</span>
                      </div>
                      <div class="grid grid-cols-7 gap-0.5">
                        <div v-for="(day, idx) in expectedCalendarDays" :key="idx" class="aspect-square flex items-center justify-center">
                          <button
                            v-if="day !== null"
                            @click="selectExpectedDay(day)"
                            class="w-8 h-8 rounded-full text-xs transition-colors"
                            :class="[
                              isSelectedExpectedDay(day)
                                ? 'bg-blue-600 text-white font-medium'
                                : todayStr === (expectedCalendarYear + '-' + String(expectedCalendarMonth + 1).padStart(2, '0') + '-' + String(day).padStart(2, '0'))
                                  ? 'text-blue-600 font-medium hover:bg-blue-50'
                                  : 'text-slate-600 hover:bg-slate-100'
                            ]"
                          >{{ day }}</button>
                        </div>
                      </div>
                      <div class="mt-3 pt-3 border-t border-slate-100 flex justify-between">
                        <button @click="clearExpectedDate" class="text-[11px] text-slate-400 hover:text-red-300 transition-colors">清除</button>
                        <button @click="closeExpectedCalendar" class="text-[11px] text-slate-400 hover:text-slate-600 transition-colors">关闭</button>
                      </div>
                    </div>
                </span>
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

          <ProjectDetailCustomer :project="project" :project-id="projectId" @update="handleCustomerUpdate" />

          <ProjectChannels :project="project" :project-id="projectId" @update="dynamics = projectStore.getProjectDynamics(projectId)" />

          <ProjectVendors :project="project" :project-id="projectId" @update="dynamics = projectStore.getProjectDynamics(projectId)" />

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
              <button @click="logFilter = 'ai'" class="h-6 px-2 rounded-md text-[11px] font-medium transition-colors" :class="logFilter === 'ai' ? 'bg-blue-50 text-blue-600' : 'text-surface-400 hover:text-surface-600 hover:bg-surface-50'">AI日志</button>
            </div>
          </div>

          <div v-if="showEventForm" class="mb-4 p-4 rounded-lg bg-surface-50 border border-surface-200 shrink-0">
            <textarea v-model="newEvent.content" rows="2" class="input-field mb-2" placeholder="发生了什么？例如：完成客户拜访、POC环境部署..." />
            <div class="flex gap-2">
              <button class="flex items-center gap-1 h-6 px-2 rounded-md text-xs bg-blue-600 text-white hover:bg-blue-700 transition-colors disabled:opacity-30" :disabled="isAddingDynamic" @click="addTimelineEvent">{{ isAddingDynamic ? '保存中...' : '保存' }}</button>
              <button class="btn-ghost" @click="showEventForm = false">取消</button>
            </div>
          </div>

          <div class="flex-1 overflow-y-auto min-h-0 pr-2 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full hover:[&::-webkit-scrollbar-thumb]:bg-gray-300">
            <Timeline :events="dynamics" :project-id="projectId" :log-filter="logFilter" @refresh="refreshDynamics" />
          </div>
        </div>
      </div>

      <div v-if="activeTab === 'notes'" class="h-full">
        <ProjectNotes :project-id="projectId" />
      </div>

      <div v-if="activeTab === 'todos'" class="h-full">
        <ProjectTodos :project-id="projectId" :project-name="project?.name || ''" />
      </div>

      <div v-if="activeTab === 'files'" class="h-full">
        <FileManager :project-id="projectId" :key="fileManagerKey" />
      </div>

      <div v-if="activeTab === 'configurator'" class="h-full">
        <SolutionConfigurator :project="project" />
      </div>
    </div>
  </div>

  <div v-else class="h-full flex items-center justify-center">
    <AppIcon name="spinner" class="w-8 h-8 text-surface-300 animate-spin" />
  </div>

</div>
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
