<script setup lang="ts">
import { ref, computed, onActivated, onDeactivated } from 'vue'
import { useRouter } from 'vue-router'
import VueApexCharts from 'vue3-apexcharts'
import { kbDashboardStats, pipelineDebugStatus } from '@/api/db-client'
import { knowledgeTypeLabel, knowledgeTypeColor } from '@/utils/knowledgeTypes'
import RelationGraph from '@/components/knowledge/RelationGraph.vue'
import AppIcon from '@/components/common/AppIcon.vue'

const router = useRouter()

// ─── 统计图表（与图谱互补：图谱看关联，这里看分布/健康度） ───
const stats = ref<any>(null)
const statsLoading = ref(true)
const statsError = ref('')
async function loadStats() {
  try {
    const r = await kbDashboardStats()
    if (!r?.success) throw new Error('统计加载失败')
    stats.value = r
    statsError.value = ''
  } catch (e: any) {
    statsError.value = String(e?.message || e)
  } finally {
    statsLoading.value = false
  }
}

const byTypeChart = computed(() => {
  const d: any[] = stats.value?.by_type || []
  if (!d.length) return null
  return {
    series: d.map((x) => x.cnt),
    labels: d.map((x) => knowledgeTypeLabel(x.label)),
    colors: d.map((x) => knowledgeTypeColor(x.label)),
  }
})

const weightDistChart = computed(() => {
  const d: any[] = stats.value?.weight_dist || []
  if (!d.length) return null
  return { series: [{ name: '碎片数', data: d.map((x: any, i: number) => ({ x: i * 5, y: x.cnt || 0 })) }] }
})

// ─── 增长趋势：本地时区日切（toISOString 在 UTC+8 早8点前会错一天）───
const growthDays = ref(30)
function fmtLocalDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}
function setGrowthDays(d: number) { growthDays.value = d }
const growthChart = computed(() => {
  const d: any[] = stats.value?.daily_growth || []
  if (!d.length) return null
  const cutoff = fmtLocalDate(new Date(Date.now() - growthDays.value * 864e5))
  const filtered = d.filter((x) => (x.label || '') >= cutoff)
  const data = filtered.length > 0 ? filtered : d.slice(-Math.min(growthDays.value, d.length))
  let cumulative = 0
  return {
    series: [{ name: '窗口累计', data: data.map((x) => { cumulative += x.cnt || 0; return cumulative }) }],
    categories: data.map((x) => (x.label || '').slice(5)),
  }
})

// ─── 管线健康（原「清理队列」死代码块的正名替换：命令真实存在、自动加载）───
const pipe = ref<any>(null)
const pipeOpen = ref(false)
const pipeError = ref('')
async function loadPipe() {
  try {
    pipe.value = await pipelineDebugStatus()
    pipeError.value = ''
  } catch (e: any) {
    pipeError.value = String(e?.message || e)
  }
}
const vecPct = computed(() => {
  const k = pipe.value?.knowledge
  if (!k?.childrenNonempty) return null
  return Math.round(((k.vectorizedValid || 0) / k.childrenNonempty) * 100)
})
const pipeBad = computed(() => {
  const k = pipe.value?.knowledge, f = pipe.value?.fileParse
  return (k?.embedPending || 0) > 0 || (f?.failed || 0) > 0 || !!k?.lastEmbedError
})

// ─── 图谱刷新 ───
const graphRef = ref<InstanceType<typeof RelationGraph> | null>(null)
function refreshAll() {
  loadStats()
  loadPipe()
  graphRef.value?.reload()
}

// ─── 60s 轮询（原 5s 全量重建太激进；数据未变不再销毁图表）───
let timer: ReturnType<typeof setInterval> | null = null
onActivated(() => {
  refreshAll()
  timer = setInterval(() => { loadStats(); loadPipe(); graphRef.value?.reload() }, 60000)
})
onDeactivated(() => { if (timer) { clearInterval(timer); timer = null } })

const isEmptyKb = computed(() => stats.value && (stats.value.health?.total || 0) === 0 && (stats.value.object_counts?.total || 0) === 0)
</script>

<template>
  <div id="kg-root" class="h-full flex flex-col bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden">
    <div class="flex-1 overflow-y-auto p-4 space-y-4">
      <!-- 统计全局错误/加载 -->
      <div v-if="statsError" class="rounded-xl border border-red-100 bg-red-50/50 px-4 py-3 flex items-center justify-between">
        <p class="text-[11px] text-red-400">知识统计加载失败：{{ statsError }}</p>
        <button @click="loadStats" class="btn-secondary text-xs">重试</button>
      </div>
      <div v-else-if="statsLoading && !stats" class="grid grid-cols-3 gap-4">
        <div v-for="i in 3" :key="i" class="h-40 rounded-2xl bg-surface-50 animate-pulse"></div>
      </div>
      <div v-else-if="isEmptyKb" class="rounded-2xl border border-surface-100 p-8 text-center">
        <p class="text-[13px] text-surface-600 font-medium">知识库还是空的</p>
        <p class="text-[11px] text-surface-400 mt-1.5 leading-relaxed">先到「资料库」上传文件，或在「设置」配置向量模型——有了知识，观测台才有东西可看。</p>
        <div class="flex items-center justify-center gap-2 mt-4">
          <button @click="router.push('/assets')" class="btn-primary text-xs">去上传资料</button>
          <button @click="router.push('/settings')" class="btn-secondary text-xs">去设置</button>
        </div>
      </div>

      <!-- 图谱主视图（开放画布，无标题无卡框，工具条即入口） -->
      <div class="flex flex-col" style="height: 640px">
        <div class="flex-1 min-h-0">
          <RelationGraph ref="graphRef" @refresh="refreshAll" />
        </div>
      </div>

      <!-- 三块统计图 -->
      <div class="grid grid-cols-3 gap-4">
        <div class="rounded-2xl border border-surface-200 bg-white p-4">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-2">知识分布</h3>
          <VueApexCharts v-if="byTypeChart" type="donut" height="220" :series="byTypeChart.series" :options="{ labels: byTypeChart.labels, colors: byTypeChart.colors, legend: { position: 'bottom', fontSize: '11px', labels: { colors: '#64748b' } }, dataLabels: { enabled: true, formatter: (v: number) => v.toFixed(0) + '%' }, plotOptions: { pie: { donut: { size: '65%', labels: { show: true, total: { show: true, label: '总计', fontSize: '12px', color: '#334155' } } } } }, chart: { fontFamily: 'inherit', toolbar: { show: false } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-16">暂无数据</p>
        </div>
        <div class="rounded-2xl border border-surface-200 bg-white p-4">
          <h3 class="text-[12px] font-semibold text-surface-600 mb-2">遗忘曲线 <span class="text-[9px] font-normal text-surface-300">有效权重分布</span></h3>
          <VueApexCharts v-if="weightDistChart" type="area" height="220" :series="weightDistChart.series" :options="{ xaxis: { type: 'numeric', min: 0, max: 100, title: { text: '有效权重', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' }, formatter: (v: string) => { const n = parseFloat(v); return n === 0 ? '0%' : n === 100 ? '100%' : '' } }, tickAmount: 1 }, yaxis: { title: { text: '碎片数', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#6366f1'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.35, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 0 }, chart: { fontFamily: 'inherit', toolbar: { show: false }, zoom: { enabled: false } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-16">暂无数据</p>
        </div>
        <div class="rounded-2xl border border-surface-200 bg-white p-4 flex flex-col">
          <div class="flex items-center justify-between mb-2">
            <h3 class="text-[12px] font-semibold text-surface-600">知识增长</h3>
            <div class="flex items-center gap-0.5">
              <button v-for="opt in [{d:7,l:'7天'},{d:30,l:'30天'},{d:90,l:'90天'},{d:180,l:'半年'},{d:365,l:'一年'}]" :key="opt.d" @click="setGrowthDays(opt.d)" class="px-1.5 py-0.5 rounded text-[9px] transition-colors" :class="growthDays===opt.d ? 'bg-blue-50 text-blue-600 font-medium' : 'text-surface-500 hover:bg-surface-50'">{{ opt.l }}</button>
            </div>
          </div>
          <VueApexCharts v-if="growthChart" type="area" height="220" :series="growthChart.series" :options="{ xaxis: { categories: growthChart.categories, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, tickAmount: 8 }, yaxis: { title: { text: '累计碎片（窗口内）', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#06b6d4'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.4, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 0 }, chart: { fontFamily: 'inherit', toolbar: { show: false }, zoom: { enabled: false } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-16">暂无数据</p>
        </div>
      </div>

      <!-- 管线健康（数据源 pipeline_debug_status，此前是永远显示不出来的死代码） -->
      <div class="rounded-2xl border border-surface-200 bg-white overflow-hidden">
        <button @click="pipeOpen = !pipeOpen" class="w-full flex items-center gap-2 px-4 py-2.5 text-left hover:bg-surface-50/50 transition-colors">
          <span class="w-1.5 h-1.5 rounded-full flex-shrink-0" :class="pipeBad ? 'bg-amber-400' : 'bg-emerald-500'"></span>
          <h3 class="text-[12px] font-semibold text-surface-600 flex-shrink-0">管线健康</h3>
          <span v-if="pipe" class="flex items-center gap-3 text-[10px] text-surface-400 min-w-0">
            <span>向量化 <span class="tabular-nums" :class="vecPct !== null && vecPct < 95 ? 'text-amber-500 font-medium' : 'text-emerald-600'">{{ vecPct === null ? '-' : vecPct + '%' }}</span></span>
            <span v-if="(pipe.knowledge?.embedPending || 0) > 0" class="text-amber-500">积压 {{ pipe.knowledge.embedPending }}</span>
            <span v-if="(pipe.fileParse?.failed || 0) > 0" class="text-red-400">解析失败 {{ pipe.fileParse.failed }}</span>
            <span v-if="(pipe.fileParse?.unsupported || 0) > 0">不支持 {{ pipe.fileParse.unsupported }}</span>
          </span>
          <span v-if="pipeError" class="text-[10px] text-red-400">管线状态加载失败</span>
          <AppIcon name="chevron-down" class="w-3 h-3 text-surface-400 transition-transform ml-auto flex-shrink-0" :class="pipeOpen ? 'rotate-180' : ''" />
        </button>
        <div v-if="pipeOpen && pipe" class="px-4 pb-3 pt-2.5">
          <div class="grid grid-cols-4 gap-2 mb-2">
            <div class="rounded-lg bg-surface-50 p-2 text-center"><div class="text-[13px] font-bold text-surface-700 tabular-nums">{{ pipe.source?.total ?? '-' }}</div><div class="text-[9px] text-surface-400">知识对象</div></div>
            <div class="rounded-lg bg-surface-50 p-2 text-center"><div class="text-[13px] font-bold text-indigo-600 tabular-nums">{{ pipe.knowledge?.children ?? '-' }}</div><div class="text-[9px] text-surface-400">子碎片</div></div>
            <div class="rounded-lg bg-surface-50 p-2 text-center"><div class="text-[13px] font-bold" :class="(pipe.knowledge?.embedPending || 0) > 0 ? 'text-amber-500' : 'text-emerald-600'">{{ pipe.knowledge?.vectorizedValid ?? '-' }}</div><div class="text-[9px] text-surface-400">已向量化</div></div>
            <div class="rounded-lg bg-surface-50 p-2 text-center"><div class="text-[13px] font-bold" :class="(pipe.fileParse?.failed || 0) > 0 ? 'text-red-500' : 'text-surface-700'">{{ pipe.fileParse?.failed ?? '-' }}</div><div class="text-[9px] text-surface-400">解析失败</div></div>
          </div>
          <p v-if="pipe.knowledge?.lastEmbedError" class="text-[10px] text-red-400 truncate mb-2" v-tooltip="pipe.knowledge.lastEmbedError">向量化错误：{{ pipe.knowledge.lastEmbedError }}</p>
          <div class="flex items-center justify-between">
            <p class="text-[9px] text-surface-300">解析中 {{ pipe.fileParse?.processing ?? 0 }} · 待解析 {{ pipe.fileParse?.pending ?? 0 }} · 完成 {{ pipe.fileParse?.done ?? 0 }}/{{ pipe.fileParse?.total ?? 0 }}</p>
            <button @click="router.push('/settings')" class="btn-secondary text-[10px]">去设置页处理</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
#kg-root .rounded-2xl {
  box-shadow: 0 1px 2px 0 rgba(0,0,0,0.04);
  transition: box-shadow 0.2s ease, border-color 0.2s ease;
}
#kg-root .rounded-2xl:hover {
  box-shadow: 0 4px 6px -1px rgba(0,0,0,0.1), 0 2px 4px -2px rgba(0,0,0,0.1);
}
</style>
