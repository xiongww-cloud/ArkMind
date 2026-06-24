<script setup lang="ts">
import { ref, computed, onMounted, onActivated, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { useRouter, useRoute } from 'vue-router'
import AppIcon from '@/components/common/AppIcon.vue'
import { formatDateTime } from '@/utils/time'
import { toast } from '@/composables/useToast'


import { fetchProjects } from '@/api/db-client'

const projectStore = useProjectStore()
const authStore = useAuthStore()
const router = useRouter()
const route = useRoute()

const searchQuery = ref('')
const statusFilter = ref('all')
const timeFilter = ref('all') // all | today | week | month | quarter
const viewMode = ref('grid')
const showNewProjectModal = ref(false)
const statusDropdownOpen = ref(false)
const timeDropdownOpen = ref(false)

// ─── Multi-select batch export ───────────────────────────────────────────
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
  try {
    const ids = [...selectedProjectIds.value]
    console.log('[export] starting, ids:', ids.length)
    const { save: saveDialog } = await import('@tauri-apps/plugin-dialog')
    console.log('[export] dialog imported')
    const filePath = await saveDialog({
      defaultPath: `project_export_${ids.length}.ark`,
      filters: [{ name: 'ArkMind 项目包', extensions: ['ark'] }],
    })
    console.log('[export] save result:', filePath)
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
  }
}

// Read ?status= query param from Dashboard card clicks + sync backend data
// Extracted so BOTH onMounted and onActivated can call it
async function syncFromBackend() {
  try {
    const remote = await fetchProjects()
    if (!remote?.length) return
    const remoteArr = Array.isArray(remote) ? remote : []
    const store = projectStore.projects
    const remoteIds = new Set(remoteArr.map(r => r.id))
    const idMap = projectStore.loadBackendIdMap()

    // Step 1: Update local projects that match backend records
    for (const r of remote) {
      const match = store.find(p => p._backendId === r.id || p.name === r.name)
      if (match) {
        match._backendId = r.id
        match.name = r.name
        match.customer = r.customer || ''
        match.budget = r.budget || ''
        match.description = r.description || ''
        match.status = r.status
        match.owner = r.owner || ''
        match.userId = r.userId || ''
        match.displayId = r.displayId || ''
        match.createdAt = r.created_at || match.createdAt
        match.updatedAt = r.updated_at || null
        match.isArchived = r.isArchived || false
        match.customerContact = r.customer_contact || ''
        match.customerPhone = r.customer_phone || ''
        idMap[match.id] = r.id
      } else {
        // Step 2: Create local entry for unmatched backend projects
        store.push({
          id: r.id, _backendId: r.id,
          name: r.name, customer: r.customer || '',
          budget: r.budget || '', description: r.description || '',
          status: r.status, owner: r.owner || '',
          userId: r.userId || '',
          displayId: r.displayId || '',
          createdAt: r.created_at || new Date().toISOString().slice(0, 10),
          updatedAt: r.updated_at || null,
          isArchived: r.isArchived || false,
          customerContact: r.customer_contact || '',
          customerPhone: r.customer_phone || '',
          timeline: [], configurations: [], plans: [],
        })
      }
    }
    projectStore.saveBackendIdMap(idMap)

    // Step 3: Remove local projects whose backend record no longer exists
    for (let i = store.length - 1; i >= 0; i--) {
      const p = store[i]
      if (p._backendId && !remoteIds.has(p._backendId)) {
        store.splice(i, 1)
      }
    }
    // With UUID IDs (no more PRO-xxx), each project has a unique id — no dedup needed
    for (const r of remote) {
      const candidates = store.filter(p => p._backendId === r.id || p.name === r.name)
      if (candidates.length > 1) {
        // Keep the one with a PRO- prefix (hardcoded), or the first one; remove the rest
        const keep = candidates.find(p => p.id.startsWith('PRO-')) || candidates[0]
        for (const dup of candidates) {
          if (dup !== keep) {
            const idx = store.indexOf(dup)
            if (idx !== -1) store.splice(idx, 1)
          }
        }
      }
    }
  } catch (e) {
    console.warn('ProjectHub 后端同步失败，使用本地数据', e)
  }
}

onMounted(() => {
  const s = route.query.status
  if (s && typeof s === 'string' && statusOptions.includes(s)) {
    statusFilter.value = s
  }
  syncFromBackend()
})
// Event-driven refresh
// Keep-alive support: re-sync every time the component is activated
onActivated(() => {
  syncFromBackend()
})

// ── Datetime formatter: ISO → "YYYY-MM-DD HH:mm" ─────────────────────────
// ── Status color dots ────────────────────────────────────────────────────
const statusDotColors = {
  Lead: 'bg-blue-500',
  SolutionDesign: 'bg-cyan-500',
  PoC: 'bg-purple-500',
  Bidding: 'bg-orange-500',
  Won: 'bg-emerald-500',
  Ordered: 'bg-indigo-500',
  Lost: 'bg-red-500',
  OnHold: 'bg-gray-400'
}

const newProject = ref({
  name: '',
  customer: '',
  budget: 0,
  description: '',
  status: 'Lead',
  owner: authStore.currentUser.name,
  userId: authStore.currentUser.id,
  bidTime: '',
  customerContact: '',
  customerPhone: '',
})

const newProjectStatusOpen = ref(false)

const statusOptions = ['all', 'Lead', 'SolutionDesign', 'PoC', 'Bidding', 'Won', 'Ordered', 'Lost', 'OnHold']

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

const statusFilterLabels = {
  all: '全部阶段',
  Lead: '线索',
  SolutionDesign: '方案报价',
  PoC: '测试',
  Bidding: '投标',
  Won: '中标',
  Ordered: '下单',
  Lost: '丢单',
  OnHold: '搁置'
}

const timeFilterLabels = {
  all: '全部时间',
  today: '今天',
  week: '最近 7 天',
  month: '最近 30 天',
  quarter: '最近 90 天'
}

function selectStatus(status) {
  statusFilter.value = status
  statusDropdownOpen.value = false
}

function selectTime(t) {
  timeFilter.value = t
  timeDropdownOpen.value = false
}

function closeDropdowns(e) {
  // Only close if focus is leaving the container entirely (not moving between children)
  if (e.relatedTarget && e.currentTarget.contains(e.relatedTarget)) return
  setTimeout(() => {
    statusDropdownOpen.value = false
    timeDropdownOpen.value = false
  }, 150)
}

const myProjects = computed(() => {
  const all = Array.isArray(projectStore.projects) ? projectStore.projects : []
  const userId = authStore.currentUser.id
  const userName = authStore.currentUser.name
  // Filter: primary by userId, fallback by owner name, always include legacy projects without userId
  const owned = all.filter(p => {
    if (p.userId && p.userId === userId) return true
    if (p.owner === userName) return true
    if (!p.userId) return true // legacy project — keep visible
    return false
  })
  return owned
})

const filteredProjects = computed(() => {
  let result = myProjects.value
  // Status filter
  if (statusFilter.value !== 'all') {
    result = result.filter(p => p.status === statusFilter.value)
  }
  // Search filter
  if (searchQuery.value.trim()) {
    const q = searchQuery.value.toLowerCase()
    result = result.filter(p =>
      p.name.toLowerCase().includes(q) ||
      p.customer.toLowerCase().includes(q)
    )
  }
  // Time range filter (by updatedAt)
  if (timeFilter.value !== 'all') {
    const now = new Date()
    let cutoff = new Date()
    switch (timeFilter.value) {
      case 'today':
        cutoff = new Date(now.getFullYear(), now.getMonth(), now.getDate())
        break
      case 'week':
        cutoff.setDate(now.getDate() - 7)
        break
      case 'month':
        cutoff.setDate(now.getDate() - 30)
        break
      case 'quarter':
        cutoff.setDate(now.getDate() - 90)
        break
    }
    result = result.filter(p => {
      const t = p.updatedAt || p.createdAt
                if (!t) return false
          // SQLite datetime('now') stores UTC without timezone marker;
          // JS parses "YYYY-MM-DD HH:MM:SS" as local time, shifting by TZ offset.
          // Normalize: replace space with 'T' and append 'Z' for correct UTC parse.
          let ds = t
          if (!t.includes('T') && !t.includes('Z') && !t.includes('+')) {
            if (t.includes(' ')) ds = t.replace(' ', 'T') + 'Z'
            else if (/^\d{4}-\d{2}-\d{2}$/.test(t)) ds = t + 'T00:00:00Z'
          }
          return new Date(ds) >= cutoff
    })
  }
  return [...result].sort((a, b) => {
    // Archived projects sort last
    if (a.isArchived !== b.isArchived) return a.isArchived ? 1 : -1
    // Within each group, newest updated first
    const ta = a.updatedAt || a.createdAt || ''
    const tb = b.updatedAt || b.createdAt || ''
    return ta > tb ? -1 : ta < tb ? 1 : 0
  })
})

// Owner column is redundant when all visible projects belong to current user
const showOwnerColumn = computed(() => {
  const owners = new Set(filteredProjects.value.map(p => p.owner))
  return owners.size > 1 || (owners.size === 1 && !owners.has(authStore.currentUser.name) && !filteredProjects.value.some(p => p.userId && p.userId === authStore.currentUser.id))
})

const statusBadgeClass = (status) => {
  const s = (status || 'Lead').toLowerCase()
  return `badge badge-${s}`
}

function formatBudget(raw) {
  if (!raw) return '-'
  return '¥' + raw + '万'
}

function goToProject(id) {
  router.push(`/projects/${id}`)
}

function createProject() {
  if (!newProject.value.name) return
  if (!newProject.value.status) return
  if (newProject.value.status === 'Bidding' && !newProject.value.bidTime) {
    alert({ title: '提示', message: '投标中阶段必须选择投标时间' })
    return
  }
  const q = newProject.value.name.trim().toLowerCase()
  const duplicate = projectStore.projects.some((p: any) => p.name && p.name.toLowerCase() === q)
  if (duplicate) { alert({ title: '提示', message: '已存在同名项目，项目名称相同代表同一项目' }); return }
  const id = projectStore.addProject({ ...newProject.value })
  showNewProjectModal.value = false
  newProjectStatusOpen.value = false
  newProject.value = { name: '', budget: 0, description: '', status: 'Lead', owner: authStore.currentUser.name, userId: authStore.currentUser.id, bidTime: '' }
  router.push(`/projects/${id}`)
}

async function handleImportProject() {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const selected = await open({
      filters: [{ name: 'ArkMind 项目包', extensions: ['ark'] }],
      multiple: false,
    })
    if (!selected) return
    const filePath = Array.isArray(selected) ? selected[0] : selected
    if (!filePath) return
    const { importProject } = await import('@/api/db-client')
    const result = await importProject({ filePath })
    if (result.success) {
      toast.success('项目导入成功')
      await syncFromBackend()
      window.location.reload()
    } else {
      toast.error(String(result.error || '') || '导入失败')
    }
  } catch (e) {
    toast.error('导入失败: ' + String(e?.message || e || ''))
  }
}

</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-y-auto">
    <div class="p-4 space-y-3">
    <!-- Header -->
    <!-- Filters Bar -->
    <div class="flex flex-wrap items-center gap-3">
      <button class="btn-primary flex-shrink-0" @click="showNewProjectModal = true">
        <AppIcon name="plus-thin" class="w-3.5 h-3.5" />
        新建项目
      </button>
      <div class="relative flex-1 min-w-[200px] max-w-sm">
        <AppIcon name="search" class="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-surface-400" />
        <input v-model="searchQuery" type="text" placeholder="筛选项目..." class="input-field h-6 !py-0.5 pl-9" />
      </div>

      <!-- Status Dropdown (matching FileManager style) -->
      <div class="relative" @focusout="closeDropdowns">
        <button @click="statusDropdownOpen = !statusDropdownOpen" class="flex items-center gap-1 h-6 text-xs text-left bg-white border border-slate-200 rounded-md px-2.5 w-[108px] outline-none focus:ring-1 focus:ring-blue-500/20 focus:border-blue-400 text-slate-600 hover:border-slate-300 transition-colors">
          <span class="truncate flex-1 text-left">{{ statusFilterLabels[statusFilter] }}</span>
          <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform duration-150 flex-shrink-0" :class="statusDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="statusDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[140px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button v-for="status in statusOptions" :key="status" @click="selectStatus(status)" class="w-full flex items-center justify-between px-3 py-1.5 text-sm text-left transition-colors" :class="statusFilter === status ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span class="flex items-center gap-2">
              <span v-if="status !== 'all'" class="w-2 h-2 rounded-full" :class="statusDotColors[status] || 'bg-gray-400'" />
              <span>{{ statusFilterLabels[status] }}</span>
            </span>
            <AppIcon v-if="statusFilter === status" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
        </div>
      </div>

      <!-- Time Range Dropdown -->
      <div class="relative" @focusout="closeDropdowns">
        <button @click="timeDropdownOpen = !timeDropdownOpen" class="flex items-center gap-1 h-6 text-xs text-left bg-white border border-slate-200 rounded-md px-2.5 w-[124px] outline-none focus:ring-1 focus:ring-blue-500/20 focus:border-blue-400 text-slate-600 hover:border-slate-300 transition-colors">
          <AppIcon name="clock" class="w-3.5 h-3.5 text-slate-400 flex-shrink-0" />
          <span class="truncate flex-1 text-left">{{ timeFilterLabels[timeFilter] }}</span>
          <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform duration-150 flex-shrink-0" :class="timeDropdownOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="timeDropdownOpen" class="absolute top-full left-0 mt-1 z-50 min-w-[150px] bg-white rounded-lg shadow-lg border border-slate-100 py-1">
          <button v-for="(label, key) in timeFilterLabels" :key="key" @click="selectTime(key)" class="w-full flex items-center justify-between px-3 py-1.5 text-sm text-left transition-colors" :class="timeFilter === key ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
            <span>{{ label }}</span>
            <AppIcon v-if="timeFilter === key" name="check" class="w-3.5 h-3.5 text-blue-600" />
          </button>
        </div>
      </div>

      <div class="flex rounded-lg border border-surface-200 overflow-hidden h-6">
        <button @click="viewMode = 'grid'" class="flex items-center px-2.5 transition-colors"
          :class="viewMode === 'grid' ? 'bg-surface-100 text-surface-700' : 'text-surface-400 hover:text-surface-600'"
          title="网格视图">
          <AppIcon name="grid" class="h-4 w-4" />
        </button>
        <button @click="viewMode = 'table'" class="flex items-center px-2.5 transition-colors"
          :class="viewMode === 'table' ? 'bg-surface-100 text-surface-700' : 'text-surface-400 hover:text-surface-600'"
          title="列表视图">
          <AppIcon name="list" class="h-4 w-4" />
        </button>
      </div>

      <div class="flex-1" />

      <!-- Import / Export buttons (right side) -->
      <div class="flex items-center gap-2">
        <button v-if="selectionMode" class="btn-secondary !h-6 text-xs" @click="handleBatchExport">
          <AppIcon name="download" size="12" stroke="2.5" class="mr-1" />
          导出所选
        </button>
        <button v-if="selectionMode" class="btn-ghost !h-6 text-xs" @click="toggleSelectionMode">
          取消
        </button>
        <button v-if="!selectionMode" class="btn-secondary !h-6 text-xs" @click="toggleSelectionMode">
          <AppIcon name="download" size="12" stroke="2.5" class="mr-1" />
          导出
        </button>
        <button v-if="!selectionMode" class="btn-secondary !h-6 text-xs" @click="handleImportProject">
          <AppIcon name="upload" size="12" stroke="2.5" class="mr-1" />
          导入
        </button>
      </div>
    </div>

    <!-- Grid View -->
    <div v-if="viewMode === 'grid'" class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      <div v-for="project in filteredProjects" :key="project.id"
        @click="selectionMode ? toggleProjectSelection(project.id) : goToProject(project.id)"
        class="card flex flex-col transition-all relative"
        :class="[
          project.isArchived ? 'bg-slate-50/50 opacity-70 grayscale-[20%] cursor-pointer' : 'cursor-pointer group hover:-translate-y-0.5 hover:shadow-md',
          selectionMode && isProjectSelected(project.id) ? 'ring-2 ring-blue-500 bg-blue-50/30' : ''
        ]">
        <!-- Row 1: Status badge with colored dot + owner -->
        <div class="flex items-center justify-between mb-3">
          <span v-if="project.isArchived" class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-slate-100 text-slate-500">
            <AppIcon name="archive" class="mr-1 w-3 h-3" />
            已归档
          </span>
          <span v-else class="inline-flex items-center gap-1.5">
            <span class="w-2 h-2 rounded-full inline-block flex-shrink-0" :class="statusDotColors[project.status] || 'bg-gray-400'" />
            <span :class="statusBadgeClass(project.status)">{{ statusLabels[project.status] || project.status }}</span>
          </span>
        </div>

        <!-- Selection checkbox (visible only in selection mode) -->
        <div v-if="selectionMode" class="absolute top-3 right-3 z-10">
          <div class="w-5 h-5 rounded border-2 flex items-center justify-center transition-colors"
            :class="isProjectSelected(project.id) ? 'bg-blue-600 border-blue-600' : 'border-slate-300 bg-white'"
          >
            <AppIcon v-if="isProjectSelected(project.id)" name="check" class="w-3 h-3 text-white" />
          </div>
        </div>

        <!-- Row 2: Project name -->
        <h3 class="text-sm font-semibold text-surface-800 line-clamp-2 mb-1.5">{{ project.name }}</h3>

        <!-- Row 3: Description -->
        <p class="text-xs text-surface-400 truncate w-full mb-1.5">{{ project.description || '无项目描述' }}</p>

        <!-- Divider -->
        <div class="border-t border-surface-100 mt-auto pt-2.5" />

        <!-- Bottom rows: Customer+更新 / Budget+立项 -->
        <div class="space-y-0.5 mt-1.5">
          <div class="flex justify-between items-baseline">
            <p class="text-xs text-surface-500 truncate min-w-0">{{ project.customer || '未指定客户' }}</p>
            <p class="text-[11px] text-surface-400 shrink-0 text-left w-[134px] ml-[16px]">更新：{{ formatDateTime(project.updatedAt || project.createdAt) }}</p>
          </div>
          <div class="flex justify-between items-baseline">
            <p class="text-xs font-medium text-surface-600">{{ formatBudget(project.budget) }}</p>
            <p class="text-[11px] text-surface-400 shrink-0 text-left w-[134px] ml-[16px]">立项：{{ formatDateTime(project.createdAt) }}</p>
          </div>
        </div>
      </div>

      <div v-if="filteredProjects.length === 0" class="col-span-full card text-center py-12">
        <AppIcon name="folder-open" class="mx-auto h-12 w-12 text-surface-300" />
        <p class="mt-3 text-sm text-surface-400">没有匹配的项目</p>
      </div>
    </div>

    <!-- Table View -->
    <div v-else class="card overflow-hidden !p-0">
      <table class="w-full text-sm table-fixed">
        <thead>
          <tr class="border-b border-surface-100 bg-surface-50 text-left">
            <th v-if="selectionMode" class="px-4 py-3 w-10"></th>
            <th class="px-4 py-3 text-xs font-medium text-surface-400 uppercase w-1/3">项目名称</th>
            <th class="px-4 py-3 text-xs font-medium text-surface-400 uppercase">阶段</th>
            <th class="px-4 py-3 text-xs font-medium text-surface-400 uppercase">金额（万）</th>
            <th class="px-4 py-3 text-xs font-medium text-surface-400 uppercase">更新</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-surface-50">
          <tr v-for="project in filteredProjects" :key="project.id"
            @click="selectionMode ? toggleProjectSelection(project.id) : goToProject(project.id)"
            class="cursor-pointer transition-colors"
            :class="[
              project.isArchived ? 'bg-slate-50/50 opacity-70 grayscale-[20%] hover:bg-slate-50' : 'hover:bg-surface-50',
              selectionMode && isProjectSelected(project.id) ? 'bg-blue-50/50 ring-1 ring-inset ring-blue-300' : ''
            ]">
            <td v-if="selectionMode" class="px-4 py-3 w-10">
              <div class="w-4 h-4 rounded border-2 flex items-center justify-center"
                :class="isProjectSelected(project.id) ? 'bg-blue-600 border-blue-600' : 'border-slate-300 bg-white'"
              >
                <AppIcon v-if="isProjectSelected(project.id)" name="check" class="w-2.5 h-2.5 text-white" />
              </div>
            </td>
            <td class="px-4 py-3 font-medium text-surface-800">{{ project.name }}</td>
            <td class="px-4 py-3">
              <span v-if="project.isArchived" class="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-slate-100 text-slate-500">
                <AppIcon name="archive" class="mr-1 w-3 h-3" />
                已归档
              </span>
              <span v-else class="inline-flex items-center gap-1.5">
                <span class="w-2 h-2 rounded-full inline-block flex-shrink-0" :class="statusDotColors[project.status] || 'bg-gray-400'" />
                <span :class="statusBadgeClass(project.status)">{{ statusLabels[project.status] || project.status }}</span>
              </span>
            </td>
            <td class="px-4 py-3 text-surface-600">{{ formatBudget(project.budget) }}</td>
            <td class="px-4 py-3 text-surface-400">{{ formatDateTime(project.updatedAt || project.createdAt) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- New Project Modal -->
    <Teleport to="body">
      <div v-if="showNewProjectModal" class="fixed inset-0 z-50 flex items-center justify-center">
        <div class="absolute inset-0 bg-black/40 backdrop-blur-sm" @click="showNewProjectModal = false" />
        <div class="relative bg-white rounded-2xl shadow-xl w-full max-w-lg mx-4 p-6">
          <h2 class="text-lg font-semibold text-surface-800 mb-4">新建项目</h2>
          <div class="space-y-4">

            <!-- 基本信息 -->
            <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">基本信息</h3>
            <div>
              <label class="block text-xs font-medium text-surface-500 mb-1">项目名称 *</label>
              <input v-model="newProject.name" type="text" class="input-field" placeholder="例如：华为 OceanStor — 某银行" />
            </div>
            <div>
              <label class="block text-xs font-medium text-surface-500 mb-1">项目金额（万）</label>
              <input v-model.number="newProject.budget" type="number" class="input-field" placeholder="例如：500" />
            </div>
            <div class="relative">
              <label class="block text-xs font-medium text-surface-500 mb-1">项目阶段 *</label>
              <button @click="newProjectStatusOpen = !newProjectStatusOpen" class="flex items-center gap-1 h-6 text-xs w-full bg-white border border-slate-200 rounded-md px-2.5 outline-none focus:ring-1 focus:ring-blue-500/20 focus:border-blue-400 text-slate-600 hover:border-slate-300 transition-colors">
                <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[newProject.status] || 'bg-gray-400'" />
                <span class="truncate flex-1 text-left">{{ statusLabels[newProject.status] || '选择阶段' }}</span>
                <AppIcon name="chevron-down" class="w-3.5 h-3.5 text-slate-400 transition-transform duration-150 flex-shrink-0" :class="newProjectStatusOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="newProjectStatusOpen" class="absolute top-full left-0 mt-1 z-50 w-full bg-white rounded-lg shadow-lg border border-slate-100 py-1">
                <button v-for="s in statusOptions.filter(o => o !== 'all')" :key="s" @click="newProject.status = s; newProjectStatusOpen = false" class="w-full flex items-center gap-2 px-3 py-1.5 text-sm text-left transition-colors" :class="newProject.status === s ? 'text-blue-600 bg-blue-50/50' : 'text-slate-600 hover:bg-slate-50'">
                  <span class="w-2 h-2 rounded-full flex-shrink-0" :class="statusDotColors[s] || 'bg-gray-400'" />
                  <span>{{ statusLabels[s] }}</span>
                </button>
              </div>
            </div>

            <!-- 投标时间（仅投标中阶段显示，必填） -->
            <div v-if="newProject.status === 'Bidding'">
              <label class="block text-xs font-medium text-surface-500 mb-1">投标时间 *</label>
              <input v-model="newProject.bidTime" type="datetime-local" class="input-field" />
            </div>

            <!-- 项目描述 -->
            <div class="pt-2 border-t border-surface-100">
              <label class="block text-xs font-medium text-surface-500 mb-1">项目描述</label>
              <textarea v-model="newProject.description" rows="3" class="input-field" placeholder="简要描述项目背景与需求" />
            </div>
          </div>
          <div class="flex justify-end gap-3 mt-6">
            <button class="btn-secondary" @click="showNewProjectModal = false">取消</button>
            <button class="btn-primary" @click="createProject">新建</button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</div>
</template>
