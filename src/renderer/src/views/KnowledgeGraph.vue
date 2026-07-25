<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue'
import VueApexCharts from 'vue3-apexcharts'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import {
  kbDashboardStats,
  pipelineClearKnowledge, pipelineClearAll,
  pipelineRevectorize,
  cleanupRetryFailed, cleanupOrphanData,
  kbPipelineDebug,
  pipelineDebugStatus, knowledgePipelineStatus,
} from '@/api/db-client'

const stats = ref<any>(null)
const pipelineData = ref<any>(null)
const pipelineRunning = ref(false)
const growthDays = ref(30)
const chartVersion = ref(0)
async function loadStats() { try { const r = await kbDashboardStats(); if (r?.success) { stats.value = r; chartVersion.value++ } } catch (e) { console.error('[KG] loadStats failed:', e) } }
async function loadPipeline() {
  try {
    const [debug, status] = await Promise.all([pipelineDebugStatus(), knowledgePipelineStatus()])
    pipelineData.value = debug
    pipelineRunning.value = status?.running || false
  } catch (e) { console.error('[KG] loadPipeline failed:', e) }
}
function setGrowthDays(d: number) { growthDays.value = d }

const growthChart = computed(() => {
  if (!stats.value?.daily_growth?.length) return null
  const d = stats.value.daily_growth
  const cutoff = new Date(); cutoff.setDate(cutoff.getDate() - growthDays.value)
  const cutoffStr = cutoff.toISOString().slice(0, 10)
  const filtered = d.filter((x: any) => (x.label || '') >= cutoffStr)
  const data = filtered.length > 0 ? filtered : d.slice(-growthDays.value)
  return { series: [{ name: '碎片数', data: data.map((x: any) => x.cnt) }], categories: data.map((x: any) => (x.label || '').slice(5)) }
})
const byTypeChart = computed(() => {
  if (!stats.value?.by_type?.length) return null
  const d = stats.value.by_type
  const colors = d.map((x: any) => typeColor(x.label))
  return { series: d.map((x: any) => x.cnt), labels: d.map((x: any) => typeLabel(x.label)), colors }
})
const weightDistChart = computed(() => {
  if (!stats.value?.weight_dist?.length) return null
  const d = stats.value.weight_dist
  return { series: [{ name: '碎片数', data: d.map((x: any, i: number) => ({ x: i * 5, y: x.cnt || 0 })) }], categories: d.map((x: any) => x.label) }
})
const chartColors = ['#60a5fa','#fbbf24','#a78bfa','#34d399','#94a3b8','#f59e0b','#ec4899','#14b8a6']

async function handleClearAll() { if (!(await dangerConfirm({ title: '重置全部数据', message: '确定重置全部数据？将重新从业务表抓取数据并重新物化知识。', confirmText: '重置' }))) return; try { await pipelineClearAll(); toast({ message: '已重置全部数据，正在重新物化打标', type: 'success' }); await loadStats(); await loadPipeline() } catch (e) { console.error('[KG] clearAll failed:', e); toast({ message: '重置失败', type: 'error' }) } }
async function handleClearKnowledge() { if (!(await dangerConfirm({ title: '重置知识管线', message: '确定重置知识碎片和向量缓存？将重新从源数据物化。', confirmText: '重置' }))) return; try { await pipelineClearKnowledge(); toast({ message: '已重置知识管线，正在重新物化', type: 'success' }); await loadStats(); await loadPipeline() } catch (e) { console.error('[KG] clearKnowledge failed:', e); toast({ message: '重置失败', type: 'error' }) } }
async function handleOrphanCleanup() { try { await cleanupOrphanData(); toast({ message: '孤儿数据清理已完成', type: 'success' }); await loadStats() } catch (e) { console.error('[KG] orphanCleanup failed:', e); toast({ message: '清理失败', type: 'error' }) } }

async function handleRevectorize() {
  if (!(await dangerConfirm({ title: '重新向量化', message: '将清除所有现有向量数据并重新向量化。这可能需要较长时间，确定继续？', confirmText: '重新向量化' }))) return
  try {
    const result = await pipelineRevectorize()
    toast({ message: result.message || '已启动重新向量化', type: 'success' })
    await loadStats()
    await loadPipeline()
  } catch (e) { console.error('[KG] revectorize failed:', e); toast({ message: '重新向量化失败', type: 'error' }) }
}

const debugData = ref<any>(null)
async function loadDebug() { try { debugData.value = await kbPipelineDebug() } catch (e) { console.error('[KG] loadDebug failed:', e) } }
function typeLabel(st: string): string { const m: Record<string,string> = { FILE:'文件',NOTE:'笔记',PRODUCT:'产品',PROJECT:'项目',TODO:'待办',MEMORY:'记忆',POLICY:'策略',CHANNEL:'渠道',VENDOR:'供应商',CONTACT:'联系人',CUSTOMER:'客户' }; return m[st] || st || '通用' }
function typeColor(st: string): string { const m: Record<string,string> = { FILE:'#60a5fa',NOTE:'#fbbf24',PRODUCT:'#a78bfa',PROJECT:'#34d399',TODO:'#94a3b8',MEMORY:'#f59e0b',POLICY:'#ec4899',CHANNEL:'#06b6d4',VENDOR:'#8b5cf6',CONTACT:'#14b8a6',CUSTOMER:'#f43f5e' }; return m[st] || '#94a3b8' }
let refreshTimer: ReturnType<typeof setInterval> | null = null
function saveScroll() {
  const el = document.querySelector('#kg-root .overflow-y-auto')
  return el ? el.scrollTop : 0
}
function restoreScroll(pos: number) {
  const el = document.querySelector('#kg-root .overflow-y-auto')
  if (el) el.scrollTop = pos
}
onMounted(() => {
  loadStats()
  loadPipeline()
  refreshTimer = setInterval(() => {
    const scrollPos = saveScroll()
    Promise.all([loadStats(), loadPipeline()]).then(() => nextTick(() => restoreScroll(scrollPos)))
  }, 5000)
})
onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})
</script>
<template>
  <div id="kg-root" class="h-full flex flex-col bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden">
    <div class="flex-1 overflow-y-auto p-6 space-y-5">
      <!-- 统计卡片 -->
      <div class="grid grid-cols-4 gap-3">
        <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-surface-900">{{ stats?.object_counts?.total ?? '-' }}</div><div class="text-[11px] text-surface-400">知识对象</div></div>
        <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-indigo-600">{{ stats?.health?.total ?? '-' }}</div><div class="text-[11px] text-surface-400">知识碎片</div></div>
        <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-orange-600">{{ stats?.health?.hot_count ?? '-' }}</div><div class="text-[11px] text-surface-400">热点碎片</div></div>
        <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-blue-600">{{ stats?.health?.cold_count ?? '-' }}</div><div class="text-[11px] text-surface-400">冷碎片</div></div>
      </div>
      <!-- 图表 -->
      <div class="grid grid-cols-2 gap-4">
        <div class="bg-white rounded-2xl border border-surface-200 p-4 content-card flex flex-col">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-3">对象分布</h3>
          <div class="flex-1 flex items-end gap-3 h-[120px]">
            <div v-for="item in (stats?.object_counts?.by_type || [])" :key="item.label" class="flex-1 flex flex-col items-center justify-end h-full group cursor-pointer">
              <div class="text-[18px] font-bold text-surface-800">{{ item.cnt }}</div>
              <div class="w-full rounded-t mt-1 transition-all duration-200 group-hover:opacity-80 group-hover:shadow-lg" :style="{ background: typeColor(item.label), height: Math.max(4, (item.cnt / Math.max(...(stats?.object_counts?.by_type || []).map((x: any) => x.cnt), 1)) * 100) + '%' }"></div>
              <div class="text-[10px] text-surface-400 mt-1">{{ typeLabel(item.label) }}</div>
            </div>
          </div>
        </div>
        <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-3">知识分布</h3>
          <VueApexCharts v-if="byTypeChart" :key="'bytype-' + chartVersion" type="donut" height="260" :series="byTypeChart.series" :options="{ labels: byTypeChart.labels, colors: byTypeChart.colors, legend: { position: 'bottom', fontSize: '12px', labels: { colors: '#64748b' } }, dataLabels: { enabled: true, formatter: (v: number) => v.toFixed(0) + '%' }, plotOptions: { pie: { donut: { size: '65%', labels: { show: true, total: { show: true, label: '总计', fontSize: '13px', color: '#334155' } } } } }, chart: { fontFamily: 'inherit' } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
        </div>
      </div>
      <div class="grid grid-cols-2 gap-4">
        <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-3">遗忘曲线</h3>
          <VueApexCharts v-if="weightDistChart" :key="'weight-' + chartVersion" type="area" height="260" :series="weightDistChart.series" :options="{ xaxis: { type: 'numeric', min: 0, max: 100, title: { text: '有效权重', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' }, formatter: (v: string) => { const n = parseFloat(v); if (n === 0) return '0%'; if (n === 100) return '100%'; return '' } }, tickAmount: 1 }, yaxis: { title: { text: '碎片数', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#6366f1'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.35, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 0 }, chart: { fontFamily: 'inherit', toolbar: { show: false }, zoom: { enabled: false } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
        </div>
        <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-3">知识增长趋势</h3>
          <VueApexCharts v-if="growthChart" :key="'growth-' + chartVersion" type="area" height="240" :series="growthChart.series" :options="{ xaxis: { categories: growthChart.categories, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, tickAmount: 15 }, yaxis: { title: { text: '新增碎片', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#06b6d4'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.4, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 3, strokeWidth: 1.5 }, chart: { fontFamily: 'inherit', toolbar: { show: false }, zoom: { enabled: false, mouseWheel: { enabled: false } } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
        </div>
      </div>
      <!-- 知识管线 -->
      <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
        <div class="flex items-center gap-2 mb-4">
          <h3 class="text-[12px] font-semibold text-surface-600">知识管线</h3>
          <span class="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded-full" :class="pipelineRunning ? 'bg-emerald-50 text-emerald-600' : 'bg-surface-50 text-surface-400'">
            <span class="w-1.5 h-1.5 rounded-full" :class="pipelineRunning ? 'bg-emerald-500 animate-pulse' : 'bg-surface-300'"></span>
            {{ pipelineRunning ? '运行中' : '空闲' }}
          </span>
        </div>
        <div class="space-y-3" v-if="pipelineData">
          <div>
            <div class="flex items-center justify-between mb-1">
              <span class="text-[11px] text-surface-500">物化进度</span>
              <span class="text-[11px] font-mono text-surface-600">{{ pipelineData.knowledge?.children ?? 0 }} / {{ pipelineData.source?.total ?? 0 }}</span>
            </div>
            <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all duration-500" :style="{ width: (pipelineData.source?.total ? Math.min(100, (pipelineData.knowledge?.children ?? 0) / pipelineData.source.total * 100) : 0) + '%', background: 'linear-gradient(90deg, #6366f1, #818cf8)' }"></div>
            </div>
            <div class="text-[10px] text-surface-400 mt-0.5 text-right">{{ pipelineData.source?.total ? ((pipelineData.knowledge?.children ?? 0) / pipelineData.source.total * 100).toFixed(1) : 0 }}%</div>
          </div>
          <div>
            <div class="flex items-center justify-between mb-1">
              <span class="text-[11px] text-surface-500">向量化进度</span>
              <span class="text-[11px] font-mono text-surface-600">{{ pipelineData.knowledge?.vectorized ?? 0 }} / {{ pipelineData.knowledge?.children ?? 0 }}</span>
            </div>
            <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all duration-500" :style="{ width: (pipelineData.knowledge?.children ? Math.min(100, (pipelineData.knowledge?.vectorized ?? 0) / pipelineData.knowledge.children * 100) : 0) + '%', background: 'linear-gradient(90deg, #06b6d4, #22d3ee)' }"></div>
            </div>
            <div class="text-[10px] text-surface-400 mt-0.5 text-right">{{ pipelineData.knowledge?.children ? ((pipelineData.knowledge?.vectorized ?? 0) / pipelineData.knowledge.children * 100).toFixed(1) : 0 }}%</div>
          </div>
          <div v-if="(pipelineData.fileParse?.pending ?? 0) + (pipelineData.fileParse?.done ?? 0) + (pipelineData.fileParse?.failed ?? 0) > 0">
            <div class="flex items-center justify-between mb-1">
              <span class="text-[11px] text-surface-500">文件解析</span>
              <span class="text-[11px] font-mono text-surface-600">
                <span class="text-emerald-600">{{ pipelineData.fileParse?.done ?? 0 }}</span>
                <span class="text-surface-300 mx-0.5">/</span>
                <span>{{ (pipelineData.fileParse?.done ?? 0) + (pipelineData.fileParse?.pending ?? 0) }}</span>
                <span v-if="pipelineData.fileParse?.failed" class="text-red-400 ml-1">({{ pipelineData.fileParse.failed }} 失败)</span>
              </span>
            </div>
            <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all duration-500" :style="{ width: (((pipelineData.fileParse?.done ?? 0) + (pipelineData.fileParse?.pending ?? 0)) ? Math.min(100, (pipelineData.fileParse?.done ?? 0) / ((pipelineData.fileParse?.done ?? 0) + (pipelineData.fileParse?.pending ?? 0)) * 100) : 0) + '%', background: 'linear-gradient(90deg, #f59e0b, #fbbf24)' }"></div>
            </div>
          </div>
        </div>
        <p v-else class="text-[11px] text-surface-300 text-center py-4">加载中...</p>
      </div>
      <!-- 操作按钮 -->
      <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
        <h3 class="text-[12px] font-semibold text-surface-600 mb-3">系统操作</h3>
        <div class="flex flex-wrap gap-2">
          <button @click="handleRevectorize" class="px-3 py-1 text-[11px] rounded-lg border border-blue-300 text-blue-600 hover:bg-blue-50 transition">重新向量化</button>
          <button @click="handleClearKnowledge" class="px-3 py-1 text-[11px] rounded-lg border border-amber-300 text-amber-600 hover:bg-amber-50 transition">重置知识管线</button>
          <button @click="handleClearAll" class="px-3 py-1 text-[11px] rounded-lg border border-red-300 text-red-600 hover:bg-red-50 transition">重置全部数据</button>
          <span class="w-px h-6 bg-surface-200 self-center" />
          <button @click="handleOrphanCleanup" class="px-3 py-1 text-[11px] rounded-lg border border-purple-300 text-purple-600 hover:bg-purple-50 transition">清理孤儿数据</button>
        </div>
      </div>
      <!-- 清理队列调试 -->
      <div v-if="debugData" class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
        <h3 class="text-[12px] font-semibold text-surface-600 mb-3">清理队列状态</h3>
        <div class="grid grid-cols-3 gap-2 mb-2">
          <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-amber-600">{{ debugData.stats.push_pending }}</div><div class="text-[9px] text-surface-400">待推送</div></div>
          <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-blue-500">{{ debugData.stats.push_processing }}</div><div class="text-[9px] text-surface-400">处理中</div></div>
          <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-red-500">{{ debugData.stats.push_failed }}</div><div class="text-[9px] text-surface-400">失败</div></div>
        </div>
        <div v-if="debugData.push_queue?.length">
          <h4 class="text-[10px] text-surface-400 mb-1">push_queue (最近 30 条)</h4>
          <div class="max-h-[200px] overflow-auto text-[8px] rounded border border-surface-200">
            <div v-for="r in debugData.push_queue" :key="r.id" class="px-1.5 py-0.5 border-b border-surface-100 flex items-center gap-1"
              :class="r.status==='failed'?'text-red-500':r.status==='processing'?'text-blue-500':'text-amber-600'">
              <span class="font-mono">{{ r.id }}</span>
              <span>{{ r.source_type }}/{{ r.source_id?.slice(0,8) }}...</span>
              <span class="text-[7px] text-surface-400">{{ r.operation }}</span>
              <span class="ml-auto">{{ r.status==='failed'?'✗':r.status==='processing'?'⟳':'⏳' }}</span>
            </div>
          </div>
        </div>
        <button @click="loadDebug" class="mt-2 px-2 py-1 text-[10px] rounded border border-surface-200 text-surface-500 hover:bg-surface-50">加载调试数据</button>
      </div>
    </div>
  </div>
</template>
<style scoped>
#kg-root .stat-card,
#kg-root .content-card { box-shadow: 0 1px 2px 0 rgba(0,0,0,0.05); transition-property: box-shadow, transform; transition-timing-function: ease; transition-duration: 200ms; }
#kg-root .stat-card:hover,
#kg-root .content-card:hover { transform: translateY(-2px); box-shadow: 0 4px 6px -1px rgba(0,0,0,0.1), 0 2px 4px -2px rgba(0,0,0,0.1); }
</style>
