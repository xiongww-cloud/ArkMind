import { ref, computed, onMounted, onActivated } from 'vue'
import { useRouter } from 'vue-router'
import { useProjectStore } from '@/stores/project'
import {
  fetchProjects, fetchTodos, fetchAllDynamics, fetchNotes,
  kbDashboardStats, kbProjectNoteIds,
  customerList, fetchChannels, fetchVendors
} from '@/api/db-client'

const MS_PER_DAY = 86400000
const PIPELINE_STAGES = [
  { key: 'Lead', label: '线索', color: '#94a3b8' },
  { key: 'SolutionDesign', label: '方案报价', color: '#22d3ee' },
  { key: 'PoC', label: '测试', color: '#6366f1' },
  { key: 'Bidding', label: '投标', color: '#f59e0b' },
  { key: 'Won', label: '中标', color: '#10b981' },
]

export function useDashboardData() {
  const router = useRouter()
  const projectStore = useProjectStore()

  const allProjects = ref<any[]>([])
  const allTodos = ref<any[]>([])
  const allDynamics = ref<any[]>([])
  const allNotes = ref<any[]>([])
  const projectNoteIds = ref<Set<string>>(new Set())
  const dashboardStats = ref<any>(null)
  const customers = ref<any[]>([])
  const channels = ref<any[]>([])
  const vendors = ref<any[]>([])

  const loading = ref(true)

  async function loadData() {
    loading.value = true
    try {
      const [p, t, d, n, pn, ks, c, ch, v] = await Promise.allSettled([
        fetchProjects(),
        fetchTodos(),
        fetchAllDynamics(),
        fetchNotes(),
        kbProjectNoteIds(),
        kbDashboardStats(),
        customerList(),
        fetchChannels(''),
        fetchVendors(''),
      ])
      if (p.status === 'fulfilled') allProjects.value = Array.isArray(p.value) ? p.value : projectStore.projects
      else allProjects.value = projectStore.projects
      if (t.status === 'fulfilled') allTodos.value = (t.value as any)?.todos || []
      if (d.status === 'fulfilled') allDynamics.value = (d.value as any) || []
      if (n.status === 'fulfilled') allNotes.value = (n.value as any)?.notes || []
      if (pn.status === 'fulfilled') projectNoteIds.value = new Set((pn.value as any)?.ids || [])
      if (ks.status === 'fulfilled') dashboardStats.value = ks.value
      if (c.status === 'fulfilled') customers.value = (c.value as any) || []
      if (ch.status === 'fulfilled') channels.value = (ch.value as any) || []
      if (v.status === 'fulfilled') vendors.value = (v.value as any) || []
    } finally {
      loading.value = false
    }
  }

  onMounted(loadData)
  onActivated(loadData)

  // ─── Helpers ────────────────────────────────────────────────────
  function formatBudget(v: number): string {
    if (v >= 10000) return `${(v / 10000).toFixed(1)}亿`
    if (v >= 1) return `${v.toLocaleString()}万`
    return v > 0 ? `${v}万` : ''
  }

  function formatBudgetCompact(v: number): string {
    if (v >= 10000) return `${(v / 10000).toFixed(1)}亿`
    if (v >= 1000) return `${(v / 1000).toFixed(1)}千万`
    if (v >= 1) return `${v.toLocaleString()}万`
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

  // ─── Time-of-day ────────────────────────────────────────────────
  const greeting = computed(() => {
    const h = new Date().getHours()
    if (h < 12) return '早上好'
    if (h < 18) return '下午好'
    return '晚上好'
  })

  const dateStr = computed(() => {
    const d = new Date()
    const week = ['日', '一', '二', '三', '四', '五', '六']
    return `${d.getFullYear()}年${d.getMonth() + 1}月${d.getDate()}日 周${week[d.getDay()]}`
  })

  // ─── Month helpers ──────────────────────────────────────────────
  function getMonthKey(dateStr: string): string {
    if (!dateStr) return ''
    return dateStr.slice(0, 7)
  }

  const now = new Date()
  const thisMonthKey = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`
  const lastMonthDate = new Date(now.getFullYear(), now.getMonth() - 1, 1)
  const lastMonthKey = `${lastMonthDate.getFullYear()}-${String(lastMonthDate.getMonth() + 1).padStart(2, '0')}`

  // ─── KPI: Active projects ──────────────────────────────────────
  const activeProjects = computed(() =>
    allProjects.value.filter((p: any) => !['Won', 'Ordered', 'Lost', 'OnHold', 'Archived'].includes(p.status))
  )
  const activeProjectCount = computed(() => activeProjects.value.length)

  // ─── KPI: Pipeline total budget ────────────────────────────────
  const pipelineTotalBudget = computed(() =>
    activeProjects.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
  )

  // ─── KPI: Monthly new projects (current month vs last month) ──
  const newThisMonth = computed(() =>
    allProjects.value.filter((p: any) => {
      const k = getMonthKey(p.created_at || '')
      return k === thisMonthKey
    })
  )
  const newLastMonth = computed(() =>
    allProjects.value.filter((p: any) => {
      const k = getMonthKey(p.created_at || '')
      return k === lastMonthKey
    })
  )
  const newThisMonthCount = computed(() => newThisMonth.value.length)
  const newThisMonthBudget = computed(() =>
    newThisMonth.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
  )
  const newBudgetDelta = computed(() => {
    const last = newLastMonth.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
    const curr = newThisMonthBudget.value
    if (last === 0) return curr > 0 ? 100 : 0
    return Math.round(((curr - last) / last) * 100)
  })

  // ─── KPI: Monthly won (current vs last) ────────────────────────
  const wonThisMonth = computed(() =>
    allProjects.value.filter((p: any) => p.status === 'Won' && getMonthKey(p.updated_at || '') === thisMonthKey)
  )
  const wonLastMonth = computed(() =>
    allProjects.value.filter((p: any) => p.status === 'Won' && getMonthKey(p.updated_at || '') === lastMonthKey)
  )
  const wonThisMonthCount = computed(() => wonThisMonth.value.length)
  const wonThisMonthBudget = computed(() =>
    wonThisMonth.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
  )
  const winRate = computed(() => {
    const totalWon = allProjects.value.filter((p: any) => p.status === 'Won').length
    const totalLost = allProjects.value.filter((p: any) => p.status === 'Lost').length
    const total = totalWon + totalLost
    return total > 0 ? Math.round((totalWon / total) * 100) : 0
  })
  const wonBudgetDelta = computed(() => {
    const last = wonLastMonth.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
    const curr = wonThisMonthBudget.value
    if (last === 0) return curr > 0 ? 100 : 0
    return Math.round(((curr - last) / last) * 100)
  })

  // ─── KPI: Expected orders (next 30 days) ───────────────────────
  const expectedOrders = computed(() => {
    const nowMs = Date.now()
    const limitMs = nowMs + MS_PER_DAY * 30
    return allProjects.value.filter((p: any) => {
      if (!p.expected_order_date || ['Won', 'Ordered', 'Lost', 'Archived'].includes(p.status)) return false
      const ms = new Date(p.expected_order_date).getTime()
      return ms >= nowMs && ms <= limitMs
    })
  })
  const expectedOrderBudget = computed(() =>
    expectedOrders.value.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
  )

  // ─── KPI: Urgent matters ───────────────────────────────────────
  const overdueTodos = computed(() => {
    const today = new Date().toISOString().slice(0, 10)
    return allTodos.value.filter((t: any) => !t.done && t.due_date && t.due_date.slice(0, 10) <= today)
  })
  const urgentBids = computed(() => {
    const nowMs = Date.now()
    return allProjects.value.filter((p: any) => {
      if (!p.bid_time || ['Won', 'Ordered', 'Lost', 'Archived'].includes(p.status)) return false
      const ms = new Date(p.bid_time).getTime() - nowMs
      return ms > 0 && ms < MS_PER_DAY * 3
    })
  })
  const urgentCount = computed(() => overdueTodos.value.length + urgentBids.value.length)

  // ─── Pipeline funnel ────────────────────────────────────────────
  const pipelineStats = computed(() => {
    const projects = allProjects.value
    return PIPELINE_STAGES.map(stage => {
      const items = projects.filter((p: any) => p.status === stage.key)
      return {
        ...stage,
        count: items.length,
        budget: items.reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0),
      }
    })
  })

  const maxPipelineCount = computed(() => Math.max(...pipelineStats.value.map(s => s.count), 1))

  // ─── Stage donut data ──────────────────────────────────────────
  const stageDonutData = computed(() =>
    pipelineStats.value.filter(s => s.budget > 0).map(s => ({
      label: s.label,
      value: s.budget,
      color: s.color,
    }))
  )

  // ─── 6-month trend ─────────────────────────────────────────────
  const trendMonths = computed(() => {
    const months: string[] = []
    for (let i = 5; i >= 0; i--) {
      const d = new Date(now.getFullYear(), now.getMonth() - i, 1)
      months.push(`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}`)
    }
    return months
  })

  const trendLabels = computed(() =>
    trendMonths.value.map(m => {
      const [, mm] = m.split('-')
      return `${parseInt(mm)}月`
    })
  )

  const trendNewBudget = computed(() =>
    trendMonths.value.map(month =>
      allProjects.value
        .filter((p: any) => getMonthKey(p.created_at || '') === month)
        .reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
    )
  )

  const trendWonBudget = computed(() =>
    trendMonths.value.map(month =>
      allProjects.value
        .filter((p: any) => p.status === 'Won' && getMonthKey(p.updated_at || '') === month)
        .reduce((s: number, p: any) => s + (Number(p.budget) || 0), 0)
    )
  )

  // ─── Top customers ──────────────────────────────────────────────
  const topCustomers = computed(() => {
    const map = new Map<string, number>()
    allProjects.value.forEach((p: any) => {
      const name = p.customer_name || p.customerName || p._customerName || p.customer || ''
      if (name && !['Won', 'Ordered', 'Lost', 'Archived'].includes(p.status)) {
        map.set(name, (map.get(name) || 0) + (Number(p.budget) || 0))
      }
    })
    return [...map.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([name, budget]) => ({ name, budget }))
  })

  const maxCustomerBudget = computed(() => Math.max(...topCustomers.value.map(c => c.budget), 1))

  // ─── Today todos ────────────────────────────────────────────────
  const todayTodos = computed(() => {
    return [...allTodos.value]
      .filter((t: any) => !t.done)
      .sort((a: any, b: any) => {
        const da = a.due_date || '9999'
        const db = b.due_date || '9999'
        if (da !== db) return da.localeCompare(db)
        const pa: Record<string, number> = { P1: 0, P2: 1, P3: 2 }
        return (pa[a.priority] ?? 9) - (pa[b.priority] ?? 9)
      })
      .slice(0, 5)
  })

  const priorityDot: Record<string, string> = {
    P1: 'bg-red-500', P2: 'bg-amber-400', P3: 'bg-surface-300'
  }
  const priorityStyle: Record<string, string> = {
    P1: 'bg-red-50 text-red-600', P2: 'bg-amber-50 text-amber-600', P3: 'bg-slate-100 text-slate-500'
  }

  // ─── Key deadlines (bid_time + expected_order_date merged) ─────
  const deadlines = computed(() => {
    const items: any[] = []
    allProjects.value.forEach((p: any) => {
      if (['Won', 'Ordered', 'Lost', 'Archived'].includes(p.status)) return
      if (p.bid_time) {
        const ms = new Date(p.bid_time).getTime() - Date.now()
        items.push({
          id: p.id, name: p.name, date: p.bid_time.slice(0, 10),
          type: '投标', urgent: ms > 0 && ms < MS_PER_DAY * 2,
          medium: ms > 0 && ms < MS_PER_DAY * 7, sortMs: new Date(p.bid_time).getTime(),
        })
      }
      if (p.expected_order_date) {
        const ms = new Date(p.expected_order_date).getTime() - Date.now()
        items.push({
          id: p.id, name: p.name, date: p.expected_order_date.slice(0, 10),
          type: '下单', urgent: ms > 0 && ms < MS_PER_DAY * 2,
          medium: ms > 0 && ms < MS_PER_DAY * 7, sortMs: new Date(p.expected_order_date).getTime(),
        })
      }
    })
    return items.sort((a, b) => a.sortMs - b.sortMs).slice(0, 5)
  })

  // ─── Recent notes ──────────────────────────────────────────────
  const recentNotes = computed(() => {
    return [...allNotes.value]
      .filter((n: any) => !n.is_daily && !projectNoteIds.value.has(n.note_id) && n.excerpt && n.excerpt.replace(/<[^>]*>/g, '').trim())
      .sort((a: any, b: any) => (b.updated_at || '').localeCompare(a.updated_at || ''))
      .slice(0, 5)
      .map((n: any) => ({
        id: n.note_id,
        title: n.title || '未命名',
        preview: n.excerpt.replace(/<[^>]*>/g, '').slice(0, 60),
        updated: n.updated_at ? n.updated_at.slice(0, 10) : '',
      }))
  })

  // ─── Knowledge health ──────────────────────────────────────────
  const kbHealth = computed(() => dashboardStats.value?.health || {})
  const kbByType = computed(() => {
    const raw = dashboardStats.value?.by_type || []
    return raw.map((item: any) => ({
      type: item.label || item.source_type || item.type || '',
      label: sourceTypeLabel(item.label || item.source_type || item.type || ''),
      count: item.cnt || item.count || 0,
    }))
  })
  const kbTotalChunks = computed(() => kbHealth.value.total || 0)
  const kbObjectTotal = computed(() => dashboardStats.value?.object_counts?.total || 0)
  const kbObjectByType = computed(() => {
    const raw = dashboardStats.value?.object_counts?.by_type || []
    return raw.map((item: any) => ({
      type: item.label || '',
      label: sourceTypeLabel(item.label || ''),
      count: item.cnt || 0,
    }))
  })
  const kbHotCount = computed(() => kbHealth.value.hot_count || 0)
  const kbColdCount = computed(() => kbHealth.value.cold_count || 0)
  const kbHotRatio = computed(() => kbHealth.value.hot_ratio || 0)
  const kbColdRatio = computed(() => kbHealth.value.cold_ratio || 0)
  const kbAvgWeight = computed(() => kbHealth.value.avg_weight || 0)
  const kbTagCount = computed(() => kbHealth.value.tag_count || 0)
  const kbWeightDist = computed(() => dashboardStats.value?.weight_dist || [])
  const kbDailyGrowth = computed(() => {
    const raw: any[] = dashboardStats.value?.daily_growth || []
    let cumulative = 0
    return raw.map((item: any) => {
      cumulative += item.cnt || 0
      return { label: item.label || '', cnt: cumulative }
    })
  })

  function sourceTypeLabel(type: string): string {
    const map: Record<string, string> = {
      PROJECT: '项目', NOTE: '笔记', TODO: '待办', FILE: '文件',
      CUSTOMER: '客户', CHANNEL: '渠道', VENDOR: '供应商', PRODUCT: '产品',
    }
    return map[type] || type
  }

  // ─── Business overview bar ─────────────────────────────────────
  const bizOverview = computed(() => [
    { label: '客户', count: customers.value.length, route: '/contacts' },
    { label: '渠道', count: channels.value.length, route: '/contacts' },
    { label: '供应商', count: vendors.value.length, route: '/contacts' },
    { label: '笔记', count: allNotes.value.length, route: '/notes' },
    { label: '知识碎片', count: kbTotalChunks.value, route: '/knowledge' },
  ])

  return {
    loading,
    greeting, dateStr,
    activeProjectCount, pipelineTotalBudget,
    newThisMonthCount, newThisMonthBudget, newBudgetDelta,
    wonThisMonthCount, wonThisMonthBudget, wonBudgetDelta, winRate,
    expectedOrders, expectedOrderBudget,
    overdueTodos, urgentBids, urgentCount,
    pipelineStats, maxPipelineCount,
    stageDonutData,
    trendMonths, trendLabels, trendNewBudget, trendWonBudget,
    topCustomers, maxCustomerBudget,
    todayTodos, priorityDot, priorityStyle,
    deadlines,
    recentNotes,
    kbHealth, kbByType, kbTotalChunks, kbObjectTotal, kbObjectByType,
    kbHotCount, kbColdCount, kbHotRatio, kbColdRatio, kbAvgWeight, kbTagCount, kbWeightDist,
    kbDailyGrowth,
    bizOverview,
    allProjects, allTodos, allDynamics, allNotes,
    formatBudget, formatBudgetCompact, timeAgo, goToProject, sourceTypeLabel,
    PIPELINE_STAGES,
  }
}
