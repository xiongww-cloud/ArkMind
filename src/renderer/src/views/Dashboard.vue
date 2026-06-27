<script setup lang="ts">
import { ref, computed, onMounted, onActivated } from 'vue'
import { useRouter } from 'vue-router'
import { useProjectStore } from '@/stores/project'
import { useNavigation } from '@/composables/useNavigation'
import AppIcon from '@/components/common/AppIcon.vue'
import { fetchProjects, fetchTodos, fetchAllDynamics, fetchNotes, kbDashboardStats, kbProjectNoteIds } from '@/api/db-client'
import VueApexCharts from 'vue3-apexcharts'

const router = useRouter()
const projectStore = useProjectStore()
const { navigateTo } = useNavigation()

const MS_PER_DAY = 86400000

// ─── Time-of-day greeting ───────────────────────────────────────────
const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 12) return '早上好'
  if (h < 18) return '下午好'
  return '晚上好'
})

const dateStr = computed(() => {
  const d = new Date()
  const week = ['日','一','二','三','四','五','六']
  return `${d.getFullYear()}年${d.getMonth()+1}月${d.getDate()}日 周${week[d.getDay()]}`
})

// ─── Data sources ───────────────────────────────────────────────────
const allProjects = ref<any[]>([])
const allTodos = ref<any[]>([])
const allDynamics = ref<any[]>([])
const allNotes = ref<any[]>([])
const projectNoteIds = ref<Set<string>>(new Set())
const dashboardStats = ref<any>(null)

async function loadData() {
  try {
    const r = await fetchProjects()
    allProjects.value = Array.isArray(r) ? r : projectStore.projects
  } catch { allProjects.value = projectStore.projects }
  try { const r: any = await fetchTodos(); allTodos.value = r?.todos || [] } catch (e) { console.error('[Dashboard] fetchTodos failed:', e) }
  try { const r: any = await fetchAllDynamics(); allDynamics.value = r || [] } catch (e) { console.error('[Dashboard] fetchAllDynamics failed:', e) }
  try { const r: any = await fetchNotes(); allNotes.value = r?.notes || [] } catch (e) { console.error('[Dashboard] fetchNotes failed:', e) }
  try { const r = await kbProjectNoteIds(); projectNoteIds.value = new Set(r?.ids || []) } catch (e) { console.error('[Dashboard] kbProjectNoteIds failed:', e) }
  try { dashboardStats.value = await kbDashboardStats() } catch (e) { console.error('[Dashboard] kbDashboardStats failed:', e) }
}

onMounted(loadData)
onActivated(loadData)

// ─── Status labels & colors ─────────────────────────────────────────
const statusLabels: Record<string,string> = {
  Lead: '线索', SolutionDesign: '方案报价', PoC: '测试',
  Bidding: '投标', Won: '中标', Ordered: '下单', Lost: '丢单', OnHold: '搁置'
}

const statusColor: Record<string,string> = {
  Lead: 'bg-slate-100 text-slate-600', SolutionDesign: 'bg-cyan-50 text-cyan-600',
  PoC: 'bg-blue-50 text-blue-600', Quotation: 'bg-indigo-50 text-indigo-600',
  Bidding: 'bg-amber-50 text-amber-700', Won: 'bg-emerald-50 text-emerald-600',
  Ordered: 'bg-violet-50 text-violet-600', Lost: 'bg-red-50 text-red-500', OnHold: 'bg-gray-100 text-gray-500'
}

// ─── Header stats ───────────────────────────────────────────────────
const activeProjectCount = computed(() =>
  allProjects.value.filter((p: any) => !['Won','Ordered','Lost','OnHold','Archived'].includes(p.status)).length
)
const totalAmount = computed(() =>
  allProjects.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
)
const wonThisMonth = computed(() => {
  const now = new Date()
  const y = now.getFullYear()
  const m = String(now.getMonth() + 1).padStart(2, '0')
  const prefix = `${y}-${m}`
  return allProjects.value.filter((p: any) => 
    p.status === 'Won' && (p.updated_at || p.updatedAt || '').startsWith(prefix)
  ).length
})
const todayTodoCount = computed(() => {
  const today = new Date().toISOString().slice(0, 10)
  return allTodos.value.filter((t: any) => !t.done && t.due_date && t.due_date.slice(0,10) <= today).length
})
const urgentDeadlineCount = computed(() => {
  const now = Date.now()
  return allProjects.value.filter((p: any) => {
    if (!p.bid_time || ['Won','Ordered','Lost'].includes(p.status)) return false
    const ms = new Date(p.bid_time).getTime() - now
    return ms > 0 && ms < MS_PER_DAY * 3
  }).length
})

// ─── Pipeline stages ────────────────────────────────────────────────
const pipelineStages = [
  { key: 'Lead', label: '线索', color: 'bg-slate-400' },
  { key: 'SolutionDesign', label: '方案报价', color: 'bg-cyan-500' },
  { key: 'PoC', label: '测试', color: 'bg-blue-600' },
  { key: 'Bidding', label: '投标', color: 'bg-indigo-400' },
  { key: 'Won', label: '中标', color: 'bg-emerald-500' },
]

const pipelineStats = computed(() => {
  const projects = allProjects.value
  const totalBudget = projects.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
  return pipelineStages.map(stage => ({
    ...stage,
    count: projects.filter((p: any) => p.status === stage.key).length,
    budget: projects.filter((p: any) => p.status === stage.key).reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0),
    totalBudget,
  }))
})

// ─── Today todos ────────────────────────────────────────────────────
const todayTodos = computed(() => {
  return [...allTodos.value]
    .filter((t: any) => !t.done)
    .sort((a: any, b: any) => {
      const da = a.due_date || '9999'; const db = b.due_date || '9999'
      if (da !== db) return da.localeCompare(db)
      const pa: Record<string,number> = {P1:0,P2:1,P3:2}; return (pa[a.priority]??9) - (pa[b.priority]??9)
    })
    .slice(0, 6)
})

const priorityDot: Record<string,string> = {
  P1: 'bg-red-500', P2: 'bg-amber-400', P3: 'bg-surface-300'
}

const priorityStyle: Record<string,string> = {
  P1: 'bg-red-50 text-red-600', P2: 'bg-amber-50 text-amber-600', P3: 'bg-slate-100 text-slate-500'
}

// ─── Deadlines ──────────────────────────────────────────────────────
const deadlines = computed(() => {
  return [...allProjects.value]
    .filter((p: any) => p.bid_time && !['Won','Ordered','Lost','Archived'].includes(p.status))
    .sort((a: any, b: any) => (a.bid_time || '').localeCompare(b.bid_time || ''))
    .slice(0, 5)
    .map((p: any) => {
      const bt = p.bid_time || ''
      const ms = bt.includes('T') ? new Date(bt).getTime() - Date.now() : Infinity
      return {
        id: p.id,
        name: p.name, deadline: bt.slice(0, 10),
        urgent: ms > 0 && ms < MS_PER_DAY * 2,
        medium: ms > 0 && ms < MS_PER_DAY * 7,
        stage: statusLabels[p.status] || p.status,
      }
    })
})

// ─── Recent notes ───────────────────────────────────────────────────
const recentNotes = computed(() => {
  return [...allNotes.value]
    .filter((n: any) => !n.is_daily && !projectNoteIds.value.has(n.note_id) && n.content && n.content.replace(/<[^>]*>/g, '').trim())
    .sort((a: any, b: any) => (b.updated_at || '').localeCompare(a.updated_at || ''))
    .slice(0, 6)
    .map((n: any) => ({
      id: n.note_id,
      title: n.title || '未命名',
      preview: n.content.replace(/<[^>]*>/g, '').slice(0, 80),
      updated: n.updated_at ? n.updated_at.slice(0, 10) : '',
    }))
})

// ─── Knowledge growth chart ─────────────────────────────────────────
const growthChartOptions = computed(() => {
  const data = dashboardStats.value?.daily_growth || []
  return {
    chart: { type: 'area' as const, height: 180, toolbar: { show: false }, sparkline: { enabled: false }, animations: { enabled: false } },
    stroke: { curve: 'smooth' as const, width: 2 },
    fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.2, opacityTo: 0, stops: [0, 100] } },
    colors: ['#2563eb'],
    dataLabels: { enabled: false },
    grid: { borderColor: '#f1f5f9', strokeDashArray: 4 },
    xaxis: { categories: data.map((d: any) => d.label || ''), labels: { style: { fontSize: '10px', colors: '#94a3b8' }, rotate: -45 }, tickAmount: 8 },
    yaxis: { labels: { style: { fontSize: '10px', colors: '#94a3b8' } } },
    tooltip: { enabled: true, theme: 'light' },
    series: [{ name: '知识碎片', data: data.map((d: any) => d.cnt || 0) }],
  }
})

// ─── Active projects with latest dynamic ────────────────────────────
const activeProjects = computed(() => {
  const active = [...allProjects.value]
    .filter((p: any) => !['Won','Ordered','Lost','OnHold','Archived'].includes(p.status))
    .sort((a: any, b: any) => (b.updated_at || b.updatedAt || '').localeCompare(a.updated_at || a.updatedAt || ''))
    .slice(0, 10)

  return active.map((p: any) => {
    const dynamic = allDynamics.value.find((d: any) => d.project_id === p.id)
    return {
      id: p.id,
      name: p.name || '未命名',
      status: p.status,
      stage: statusLabels[p.status] || p.status,
      budget: formatBudget(Number(p.budget) || 0),
      customer: p.customer || '',
      owner: p.owner || '',
      bidTime: p.bid_time ? p.bid_time.slice(0, 10) : '',
      latestDynamic: dynamic ? { content: dynamic.content || '', time: timeAgo(dynamic.created_at || '') } : null,
    }
  })
})

// ─── Global activity log ────────────────────────────────────────────
// ─── Helpers ────────────────────────────────────────────────────────
function formatBudget(v: number): string {
  if (v >= 10000) return `${(v / 10000).toFixed(0)} 亿`
  if (v >= 1) return `${v.toLocaleString()} 万`
  return ''
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

function goToProject(projectId: string) {
  const match = projectStore.projects.find((p: any) => p._backendId === projectId)
  router.push(`/projects/${match?.id || projectId}`)
}
</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-y-auto">
    <div class="p-6 space-y-5 max-w-[1200px] mx-auto">

    <!-- ── Header ── -->
    <header>
      <h1 class="text-2xl font-bold tracking-tight text-surface-900">{{ greeting }} ☀️</h1>
      <p class="text-surface-400 text-[13px] mt-1">{{ dateStr }}</p>
      <div class="flex items-center gap-4 mt-3">
        <button @click="router.push('/projects')" class="flex items-center gap-1.5 text-[12px] text-surface-500 hover:text-brand-600 transition-colors">
          <span class="text-brand-600 font-bold text-[14px]">{{ activeProjectCount }}</span><span>个项目推进中</span>
        </button>
        <span class="w-px h-3 bg-surface-200"></span>
        <button @click="router.push('/projects')" class="flex items-center gap-1.5 text-[12px] text-surface-500 hover:text-brand-600 transition-colors">
          <span class="text-emerald-600 font-bold text-[14px]">¥{{ formatBudget(totalAmount) }}</span><span>总金额</span>
        </button>
        <span class="w-px h-3 bg-surface-200"></span>
        <button @click="router.push('/todos')" class="flex items-center gap-1.5 text-[12px] text-surface-500 hover:text-brand-600 transition-colors">
          <span class="text-amber-600 font-bold text-[14px]">{{ todayTodoCount }}</span><span>项待办到期</span>
        </button>
        <span class="w-px h-3 bg-surface-200"></span>
        <button @click="router.push('/projects')" class="flex items-center gap-1.5 text-[12px] text-surface-500 hover:text-brand-600 transition-colors">
          <span class="text-violet-600 font-bold text-[14px]">{{ wonThisMonth }}</span><span>个本月中标</span>
        </button>
      </div>
    </header>

    <!-- ── Pipeline ── -->
    <div class="bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
      <h2 class="text-[13px] font-semibold text-surface-700 mb-4 flex items-center gap-1.5">
        <AppIcon name="bar-chart" size="15" class="text-surface-400" />
        Pipeline 概览
      </h2>
      <div class="flex items-start gap-3">
        <div v-for="s in pipelineStats" :key="s.key"
          class="flex-1 rounded-xl p-4 bg-white border border-surface-200 shadow-sm hover:shadow-md hover:-translate-y-0.5 transition-all duration-200 cursor-pointer"
          @click="router.push('/projects')">
          <div class="flex items-center gap-2 mb-2">
            <div class="w-2 h-2 rounded-full" :class="s.color"></div>
            <span class="text-[11px] text-surface-500">{{ s.label }}</span>
            <span class="text-[10px] text-surface-300 ml-auto">{{ s.count }}</span>
          </div>
          <div class="text-[18px] font-bold text-surface-900" v-if="s.budget > 0">¥{{ formatBudget(s.budget) }}</div>
          <div class="text-[18px] font-bold text-surface-300" v-else>¥0</div>
        </div>
      </div>
    </div>

    <!-- ── Active Projects + Deadlines ── -->
    <div class="flex flex-col lg:flex-row gap-4 items-start">
      <div class="flex-1 lg:flex-[7] bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
            <AppIcon name="folder-open" size="15" class="text-surface-400" />
            活跃项目
          </h2>
          <button @click="router.push('/projects')" class="text-[11px] text-brand-600 hover:underline">查看全部</button>
        </div>
        <div v-if="activeProjects.length" class="space-y-1.5 pr-1" style="height:210px;overflow-y:auto">
          <div v-for="p in activeProjects" :key="p.id"
            class="flex items-center gap-3 px-3 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
            @click="goToProject(p.id)">
            <span class="text-[12px] font-medium text-surface-800 truncate flex-1">{{ p.name }}</span>
            <span class="text-[10px] px-1.5 py-0.5 rounded flex-shrink-0" :class="statusColor[p.status] || 'bg-surface-100 text-surface-500'">{{ p.stage }}</span>
            <span v-if="p.budget" class="text-[11px] text-surface-400 flex-shrink-0">¥{{ p.budget }}</span>
          </div>
        </div>
        <p v-else class="text-[12px] text-surface-300 py-8 text-center">暂无活跃项目</p>
      </div>
      <div class="flex-1 lg:flex-[5] bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
            <AppIcon name="clock" size="15" class="text-surface-400" />
            关键截止
          </h2>
        </div>
        <div v-if="deadlines.length" class="space-y-2 pr-1" style="height:210px;overflow-y:auto">
          <div v-for="d in deadlines" :key="d.name"
            class="flex items-center gap-2 px-2.5 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
            @click="goToProject(d.id)">
            <div class="w-1.5 h-1.5 rounded-full flex-shrink-0" :class="d.urgent ? 'bg-red-500 animate-pulse' : d.medium ? 'bg-amber-400' : 'bg-surface-300'"></div>
            <span class="flex-1 text-[12px] text-surface-700 truncate">{{ d.name }}</span>
            <span class="text-[11px] font-semibold flex-shrink-0" :class="d.urgent ? 'text-red-600' : d.medium ? 'text-amber-600' : 'text-surface-500'">{{ d.deadline }}</span>
          </div>
        </div>
        <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无临近截止</p>
      </div>
    </div>

    <!-- ── Todos + Recent Notes ── -->
    <div class="grid grid-cols-1 lg:grid-cols-12 gap-4">
      <div class="lg:col-span-6 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
            <AppIcon name="list" size="15" class="text-surface-400" />
            待办事项
          </h2>
          <button @click="router.push('/todos')" class="text-[11px] text-brand-600 hover:underline">查看全部</button>
        </div>
        <div v-if="todayTodos.length" class="space-y-1.5">
          <div v-for="t in todayTodos" :key="t.id"
            class="flex items-center gap-3 px-3 py-2.5 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
            @click="router.push('/todos')">
            <div class="w-2 h-2 rounded-full flex-shrink-0" :class="priorityDot[t.priority] || 'bg-surface-300'"></div>
            <span class="flex-1 text-[12px] text-surface-800 truncate">{{ t.title || '未命名' }}</span>
            <span v-if="t.priority" class="text-[10px] font-medium px-1.5 py-0.5 rounded" :class="priorityStyle[t.priority] || 'bg-surface-100 text-surface-500'">{{ t.priority }}</span>
            <span v-if="t.due_date" class="text-[10px] text-surface-400 flex-shrink-0">{{ t.due_date.slice(5, 10) }}</span>
          </div>
        </div>
        <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无待办</p>
      </div>
      <div class="lg:col-span-6 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
            <AppIcon name="file-doc" size="15" class="text-surface-400" />
            最近笔记
          </h2>
          <button @click="router.push('/notes')" class="text-[11px] text-brand-600 hover:underline">查看全部</button>
        </div>
        <div v-if="recentNotes.length" class="space-y-1.5">
          <div v-for="n in recentNotes" :key="n.id"
            class="flex items-center gap-3 px-3 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
            @click="navigateTo('note', n.id)">
            <span class="flex-1 text-[12px] text-surface-800 truncate">{{ n.title }}</span>
            <span class="text-[10px] text-surface-400 flex-shrink-0">{{ n.updated }}</span>
          </div>
        </div>
        <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无笔记</p>
      </div>
    </div>

    <!-- ── Knowledge Growth ── -->
    <div v-if="dashboardStats?.daily_growth?.length && growthChartOptions.series?.[0]?.data?.length" class="bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
      <h2 class="text-[13px] font-semibold text-surface-700 mb-4 flex items-center gap-1.5">
        <AppIcon name="sparkles" size="15" class="text-surface-400" />
        知识增长趋势
      </h2>
      <VueApexCharts :options="growthChartOptions" :series="growthChartOptions.series" type="area" height="180" />
    </div>

    </div>
  </div>
</template>
