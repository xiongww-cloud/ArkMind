<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onDeactivated, onUnmounted, nextTick, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { useRouter, useRoute } from 'vue-router'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'
import { formatDateTime } from '@/utils/time'
import { toast } from '@/composables/useToast'
import { STATUS_OPTIONS, STATUS_LABELS, STATUS_DOT_COLORS, statusBadgeClass } from '@/composables/useProjectStatus'

import { fetchProjects, customerList, customerLinkProject } from '@/api/db-client'

const projectStore = useProjectStore()
const authStore = useAuthStore()
const router = useRouter()
const route = useRoute()

// ─── Table scroll ────────────────────────────────────────────────
const tableScrollRef = ref<HTMLElement | null>(null)
let savedScrollTop = 0

// ─── Filters ─────────────────────────────────────────────────────
const searchInput = ref('')
const searchQuery = ref('')
const selectedStatuses = ref(new Set<string>()) // 多选阶段
const customerFilter = ref('all')
const budgetFilter = ref('all')
const timeFilter = ref('all')
const sortField = ref('updatedAt')
const sortOrder = ref('desc')

let searchTimer: ReturnType<typeof setTimeout> | null = null
watch(searchInput, (v) => {
  if (searchTimer) clearTimeout(searchTimer)
  searchTimer = setTimeout(() => { searchQuery.value = v }, 200)
})

// ─── Dropdowns ───────────────────────────────────────────────────
const showNewProjectModal = ref(false)
const newProjectStatusOpen = ref(false)
const statusDropdownOpen = ref(false)
const budgetDropdownOpen = ref(false)
const timeDropdownOpen = ref(false)

// ─── Async states ────────────────────────────────────────────────
const exporting = ref(false)
const importing = ref(false)

// ─── Customers (for form select) ─────────────────────────────────
const customers = ref<any[]>([])
const customerSelectOptions = computed(() => customers.value.map(c => ({ id: c.id, name: c.name })))
async function loadCustomers() {
  try { customers.value = await customerList() || [] } catch (e) { console.warn('加载客户列表失败', e) }
}

// ─── Multi-select batch export ───────────────────────────────────
const selectionMode = ref(false)
const selectedProjectIds = ref(new Set())

function toggleSelectionMode() {
  selectionMode.value = !selectionMode.value
  if (!selectionMode.value) selectedProjectIds.value = new Set()
}

function toggleProjectSelection(projectId) {
  const next = new Set(selectedProjectIds.value)
  if (next.has(projectId)) next.delete(projectId)
  else next.add(projectId)
  selectedProjectIds.value = next
}

function isProjectSelected(projectId) {
  return selectedProjectIds.value.has(projectId)
}

async function handleBatchExport() {
  if (selectedProjectIds.value.size === 0) {
    toast.error('请至少选择一个项目')
    return
  }
  if (exporting.value) return
  exporting.value = true
  try {
    const ids = [...selectedProjectIds.value]
    const { save: saveDialog } = await import('@tauri-apps/plugin-dialog')
    const filePath = await saveDialog({
      defaultPath: `project_export_${ids.length}.ark`,
      filters: [{ name: 'ArkMind 项目包', extensions: ['ark'] }],
    })
    if (!filePath) return
    const { batchExportProjects } = await import('@/api/db-client')
    const result = await batchExportProjects(ids, filePath)
    if (!result.success) {
      toast.error(result.error || '导出失败')
      return
    }
    toast.success(`已导出 ${ids.length} 个项目至：${filePath}`)
    toggleSelectionMode()
  } catch (e) {
    console.error('[export] error:', e)
    toast.error((e && e.message) || '导出失败')
  } finally {
    exporting.value = false
  }
}

// ─── Backend sync ────────────────────────────────────────────────
async function syncFromBackend() {
  try {
    const remote = await fetchProjects()
    if (!remote?.length) return
    const remoteArr = Array.isArray(remote) ? remote : []
    const store = projectStore.projects
    const remoteIds = new Set(remoteArr.map(r => r.id))
    const idMap = projectStore.loadBackendIdMap()

    for (const r of remote) {
      const match = store.find(p => p._backendId === r.id || p.name === r.name)
      if (match) {
        match._backendId = r.id
        match.name = r.name
        match.budget = r.budget || ''
        match.description = r.description || ''
        match.status = r.status
        match.owner = r.owner || ''
        match.userId = r.userId || ''
        match.displayId = r.displayId || ''
        match._customerName = r.customer_name || ''
        match.createdAt = r.created_at || match.createdAt
        match.updatedAt = r.updated_at || null
        match.bidTime = r.bid_time || ''
        match.expectedOrderDate = r.expected_order_date || ''
        idMap[match.id] = r.id
      } else {
        store.push({
          id: r.id, _backendId: r.id,
          name: r.name, budget: r.budget || '',
          description: r.description || '',
          status: r.status, owner: r.owner || '',
          userId: r.userId || '', displayId: r.displayId || '',
          _customerName: r.customer_name || '',
          createdAt: r.created_at || new Date().toISOString().slice(0, 10),
          updatedAt: r.updated_at || null,
          bidTime: r.bid_time || '',
          expectedOrderDate: r.expected_order_date || '',
          timeline: [], configurations: [], plans: [],
        })
      }
    }
    projectStore.saveBackendIdMap(idMap)

    for (let i = store.length - 1; i >= 0; i--) {
      const p = store[i]
      if (p._backendId && !remoteIds.has(p._backendId)) store.splice(i, 1)
    }
    for (const r of remote) {
      const candidates = store.filter(p => p._backendId === r.id || p.name === r.name)
      if (candidates.length > 1) {
        const keep = candidates.find(p => p.id.startsWith('PRO-')) || candidates[0]
        for (const dup of candidates) {
          if (dup !== keep) { const idx = store.indexOf(dup); if (idx !== -1) store.splice(idx, 1) }
        }
      }
    }
  } catch (e) {
    console.warn('ProjectHub 后端同步失败', e)
  }
}

onMounted(() => {
  const s = route.query.status
  if (s && typeof s === 'string' && allStatusOptions.includes(s)) {
    selectedStatuses.value = new Set([s])
  }
  syncFromBackend()
  loadCustomers()
})
onActivated(() => {
  syncFromBackend()
  nextTick(() => {
    if (tableScrollRef.value) tableScrollRef.value.scrollTop = savedScrollTop
  })
})
onDeactivated(() => {
  if (tableScrollRef.value) savedScrollTop = tableScrollRef.value.scrollTop
})

// ─── Status definitions ──────────────────────────────────────────
const allStatusOptions = STATUS_OPTIONS
const statusDotColors = STATUS_DOT_COLORS
const statusLabels = STATUS_LABELS

// ─── Filter options ──────────────────────────────────────────────
const timeFilterLabels = { all: '全部时间', today: '今天', week: '最近 7 天', month: '最近 30 天', quarter: '最近 90 天' }
const budgetOptions = { all: '全部金额', '0-100': '0~100万', '100-300': '100~300万', '300-1000': '300~1000万', '1000+': '1000万以上' }

function closeAllDropdowns(except?: 'status' | 'budget' | 'time' | 'quarter') {
  if (except !== 'status') statusDropdownOpen.value = false
  if (except !== 'budget') budgetDropdownOpen.value = false
  if (except !== 'time') timeDropdownOpen.value = false
  if (except !== 'quarter') quarterDropdownOpen.value = false
}

// ─── Expected Date Calendar ──────────────────────────────────────
const showExpectedDateCalendar = ref(false)
const expectedDateCalendarRef = ref(null)
const expectedDateCalendarYear = ref(new Date().getFullYear())
const expectedDateCalendarMonth = ref(new Date().getMonth())
const expectedDateCalendarProjectId = ref('')
const todayStr = computed(() => {
  const now = new Date()
  return now.getFullYear() + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
})

function startEditExpectedDateCalendar(project: any, e: MouseEvent) {
  e.stopPropagation()
  const raw = (project.expectedOrderDate || project.expected_order_date || '')
  if (raw) {
    const d = new Date(raw)
    if (!isNaN(d.getTime())) {
      expectedDateCalendarYear.value = d.getFullYear()
      expectedDateCalendarMonth.value = d.getMonth()
    }
  } else {
    const now = new Date()
    expectedDateCalendarYear.value = now.getFullYear()
    expectedDateCalendarMonth.value = now.getMonth()
  }
  expectedDateCalendarProjectId.value = project.id
  showExpectedDateCalendar.value = true
}

function prevExpectedDateMonth() {
  if (expectedDateCalendarMonth.value === 0) { expectedDateCalendarMonth.value = 11; expectedDateCalendarYear.value-- }
  else { expectedDateCalendarMonth.value-- }
}

function nextExpectedDateMonth() {
  if (expectedDateCalendarMonth.value === 11) { expectedDateCalendarMonth.value = 0; expectedDateCalendarYear.value++ }
  else { expectedDateCalendarMonth.value++ }
}

const expectedDateCalendarDays = computed(() => {
  const year = expectedDateCalendarYear.value
  const month = expectedDateCalendarMonth.value
  const firstDay = new Date(year, month, 1).getDay()
  const daysInMonth = new Date(year, month + 1, 0).getDate()
  const days = []
  for (let i = 0; i < firstDay; i++) days.push(null)
  for (let d = 1; d <= daysInMonth; d++) days.push(d)
  return days
})

function getSelectedProject() {
  return filteredProjects.value.find(p => p.id === expectedDateCalendarProjectId.value)
}

function isSelectedExpectedDay(day: number) {
  const project = getSelectedProject()
  if (!project) return false
  const y = expectedDateCalendarYear.value
  const m = String(expectedDateCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  return (project.expectedOrderDate || project.expected_order_date || '').startsWith(`${y}-${m}-${d}`)
}

function closeExpectedDateCalendar() { showExpectedDateCalendar.value = false }

async function selectExpectedDay(day: number) {
  const project = getSelectedProject()
  if (!project) return
  const y = expectedDateCalendarYear.value
  const m = String(expectedDateCalendarMonth.value + 1).padStart(2, '0')
  const d = String(day).padStart(2, '0')
  const displayTime = `${y}-${m}-${d}`
  showExpectedDateCalendar.value = false
  project.expectedOrderDate = displayTime
  project.expected_order_date = displayTime
  projectStore.updateProjectInList(project.id, { expected_order_date: displayTime })
  try {
    await projectStore.updateProject(project.id, { expected_order_date: displayTime })
  } catch (e) { console.warn('保存预计下单时间失败', e) }
}

async function clearExpectedDate() {
  const project = getSelectedProject()
  if (!project) return
  showExpectedDateCalendar.value = false
  project.expectedOrderDate = ''
  project.expected_order_date = ''
  projectStore.updateProjectInList(project.id, { expected_order_date: '' })
  try {
    await projectStore.updateProject(project.id, { expected_order_date: '' })
  } catch (e) { console.warn('清除预计下单时间失败', e) }
}

function handleClickOutside(e: MouseEvent) {
  if (expectedDateCalendarRef.value && !expectedDateCalendarRef.value.contains(e.target)) {
    showExpectedDateCalendar.value = false
  }
}

onMounted(() => { document.addEventListener('click', handleClickOutside) })
onUnmounted(() => { document.removeEventListener('click', handleClickOutside) })

function toggleStatus(status) {
  const next = new Set(selectedStatuses.value)
  next.has(status) ? next.delete(status) : next.add(status)
  selectedStatuses.value = next
}
function selectAllStatuses() {
  if (selectedStatuses.value.size === allStatusOptions.length) {
    selectedStatuses.value = new Set()
  } else {
    selectedStatuses.value = new Set(allStatusOptions)
  }
}

const hasActiveFilters = computed(() =>
  searchQuery.value.trim() !== '' ||
  selectedStatuses.value.size > 0 ||
  customerFilter.value !== 'all' ||
  budgetFilter.value !== 'all' ||
  timeFilter.value !== 'all'
)
function resetFilters() {
  searchInput.value = ''
  searchQuery.value = ''
  selectedStatuses.value = new Set()
  customerFilter.value = 'all'
  budgetFilter.value = 'all'
  timeFilter.value = 'all'
}

// ─── Order Forecast ──────────────────────────────────────────────
const currentYear = new Date().getFullYear()
const selectedQuarter = ref('')
const quarterDropdownOpen = ref(false)
const quarterOptions = [
  { key: 'Q1', label: 'Q1（1-3月）', months: [1, 2, 3] },
  { key: 'Q2', label: 'Q2（4-6月）', months: [4, 5, 6] },
  { key: 'Q3', label: 'Q3（7-9月）', months: [7, 8, 9] },
  { key: 'Q4', label: 'Q4（10-12月）', months: [10, 11, 12] },
]
const quarterLabel = computed(() => quarterOptions.find(o => o.key === selectedQuarter.value)?.label || '全部季度')

// ─── Project filtering ───────────────────────────────────────────
const myProjects = computed(() => {
  const all = Array.isArray(projectStore.projects) ? projectStore.projects : []
  const userId = authStore.currentUser.id
  const userName = authStore.currentUser.name
  return all.filter(p => {
    if (p.userId && p.userId === userId) return true
    if (p.owner === userName) return true
    if (!p.userId) return true
    return false
  })
})

const filteredProjects = computed(() => {
  let result = myProjects.value

  // Status filter (multi-select)
  if (selectedStatuses.value.size > 0 && selectedStatuses.value.size < allStatusOptions.length) {
    result = result.filter(p => selectedStatuses.value.has(p.status))
  }

  // Search filter (name + description + customer + owner)
  if (searchQuery.value.trim()) {
    const q = searchQuery.value.toLowerCase()
    result = result.filter(p =>
      p.name.toLowerCase().includes(q) ||
      (p.description || '').toLowerCase().includes(q) ||
      (p._customerName || '').toLowerCase().includes(q) ||
      (p.owner || '').toLowerCase().includes(q)
    )
  }

  // Customer filter
  if (customerFilter.value !== 'all') {
    result = result.filter(p =>
      customerFilter.value === '__none__' ? !p._customerName : p._customerName === customerFilter.value
    )
  }

  // Budget filter
  if (budgetFilter.value !== 'all') {
    const ranges = { '0-100': [0, 100], '100-300': [100, 300], '300-1000': [300, 1000], '1000+': [1000, Infinity] }
    const [min, max] = ranges[budgetFilter.value] || [0, Infinity]
    result = result.filter(p => { const b = Number(p.budget) || 0; return b >= min && b < max })
  }

  // Time range filter (by updatedAt)
  if (timeFilter.value !== 'all') {
    const now = new Date()
    let cutoff = new Date()
    switch (timeFilter.value) {
      case 'today': cutoff = new Date(now.getFullYear(), now.getMonth(), now.getDate()); break
      case 'week': cutoff.setDate(now.getDate() - 7); break
      case 'month': cutoff.setDate(now.getDate() - 30); break
      case 'quarter': cutoff.setDate(now.getDate() - 90); break
    }
    result = result.filter(p => {
      const t = p.updatedAt || p.createdAt
      if (!t) return false
      let ds = t
      if (!t.includes('T') && !t.includes('Z') && !t.includes('+')) {
        if (t.includes(' ')) ds = t.replace(' ', 'T') + 'Z'
        else if (/^\d{4}-\d{2}-\d{2}$/.test(t)) ds = t + 'T00:00:00Z'
      }
      return new Date(ds) >= cutoff
    })
  }

  // Quarter filter (expected order date)
  if (selectedQuarter.value) {
    const q = quarterOptions.find(o => o.key === selectedQuarter.value)
    if (q) {
      result = result.filter(p => {
        const d = p.expectedOrderDate || p.expected_order_date
        if (!d) return false
        const date = new Date(d)
        if (date.getFullYear() !== currentYear) return false
        return q.months.includes(date.getMonth() + 1)
      })
    }
  }

  // Sort
  return [...result].sort((a, b) => {
    const mul = sortOrder.value === 'desc' ? -1 : 1
    let va = '', vb = ''
    if (sortField.value === 'budget') {
      va = String(Number(a.budget) || 0)
      vb = String(Number(b.budget) || 0)
      return (Number(va) - Number(vb)) * mul
    } else if (sortField.value === 'bidTime') {
      va = a.bidTime || ''
      vb = b.bidTime || ''
    } else if (sortField.value === 'name') {
      va = (a.name || '').toLowerCase()
      vb = (b.name || '').toLowerCase()
      return va.localeCompare(vb) * mul
    } else if (sortField.value === 'expected_order_date') {
      va = a.expectedOrderDate || a.expected_order_date || ''
      vb = b.expectedOrderDate || b.expected_order_date || ''
    } else {
      va = a[sortField.value] || a.createdAt || ''
      vb = b[sortField.value] || b.createdAt || ''
    }
    return va > vb ? 1 * mul : va < vb ? -1 * mul : 0
  })
})

// ─── Summary footer ──────────────────────────────────────────────
const footerSummary = computed(() => {
  const list = filteredProjects.value
  const total = list.reduce((s, p) => s + (Number(p.budget) || 0), 0)
  return {
    count: list.length,
    totalBudget: total,
    avgBudget: list.length > 0 ? Math.round(total / list.length) : 0,
  }
})

// ─── Helpers ─────────────────────────────────────────────────────
function formatBudget(raw) { if (!raw || raw === 0) return '-'; return '¥' + raw + '万' }
function formatBidTime(raw) { if (!raw) return '-'; return formatDateTime(raw) }
function goToProject(id) { router.push(`/projects/${id}`) }

// ─── Inline Edit ────────────────────────────────────────────────
const editingField = ref<{ projectId: string; field: 'name' | 'budget' | 'status' } | null>(null)
const editValue = ref('')
const statusMenuProjectId = ref<string | null>(null)
const statusMenuRect = ref<DOMRect | null>(null)

function startEditName(project) {
  editingField.value = { projectId: project.id, field: 'name' }
  editValue.value = project.name || ''
  statusMenuProjectId.value = null
}
function startEditBudget(project) {
  editingField.value = { projectId: project.id, field: 'budget' }
  editValue.value = String(project.budget || '')
  statusMenuProjectId.value = null
}
function cancelEdit() {
  editingField.value = null
  editValue.value = ''
}
async function saveEdit(project) {
  if (!editingField.value || editingField.value.projectId !== project.id) return
  const field = editingField.value.field
  const v = editValue.value.trim()
  if (field === 'name') {
    if (!v) { cancelEdit(); return }
    if (v.length > 30) { toast.error('项目名称不能超过 30 个字符'); return }
    if (v === project.name) { cancelEdit(); return }
    const q = v.toLowerCase()
    const duplicate = projectStore.projects.some((p: any) => p.id !== project.id && p.name && p.name.toLowerCase() === q)
    if (duplicate) { toast.error('已存在同名项目'); return }
    project.name = v
    projectStore.updateProjectInList(project.id, { name: v })
    try { await projectStore.updateProject(project.id, { name: v }) } catch (e) { console.warn(e) }
  } else if (field === 'budget') {
    const clean = v.replace(/[^0-9.]/g, '') || '0'
    if (clean === String(project.budget || '')) { cancelEdit(); return }
    project.budget = clean
    projectStore.updateProjectInList(project.id, { budget: clean })
    try { await projectStore.updateProject(project.id, { budget: clean }) } catch (e) { console.warn(e) }
  }
  editingField.value = null
  editValue.value = ''
}
async function changeStatus(project, status) {
  statusMenuProjectId.value = null
  if (!status || project.status === status) return
  project.status = status
  projectStore.updateProjectInList(project.id, { status })
  try { await projectStore.updateProject(project.id, { status }) } catch (e) { console.warn(e) }
}

function toggleSort(field) {
  if (sortField.value === field) { sortOrder.value = sortOrder.value === 'desc' ? 'asc' : 'desc' }
  else { sortField.value = field; sortOrder.value = 'desc' }
}
function sortIcon(field) { if (sortField.value !== field) return '↕'; return sortOrder.value === 'desc' ? '↓' : '↑' }

// ─── Create project ──────────────────────────────────────────────
const projectForm = ref({ name: '', budget: 0, description: '', status: 'Lead', bidTime: '', customerId: '' })

function resetForm() {
  projectForm.value = { name: '', budget: 0, description: '', status: 'Lead', bidTime: '', customerId: '' }
}

function openCreate() {
  resetForm()
  showNewProjectModal.value = true
}

function closeModal() {
  showNewProjectModal.value = false
  newProjectStatusOpen.value = false
}

function validateForm(): boolean {
  const f = projectForm.value
  if (!f.name.trim()) { toast.error('请输入项目名称'); return false }
  if (f.name.trim().length > 30) { toast.error('项目名称不能超过 30 个字符'); return false }
  if (!f.status) { toast.error('请选择项目阶段'); return false }
  if (f.status === 'Bidding' && !f.bidTime) { toast.error('投标中阶段必须选择投标时间'); return false }
  const q = f.name.trim().toLowerCase()
  const duplicate = projectStore.projects.some((p: any) => p.name && p.name.toLowerCase() === q)
  if (duplicate) { toast.error('已存在同名项目'); return false }
  return true
}

async function createProject() {
  if (!validateForm()) return
  const f = projectForm.value
  const customerId = f.customerId
  const id = projectStore.addProject({
    name: f.name.trim(), budget: f.budget, description: f.description,
    status: f.status, owner: authStore.currentUser.name,
    userId: authStore.currentUser.id, bidTime: f.bidTime,
  })
  closeModal()
  if (customerId) {
    try {
      await syncFromBackend()
      const backendId = projectStore.loadBackendIdMap()[id]
      if (backendId) {
        await customerLinkProject(customerId, backendId)
        await syncFromBackend()
      }
    } catch (e) { console.warn('客户关联失败', e) }
  }
  router.push(`/projects/${id}`)
}

// ─── Import ──────────────────────────────────────────────────────
async function handleImportProject() {
  if (importing.value) return
  importing.value = true
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({ filters: [{ name: 'ArkMind 项目包', extensions: ['ark'] }], multiple: false })
    if (!selected) return
    const filePath = Array.isArray(selected) ? selected[0] : selected
    if (!filePath) return
    const { importProject } = await import('@/api/db-client')
    const result = await importProject({ filePath })
    if (result.success) { toast.success('项目导入成功'); await syncFromBackend() }
    else { toast.error(String(result.error || '') || '导入失败') }
  } catch (e) { toast.error('导入失败: ' + String(e?.message || e || '')) } finally { importing.value = false }
}
</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm flex flex-col overflow-hidden">
    <div class="flex flex-col h-full py-4">

    <!-- ═══ Filter Bar ═══ -->
    <div class="flex flex-wrap items-center gap-2 px-4 mb-3">
      <button class="btn-primary flex-shrink-0" @click="openCreate">
        <AppIcon name="plus-thin" class="w-3.5 h-3.5" />
        新建项目
      </button>

      <div class="relative w-[200px] flex-shrink-0">
        <AppIcon name="search" class="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-surface-400" />
        <input v-model="searchInput" type="text" placeholder="搜索名称、描述、客户..." class="w-full h-7 pl-8 pr-3 text-xs text-surface-700 bg-white border border-slate-200 rounded-md outline-none focus:border-slate-200 focus:ring-0 placeholder:text-surface-400" />
      </div>

      <!-- Status multi-select dropdown -->
      <div class="relative flex-shrink-0" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) statusDropdownOpen = false }">
        <button @click="statusDropdownOpen = !statusDropdownOpen; closeAllDropdowns('status')" class="flex items-center gap-1 h-7 text-xs bg-white border border-slate-200 rounded-md px-2.5 outline-none transition-colors" :class="selectedStatuses.size > 0 ? 'border-blue-400 text-blue-600' : 'text-slate-600'">
          <span>{{ selectedStatuses.size > 0 ? `已选 ${selectedStatuses.size} 项` : '全部阶段' }}</span>
          <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="statusDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="statusDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[140px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button @click="selectAllStatuses" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs transition-colors" :class="selectedStatuses.size === allStatusOptions.length ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span class="w-4 h-4 rounded border flex items-center justify-center" :class="selectedStatuses.size === allStatusOptions.length ? 'bg-blue-600 border-blue-600' : 'border-slate-300'">
              <AppIcon v-if="selectedStatuses.size === allStatusOptions.length" name="check" class="w-3 h-3 text-white" />
            </span>
            全部阶段
          </button>
          <div class="border-t border-slate-100 my-1" />
          <button v-for="s in allStatusOptions" :key="s" @click="toggleStatus(s)" class="w-full flex items-center gap-2 px-3 py-1.5 text-xs transition-colors" :class="selectedStatuses.has(s) ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span class="w-4 h-4 rounded border flex items-center justify-center" :class="selectedStatuses.has(s) ? 'bg-blue-600 border-blue-600' : 'border-slate-300'">
              <AppIcon v-if="selectedStatuses.has(s)" name="check" class="w-3 h-3 text-white" />
            </span>
            <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[s] || 'bg-gray-400'" />
            <span>{{ statusLabels[s] }}</span>
          </button>
        </div>
      </div>

      <!-- Budget filter -->
      <div class="relative flex-shrink-0" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) budgetDropdownOpen = false }">
        <button @click="budgetDropdownOpen = !budgetDropdownOpen; closeAllDropdowns('budget')" class="flex items-center gap-1 h-7 text-xs bg-white border border-slate-200 rounded-md px-2.5 outline-none transition-colors" :class="budgetFilter !== 'all' ? 'border-blue-400 text-blue-600' : 'text-slate-600'">
          <span>{{ budgetOptions[budgetFilter] }}</span>
          <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="budgetDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="budgetDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[120px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button v-for="(label, key) in budgetOptions" :key="key" @click="budgetFilter = key; budgetDropdownOpen = false" class="w-full flex items-center justify-between px-3 py-1.5 text-xs transition-colors" :class="budgetFilter === key ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span>{{ label }}</span><AppIcon v-if="budgetFilter === key" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
        </div>
      </div>

      <!-- Time filter -->
      <div class="relative flex-shrink-0" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) timeDropdownOpen = false }">
        <button @click="timeDropdownOpen = !timeDropdownOpen; closeAllDropdowns('time')" class="flex items-center gap-1 h-7 text-xs bg-white border border-slate-200 rounded-md px-2.5 outline-none transition-colors" :class="timeFilter !== 'all' ? 'border-blue-400 text-blue-600' : 'text-slate-600'">
          <span>{{ timeFilterLabels[timeFilter] }}</span>
          <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="timeDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="timeDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[130px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button v-for="(label, key) in timeFilterLabels" :key="key" @click="timeFilter = key; timeDropdownOpen = false" class="w-full flex items-center justify-between px-3 py-1.5 text-xs transition-colors" :class="timeFilter === key ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span>{{ label }}</span><AppIcon v-if="timeFilter === key" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
        </div>
      </div>

      <!-- Quarter filter -->
      <div class="relative flex-shrink-0" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) quarterDropdownOpen = false }">
        <button @click="quarterDropdownOpen = !quarterDropdownOpen; closeAllDropdowns('quarter')" class="flex items-center gap-1 h-7 text-xs bg-white border border-slate-200 rounded-md px-2.5 outline-none transition-colors" :class="selectedQuarter ? 'border-blue-400 text-blue-600' : 'text-slate-600'">
          <span>{{ selectedQuarter ? quarterLabel : '预计下单' }}</span>
          <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="quarterDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="quarterDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[130px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button @click="selectedQuarter = ''; quarterDropdownOpen = false" class="w-full flex items-center justify-between px-3 py-1.5 text-xs transition-colors" :class="!selectedQuarter ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span>全部季度</span><AppIcon v-if="!selectedQuarter" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
          <button v-for="q in quarterOptions" :key="q.key" @click="selectedQuarter = q.key; quarterDropdownOpen = false" class="w-full flex items-center justify-between px-3 py-1.5 text-xs transition-colors" :class="selectedQuarter === q.key ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span>{{ q.label }}</span><AppIcon v-if="selectedQuarter === q.key" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
        </div>
      </div>

      <div class="flex-1" />

      <!-- Import / Export buttons -->
      <div class="flex items-center gap-1.5">
        <button v-if="selectionMode" class="btn-secondary !h-7 text-xs" :disabled="exporting" @click="handleBatchExport">
          <AppIcon v-if="exporting" name="spinner" size="12" class="mr-1 animate-spin" />
          <AppIcon v-else name="download" size="12" stroke="2.5" class="mr-1" />{{ exporting ? '导出中...' : '导出所选' }}
        </button>
        <button v-if="selectionMode" class="btn-ghost !h-7 text-xs" @click="toggleSelectionMode">取消</button>
        <button v-if="!selectionMode" class="btn-secondary !h-7 text-xs" @click="toggleSelectionMode">
          <AppIcon name="download" size="12" stroke="2.5" class="mr-1" />导出
        </button>
        <button v-if="!selectionMode" class="btn-secondary !h-7 text-xs" :disabled="importing" @click="handleImportProject">
          <AppIcon v-if="importing" name="spinner" size="12" class="mr-1 animate-spin" />
          <AppIcon v-else name="upload" size="12" stroke="2.5" class="mr-1" />{{ importing ? '导入中...' : '导入' }}
        </button>
      </div>
    </div>

    <!-- ═══ Bottom Status Bar ═══ -->
    <div class="flex items-center justify-between text-xs text-surface-400 px-4 mb-3">
      <div class="flex items-center gap-2">
        <template v-if="selectionMode">
          <span class="text-blue-600 font-medium">已选 {{ selectedProjectIds.size }} 个项目</span>
        </template>
        <span>{{ filteredProjects.length }} 个项目</span>
        <span>总计 ¥{{ footerSummary.totalBudget }}万</span>
      </div>
    </div>

    <!-- ═══ Table Header ═══ -->
    <table class="w-full text-xs table-fixed min-w-[770px]">
      <thead>
        <tr class="border-b border-surface-100 bg-surface-50 text-left">
          <th v-if="selectionMode" class="px-3 py-2 w-10 align-middle text-center"></th>
          <th @click="toggleSort('name')" class="pl-5 pr-3 py-2 text-[11px] font-medium text-surface-400 uppercase tracking-wide cursor-pointer hover:bg-surface-100 select-none align-middle text-left w-[330px]">项目名称 <span class="ml-0.5 opacity-60">{{ sortIcon('name') }}</span></th>
          <th @click="toggleSort('status')" class="px-3 py-2 text-[11px] font-medium text-surface-400 uppercase tracking-wide cursor-pointer hover:bg-surface-100 select-none w-[120px] align-middle text-left">阶段 <span class="ml-0.5 opacity-60">{{ sortIcon('status') }}</span></th>
          <th @click="toggleSort('budget')" class="px-3 py-2 text-[11px] font-medium text-surface-400 uppercase tracking-wide cursor-pointer hover:bg-surface-100 select-none w-[110px] align-middle text-left">金额 <span class="ml-0.5 opacity-60">{{ sortIcon('budget') }}</span></th>
          <th @click="toggleSort('expected_order_date')" class="px-3 py-2 text-[11px] font-medium text-surface-400 uppercase tracking-wide cursor-pointer hover:bg-surface-100 select-none w-[120px] align-middle text-left">预计下单 <span class="ml-0.5 opacity-60">{{ sortIcon('expected_order_date') }}</span></th>
          <th @click="toggleSort('updatedAt')" class="px-3 py-2 text-[11px] font-medium text-surface-400 uppercase tracking-wide cursor-pointer hover:bg-surface-100 select-none w-[120px] align-middle text-left">更新 <span class="ml-0.5 opacity-60">{{ sortIcon('updatedAt') }}</span></th>
        </tr>
      </thead>
    </table>

    <!-- ═══ Table Body ═══ -->
    <div ref="tableScrollRef" class="flex-1 min-h-0 overflow-auto">
      <table class="w-full text-xs table-fixed min-w-[770px]">
        <tbody class="divide-y divide-surface-50">
          <tr v-for="project in filteredProjects" :key="project.id"
            @click="selectionMode ? toggleProjectSelection(project.id) : goToProject(project.id)"
            class="cursor-pointer transition-colors group hover:bg-surface-50"
            :class="selectionMode && isProjectSelected(project.id) ? 'bg-blue-50/50 ring-1 ring-inset ring-blue-300' : ''">
            <td v-if="selectionMode" class="px-3 py-2.5 w-10 align-middle text-center">
              <div class="w-4 h-4 rounded border flex items-center justify-center mx-auto" :class="isProjectSelected(project.id) ? 'bg-blue-600 border-blue-600' : 'border-slate-300 bg-white'">
                <AppIcon v-if="isProjectSelected(project.id)" name="check" class="w-2.5 h-2.5 text-white" />
              </div>
            </td>
            <td class="pl-5 pr-3 py-2.5 align-middle text-left w-[330px]">
              <template v-if="editingField?.projectId === project.id && editingField.field === 'name'">
                <input v-model="editValue" type="text" maxlength="30" @blur="saveEdit(project)" @keyup.enter="saveEdit(project)" @keyup.escape="cancelEdit"
                  class="w-full text-xs font-medium text-surface-800 bg-white border border-blue-400 rounded px-1.5 py-0.5 outline-none focus:ring-1 focus:ring-blue-200" autofocus />
              </template>
              <template v-else>
                <div class="flex items-center gap-1 group/name">
                  <span class="font-medium text-surface-800 truncate">{{ project.name }}</span>
                  <button @click.stop="startEditName(project)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/name:opacity-100 flex-shrink-0" title="编辑名称">
                    <AppIcon name="edit" class="w-2.5 h-2.5" />
                  </button>
                </div>
              </template>
            </td>
            <td class="px-3 py-2.5 w-[120px] align-middle whitespace-nowrap text-left">
              <div class="relative" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) statusMenuProjectId = null }">
                <button ref="statusBtnRefs" @click.stop="statusMenuProjectId = statusMenuProjectId === project.id ? null : project.id; statusMenuRect = $event.target.getBoundingClientRect()"
                  class="inline-flex items-center gap-1 cursor-pointer rounded px-1 py-0.5 hover:bg-slate-50 transition-colors">
                  <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[project.status] || 'bg-gray-400'" />
                  <span class="text-xs" :class="statusBadgeClass(project.status)">{{ statusLabels[project.status] || project.status }}</span>
                  <AppIcon name="chevron-down" class="w-2.5 h-2.5 text-slate-400 transition-transform" :class="statusMenuProjectId === project.id ? 'rotate-180' : ''" />
                </button>
              </div>
            </td>
            <td class="px-3 py-2.5 text-surface-600 w-[110px] align-middle text-left">
              <template v-if="editingField?.projectId === project.id && editingField.field === 'budget'">
                <input v-model="editValue" type="text" @blur="saveEdit(project)" @keyup.enter="saveEdit(project)" @keyup.escape="cancelEdit"
                  class="w-20 text-xs text-surface-600 bg-white border border-blue-400 rounded px-1.5 py-0.5 outline-none focus:ring-1 focus:ring-blue-200" autofocus />
              </template>
              <template v-else>
                <div class="flex items-center gap-1 group/budget">
                  <span>{{ formatBudget(project.budget) }}</span>
                  <button @click.stop="startEditBudget(project)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/budget:opacity-100 flex-shrink-0" title="编辑金额">
                    <AppIcon name="edit" class="w-2.5 h-2.5" />
                  </button>
                </div>
              </template>
            </td>
            <td class="px-3 py-2.5 w-[120px] align-middle whitespace-nowrap text-left" @click.stop>
              <span ref="expectedDateCalendarRef" class="relative inline-flex items-center gap-0.5 group/expected-inline">
                <span @click.stop="startEditExpectedDateCalendar(project, $event)" class="text-xs text-surface-600 rounded px-1 py-0.5 border border-transparent cursor-pointer hover:bg-slate-50 transition-colors">
                  {{ (project.expectedOrderDate || project.expected_order_date) || '-' }}
                </span>
                <button @click.stop="startEditExpectedDateCalendar(project, $event)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/expected-inline:opacity-100 flex items-center justify-center" title="编辑预计下单时间">
                  <AppIcon name="edit" class="w-2.5 h-2.5" />
                </button>
                <div v-if="showExpectedDateCalendar && expectedDateCalendarProjectId === project.id" @click.stop class="absolute top-full left-0 mt-1 z-[80] bg-white/85 backdrop-blur-xl rounded-xl shadow-2xl border border-white/30 p-4 w-[260px]">
                  <div class="flex items-center justify-between mb-3">
                    <button @click="prevExpectedDateMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                      <AppIcon name="chevron-left" class="w-4 h-4 text-slate-500" />
                    </button>
                    <span class="text-sm font-medium text-slate-700">{{ expectedDateCalendarYear }} 年 {{ expectedDateCalendarMonth + 1 }} 月</span>
                    <button @click="nextExpectedDateMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
                      <AppIcon name="chevron-right" class="w-4 h-4 text-slate-500" />
                    </button>
                  </div>
                  <div class="grid grid-cols-7 mb-1">
                    <span v-for="w in ['日','一','二','三','四','五','六']" :key="w" class="text-center text-[11px] text-slate-400 py-1">{{ w }}</span>
                  </div>
                  <div class="grid grid-cols-7 gap-0.5">
                    <div v-for="(day, idx) in expectedDateCalendarDays" :key="idx" class="aspect-square flex items-center justify-center">
                      <button
                        v-if="day !== null"
                        @click="selectExpectedDay(day)"
                        class="w-8 h-8 rounded-full text-xs transition-colors"
                        :class="[
                          isSelectedExpectedDay(day)
                            ? 'bg-blue-600 text-white font-medium'
                            : todayStr === (expectedDateCalendarYear + '-' + String(expectedDateCalendarMonth + 1).padStart(2, '0') + '-' + String(day).padStart(2, '0'))
                              ? 'text-blue-600 font-medium hover:bg-blue-50'
                              : 'text-slate-600 hover:bg-slate-100'
                        ]"
                      >{{ day }}</button>
                    </div>
                  </div>
                  <div class="mt-3 pt-3 border-t border-slate-100 flex justify-between">
                    <button @click="clearExpectedDate" class="text-[11px] text-slate-400 hover:text-red-300 transition-colors">清除</button>
                    <button @click="closeExpectedDateCalendar" class="text-[11px] text-slate-400 hover:text-slate-600 transition-colors">关闭</button>
                  </div>
                </div>
              </span>
            </td>
            <td class="px-3 py-2.5 text-surface-400 w-[120px] align-middle whitespace-nowrap text-left">{{ formatDateTime(project.updatedAt || project.createdAt) }}</td>
          </tr>
          <!-- Empty state -->
          <tr v-if="filteredProjects.length === 0">
            <td :colspan="selectionMode ? 8 : 7" class="text-center py-4 text-surface-400 text-xs min-w-full">
              无项目
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <Teleport to="body">
      <div v-if="statusMenuProjectId && statusMenuRect"
        :style="{ position: 'fixed', top: statusMenuRect.bottom + 4 + 'px', left: statusMenuRect.left + 'px', zIndex: 9999 }"
        class="min-w-[110px] bg-white rounded-lg shadow-lg border border-slate-100 py-1"
        @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) statusMenuProjectId = null }">
        <button v-for="s in allStatusOptions" :key="s" @click.stop="changeStatus(filteredProjects.find(p => p.id === statusMenuProjectId), s)"
          class="w-full flex items-center gap-1.5 px-2.5 py-1 text-[11px] transition-colors text-left"
          :class="filteredProjects.find(p => p.id === statusMenuProjectId)?.status === s ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
          <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[s]" />
          <span>{{ statusLabels[s] }}</span>
        </button>
      </div>
    </Teleport>

    </div> <!-- close inner div -->

    <!-- ═══ New / Edit Project Modal ═══ -->
    <Teleport to="body">
      <div v-if="showNewProjectModal" class="fixed inset-0 z-50 flex items-center justify-center">
        <div class="absolute inset-0 bg-black/40 backdrop-blur-sm" @click="closeModal" />
        <div class="relative bg-white rounded-2xl shadow-xl w-full max-w-lg mx-4 p-6">
          <h2 class="text-lg font-semibold text-surface-800 mb-4">新建项目</h2>
          <div class="space-y-3">
            <div>
              <label class="block text-xs font-medium text-surface-300 mb-1">项目名称 *</label>
              <div class="relative">
                <input v-model="projectForm.name" type="text" maxlength="30" class="input-field pr-10" placeholder="例如：华为 OceanStor — 某银行" />
                <span class="absolute right-2.5 top-1/2 -translate-y-1/2 text-[10px] flex-shrink-0" :class="projectForm.name.length >= 30 ? 'text-red-400' : 'text-surface-300'">{{ projectForm.name.length }}/30</span>
              </div>
            </div>
            <div>
              <label class="block text-xs font-medium text-surface-300 mb-1">项目金额（万）</label>
              <input v-model.number="projectForm.budget" type="number" class="input-field" placeholder="例如：300" />
            </div>
            <div>
              <label class="block text-xs font-medium text-surface-300 mb-1">客户</label>
              <SelectWithSearch v-model="projectForm.customerId" :options="customerSelectOptions" placeholder="选择客户（可选）" search-placeholder="搜索客户..." />
            </div>
            <div class="relative">
              <label class="block text-xs font-medium text-surface-300 mb-1">项目阶段 *</label>
              <button @click="newProjectStatusOpen = !newProjectStatusOpen" class="flex items-center gap-1 h-7 text-xs w-full bg-white border border-slate-200 rounded-md px-2.5 outline-none focus:ring-1 focus:ring-blue-300/20 focus:border-blue-400 text-slate-600 hover:border-slate-500 transition-colors">
                <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[projectForm.status] || 'bg-gray-400'" />
                <span class="truncate flex-1 text-left">{{ statusLabels[projectForm.status] || '选择阶段' }}</span>
                <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform" :class="newProjectStatusOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="newProjectStatusOpen" class="absolute top-full left-0 mt-1 z-50 w-full bg-white rounded-lg shadow-lg border border-slate-100 py-1">
                <button v-for="s in allStatusOptions" :key="s" @click="projectForm.status = s; newProjectStatusOpen = false" class="w-full flex items-center gap-2 px-3 py-1.5 text-sm text-left transition-colors" :class="projectForm.status === s ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
                  <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[s] || 'bg-gray-400'" />
                  <span>{{ statusLabels[s] }}</span>
                </button>
              </div>
            </div>
            <div v-if="projectForm.status === 'Bidding'">
              <label class="block text-xs font-medium text-surface-300 mb-1">投标时间 *</label>
              <input v-model="projectForm.bidTime" type="datetime-local" class="input-field" />
            </div>
            <div class="pt-2 border-t border-surface-100">
              <label class="block text-xs font-medium text-surface-300 mb-1">项目描述</label>
              <textarea v-model="projectForm.description" rows="3" class="input-field" placeholder="简要描述项目背景与需求" />
            </div>
          </div>
          <div class="flex justify-end gap-3 mt-5">
            <button class="btn-secondary" @click="closeModal">取消</button>
            <button class="btn-primary" @click="createProject">新建</button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
