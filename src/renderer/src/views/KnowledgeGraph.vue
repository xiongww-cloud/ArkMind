<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import VueApexCharts from 'vue3-apexcharts'
import AppIcon from '@/components/common/AppIcon.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import {
  kbDashboardStats, tagList, tagCreate, tagDelete, tagListCategories, tagStats,
  pipelineDebugStatus,
  pipelineClearKnowledge, pipelineClearAll, pipelineClearTagPipeline,
  aiSyncRetryFailed, cleanupRetryFailed,
  kbPipelineDebug,
} from '@/api/db-client'

type ViewType = 'dashboard' | 'tags' | 'status'
const activeView = ref<ViewType>('dashboard')
const stats = ref<any>(null)
const growthDays = ref(30)
async function loadStats() { try { const r = await kbDashboardStats(); if (r?.success) stats.value = r } catch (e) { console.error('[KG] loadStats failed:', e) } }
function setGrowthDays(d: number) { growthDays.value = d }

const growthChart = computed(() => {
  if (!stats.value?.daily_growth?.length) return null
  const d = stats.value.daily_growth
  const cutoff = new Date(); cutoff.setDate(cutoff.getDate() - growthDays.value)
  const cutoffStr = cutoff.toISOString().slice(0, 10)
  const filtered = d.filter((x: any) => (x.label || '') >= cutoffStr)
  const data = filtered.length > 0 ? filtered : d.slice(-growthDays.value)
  return { series: [{ name: '通用', data: data.map((x: any) => x.cnt) }], categories: data.map((x: any) => (x.label || '').slice(5)) }
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

const newTagName = ref(''); const newTagCategory = ref('general')
const allTags = ref<any[]>([]); const categories = ref<string[]>([])
const tagStatsData = ref<any>(null); const showAllCloudTags = ref(false); const showCategoryDropdown = ref(false)
const categoryOptions = [
  { value: 'general', label: '通用' }, { value: 'scene', label: '场景' }, { value: 'tech', label: '技术' },
  { value: 'product', label: '产品' }, { value: 'daily', label: '日常' }, { value: 'custom', label: '自定义' },
]
const groupedTags = computed(() => {
  const g: Record<string,any[]> = {}; for (const cat of categories.value) g[cat] = []
  for (const t of allTags.value) { if (!g[t.type]) g[t.type] = []; g[t.type].push(t) }; return g
})
async function loadTags() {
  try { const [t, c, s] = await Promise.all([tagList(), tagListCategories(), tagStats()]); allTags.value = Array.isArray(t) ? t : []; categories.value = Array.isArray(c) ? c.map((x: any) => x.type || x) : []; if (s?.success) tagStatsData.value = s } catch { allTags.value = []; categories.value = [] }
}
async function handleCreateTag() { if (!newTagName.value.trim()) return; try { await tagCreate(newTagName.value.trim(), '', newTagCategory.value); newTagName.value = ''; await loadTags() } catch (e) { console.error('[KG] createTag failed:', e) } }
async function handleDeleteTag(tagId: string) { try { await tagDelete(tagId); await loadTags() } catch (e) { console.error('[KG] deleteTag failed:', e) } }

const pipelineStatus = ref<any>({})
async function loadPipelineStatus() { try { pipelineStatus.value = await pipelineDebugStatus() || {} } catch (e) { console.error('[KG] loadPipelineStatus failed:', e) } }
async function handleClearAll() { if (!(await dangerConfirm({ title: '重置全部数据', message: '确定重置全部数据？将重新从业务表抓取数据并重新物化打标。', confirmText: '重置' }))) return; try { await pipelineClearAll(); toast({ message: '已重置全部数据，正在重新物化打标', type: 'success' }); await loadStats(); await loadPipelineStatus() } catch (e) { console.error('[KG] clearAll failed:', e); toast({ message: '重置失败', type: 'error' }) } }
async function handleClearKnowledge() { if (!(await dangerConfirm({ title: '重置知识管线', message: '确定重置知识碎片和向量缓存？将重新从源数据物化。', confirmText: '重置' }))) return; try { await pipelineClearKnowledge(); toast({ message: '已重置知识管线，正在重新物化', type: 'success' }); await loadStats(); await loadPipelineStatus() } catch (e) { console.error('[KG] clearKnowledge failed:', e); toast({ message: '重置失败', type: 'error' }) } }
async function handleClearTagPipeline() { if (!(await dangerConfirm({ title: '重置标签管线', message: '确定重置所有标签映射？将清除已有标签并重新打标。', confirmText: '重置' }))) return; try { await pipelineClearTagPipeline(); toast({ message: '已重置标签管线，正在重新打标', type: 'success' }); await loadPipelineStatus() } catch (e) { console.error('[KG] clearTagPipeline failed:', e); toast({ message: '重置失败', type: 'error' }) } }
async function handleRetryFailedSync() { try { const r = await aiSyncRetryFailed(); toast({ message: `已重置 ${r?.retried ?? 0} 个失败同步任务`, type: 'success' }); await loadStats(); await loadPipelineStatus() } catch (e) { console.error('[KG] retryFailedSync failed:', e); toast({ message: '重试失败', type: 'error' }) } }
async function handleRetryFailedCleanup() { try { const r = await cleanupRetryFailed(); toast({ message: `已重置 ${r?.retried ?? 0} 个失败清理任务`, type: 'success' }); await loadStats(); await loadPipelineStatus() } catch (e) { console.error('[KG] retryFailedCleanup failed:', e); toast({ message: '重试失败', type: 'error' }) } }

const debugData = ref<any>(null)
async function loadDebug() { try { debugData.value = await kbPipelineDebug() } catch (e) { console.error('[KG] loadDebug failed:', e) } }
function typeLabel(st: string): string { const m: Record<string,string> = { FILE:'文件',NOTE:'笔记',PRODUCT:'产品',PROJECT:'项目',TODO:'待办' }; return m[st] || st || '通用' }
function typeColor(st: string): string { const m: Record<string,string> = { FILE:'#60a5fa',NOTE:'#fbbf24',PRODUCT:'#a78bfa',PROJECT:'#34d399',TODO:'#94a3b8' }; return m[st] || '#94a3b8' }
function categoryLabel(c: string): string { const m: Record<string,string> = { general:'通用',scene:'场景',tech:'技术',product:'产品',daily:'日常',custom:'自定义' }; return m[c] || c }
const categoryColors: Record<string,string> = { general:'#6366f1', scene:'#06b6d4', tech:'#10b981', product:'#f59e0b', daily:'#f472b6', custom:'#8b5cf6' }
function tagHeatColor(tag: any): string {
  const base = categoryColors[tag.type] || tag.color || '#6366f1'
  const usage = (tagStatsData.value?.tags || []).find((t: any) => t.tag_id === tag.tag_id)?.usage_count ?? 0
  if (usage <= 0) return '#94a3b8'
  if (usage <= 3) return base + '80'
  if (usage <= 9) return base + 'CC'
  return base
}
onMounted(() => { loadStats(); loadTags(); loadPipelineStatus(); document.addEventListener('click', () => { showCategoryDropdown.value = false }) })
</script>
<template>
  <div id="kg-root" class="h-full flex flex-col bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden">
    <div class="flex items-center bg-surface-50 pt-1.5 px-3 border-b border-surface-200 shrink-0">
      <nav class="flex gap-0 -mb-px">
        <button @click="activeView = 'dashboard'" class="tab flex items-center gap-1.5" :class="activeView === 'dashboard' ? 'tab-active' : ''">
          <AppIcon name="bar-chart" class="h-4 w-4" />数据概览</button>
        <button @click="activeView = 'tags'" class="tab flex items-center gap-1.5" :class="activeView === 'tags' ? 'tab-active' : ''">
          <AppIcon name="flag" class="h-4 w-4" />标签管理</button>
        <button @click="activeView = 'status'" class="tab flex items-center gap-1.5" :class="activeView === 'status' ? 'tab-active' : ''">
          <AppIcon name="settings" class="h-4 w-4" />系统状态</button>
      </nav>
    </div>
    <div class="flex-1 overflow-y-auto">
      <div v-if="activeView === 'dashboard'" class="p-6 space-y-5">
        <div class="grid grid-cols-5 gap-3">
          <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-surface-900">{{ stats?.object_counts?.total ?? '-' }}</div><div class="text-[11px] text-surface-400">知识对象</div></div>
          <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-indigo-600">{{ stats?.health?.total ?? '-' }}</div><div class="text-[11px] text-surface-400">知识碎片</div></div>
          <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-violet-600">{{ stats?.health?.tag_count ?? '-' }}</div><div class="text-[11px] text-surface-400">用户标签</div></div>
          <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-orange-600">{{ stats?.health?.hot_count ?? '-' }}</div><div class="text-[11px] text-surface-400">热点碎片</div></div>
          <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-blue-600">{{ stats?.health?.cold_count ?? '-' }}</div><div class="text-[11px] text-surface-400">冷碎片</div></div>
        </div>
        <div class="grid grid-cols-2 gap-4">
          <div class="bg-white rounded-2xl border border-surface-200 p-4 content-card">
            <h3 class="text-[12px] font-semibold text-surface-600 mb-3">对象分布</h3>
            <div class="flex items-end gap-3">
              <div v-for="item in (stats?.object_counts?.by_type || [])" :key="item.label" class="flex-1 text-center">
                <div class="text-[18px] font-bold text-surface-800">{{ item.cnt }}</div>
                <div class="h-1.5 rounded-full mt-1.5 mx-auto w-full" :style="{ background: typeColor(item.label), height: Math.max(4, item.cnt / Math.max(stats?.object_counts?.total || 1, 1) * 60) + 'px' }"></div>
                <div class="text-[10px] text-surface-400 mt-1">{{ typeLabel(item.label) }}</div>
              </div>
            </div>
          </div>
          <div class="bg-white rounded-2xl border border-surface-200 p-4 content-card">
            <h3 class="text-[12px] font-semibold text-surface-600 mb-3">管线状态</h3>
            <div class="grid grid-cols-5 gap-2 text-center pt-2">
              <div><div class="text-[22px] font-bold text-emerald-600">{{ stats?.pipeline?.embedded ?? '-' }}</div><div class="text-[10px] text-surface-400 mt-1">已嵌入</div></div>
              <div><div class="text-[22px] font-bold text-amber-600">{{ stats?.pipeline?.pending ?? '-' }}</div><div class="text-[10px] text-surface-400 mt-1">待处理</div></div>
              <div><div class="text-[22px] font-bold text-red-500">{{ stats?.pipeline?.failed ?? '-' }}</div><div class="text-[10px] text-surface-400 mt-1">失败</div></div>
              <div><div class="text-[22px] font-bold text-emerald-600">{{ stats?.pipeline?.tagged ?? '-' }}</div><div class="text-[10px] text-surface-400 mt-1">已打标</div></div>
              <div><div class="text-[22px] font-bold text-surface-400">{{ stats?.pipeline?.untagged ?? '-' }}</div><div class="text-[10px] text-surface-400 mt-1">待打标</div></div>
            </div>
          </div>
        </div>
        <div class="grid grid-cols-2 gap-4">
          <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
            <h3 class="text-[12px] font-semibold text-surface-600 mb-3">知识分布</h3>
            <VueApexCharts v-if="byTypeChart" type="donut" height="260" :series="byTypeChart.series" :options="{ labels: byTypeChart.labels, colors: byTypeChart.colors, legend: { position: 'bottom', fontSize: '12px', labels: { colors: '#64748b' } }, dataLabels: { enabled: true, formatter: (v: number) => v.toFixed(0) + '%' }, plotOptions: { pie: { donut: { size: '65%', labels: { show: true, total: { show: true, label: '总计', fontSize: '13px', color: '#334155' } } } } }, chart: { fontFamily: 'inherit' } }" />
            <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
          </div>
          <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
            <h3 class="text-[12px] font-semibold text-surface-600 mb-3">遗忘曲线</h3>
            <VueApexCharts v-if="weightDistChart" type="area" height="260" :series="weightDistChart.series" :options="{ xaxis: { type: 'numeric', min: 0, max: 100, title: { text: '有效权重', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' }, formatter: (v: string) => { const n = parseFloat(v); if (n === 0) return '0%'; if (n === 100) return '100%'; return '' } }, tickAmount: 1 }, yaxis: { title: { text: '碎片数', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#6366f1'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.35, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 0 }, chart: { fontFamily: 'inherit', toolbar: { show: false }, zoom: { enabled: false } } }" />
            <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
          </div>
        </div>
        <div class="bg-white rounded-2xl border border-surface-200 p-5 content-card">
          <div class="flex items-center justify-between mb-3">
            <h3 class="text-[12px] font-semibold text-surface-600">知识增长趋势</h3>
            <div class="flex items-center gap-1">
              <button @click="setGrowthDays(1)" class="text-[10px] px-2 py-1 rounded transition-colors" :class="growthDays === 1 ? 'bg-brand-100 text-brand-700 font-medium' : 'text-surface-400 hover:bg-surface-100'">今天</button>
              <button @click="setGrowthDays(7)" class="text-[10px] px-2 py-1 rounded transition-colors" :class="growthDays === 7 ? 'bg-brand-100 text-brand-700 font-medium' : 'text-surface-400 hover:bg-surface-100'">近7天</button>
              <button @click="setGrowthDays(30)" class="text-[10px] px-2 py-1 rounded transition-colors" :class="growthDays === 30 ? 'bg-brand-100 text-brand-700 font-medium' : 'text-surface-400 hover:bg-surface-100'">近30天</button>
              <button @click="setGrowthDays(365)" class="text-[10px] px-2 py-1 rounded transition-colors" :class="growthDays === 365 ? 'bg-brand-100 text-brand-700 font-medium' : 'text-surface-400 hover:bg-surface-100'">近一年</button>
            </div>
          </div>
          <VueApexCharts v-if="growthChart" type="area" height="240" :series="growthChart.series" :options="{ xaxis: { categories: growthChart.categories, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, tickAmount: 15 }, yaxis: { title: { text: '新增碎片', style: { fontSize: '11px', color: '#94a3b8' } }, labels: { style: { fontSize: '10px', colors: '#94a3b8' } }, min: 0 }, colors: ['#06b6d4'], stroke: { curve: 'smooth', width: 2.5 }, fill: { type: 'gradient', gradient: { shadeIntensity: 1, opacityFrom: 0.4, opacityTo: 0.05 } }, dataLabels: { enabled: false }, markers: { size: 3, strokeWidth: 1.5 }, chart: { fontFamily: 'inherit', toolbar: { show: false } } }" />
          <p v-else class="text-[11px] text-surface-300 text-center py-12">暂无数据</p>
        </div>
      </div>

      <div v-if="activeView === 'tags'" class="p-6">
        <div class="grid grid-cols-12 gap-5">
          <div class="col-span-5 space-y-4">
            <div class="grid grid-cols-2 gap-3">
              <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-surface-900">{{ tagStatsData?.total_tags ?? '-' }}</div><div class="text-[11px] text-surface-400">总标签数</div></div>
              <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-emerald-600">{{ tagStatsData?.used_tags ?? '-' }}</div><div class="text-[11px] text-surface-400">已使用</div></div>
              <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-amber-600">{{ tagStatsData?.unused_tags ?? '-' }}</div><div class="text-[11px] text-surface-400">未使用</div></div>
              <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[22px] font-bold text-blue-600">{{ tagStatsData?.coverage ?? '-' }}%</div><div class="text-[11px] text-surface-400">覆盖率</div></div>
            </div>
            <div class="bg-white rounded-xl border border-surface-200 p-4 content-card">
              <h3 class="text-[12px] font-semibold text-surface-600 mb-3">使用排名</h3>
              <div v-if="tagStatsData?.top_tags?.length" class="space-y-2">
                <div v-for="(t, i) in tagStatsData.top_tags" :key="t.tag_id" class="flex items-center gap-2.5">
                  <span class="text-[11px] font-bold text-surface-300 w-4 text-right">{{ i + 1 }}</span>
                  <span class="w-2.5 h-2.5 rounded-full shrink-0" :style="{ background: tagHeatColor(t) }"></span>
                  <span class="flex-1 text-[12px] text-surface-700 truncate">{{ t.name }}</span>
                  <span class="text-[11px] font-semibold text-surface-400">{{ t.usage_count }}</span>
                </div>
              </div>
              <p v-else class="text-[11px] text-surface-300 text-center py-4">暂无使用数据</p>
            </div>
          </div>
          <div class="col-span-7 space-y-5">
            <div class="bg-white rounded-xl border border-surface-200 p-4 content-card">
              <h3 class="text-[12px] font-semibold text-surface-600 mb-3">新建标签</h3>
              <div class="flex items-center gap-2">
                <input v-model="newTagName" @keyup.enter="handleCreateTag" placeholder="输入标签名称" class="flex-1 border border-surface-200 rounded-lg px-3 py-1.5 text-[11px] outline-none focus:border-brand-400"/>
                <div class="relative">
                  <button @click.stop="showCategoryDropdown = !showCategoryDropdown" class="border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] text-left outline-none focus:border-brand-400 bg-white min-w-[72px] flex items-center justify-between gap-1">
                    <span>{{ categoryLabel(newTagCategory) }}</span>
                    <svg class="w-3 h-3 text-surface-300 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M19 9l-7 7-7-7"/></svg>
                  </button>
                  <Transition name="dropdown">
                    <div v-if="showCategoryDropdown" class="absolute right-0 top-full mt-1 z-[80] bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[80px]">
                      <button v-for="opt in categoryOptions" :key="opt.value" @click="newTagCategory = opt.value; showCategoryDropdown = false" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors hover:bg-surface-50" :class="newTagCategory === opt.value ? 'text-brand-700 font-medium bg-brand-50' : 'text-surface-600'">{{ opt.label }}</button>
                    </div>
                  </Transition>
                </div>
                <button @click="handleCreateTag" :disabled="!newTagName.trim()" class="shrink-0 h-7 px-3 text-[10px] rounded-lg bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-40 transition-colors">创建</button>
              </div>
            </div>
            <div class="bg-white rounded-xl border border-surface-200 p-4 content-card">
              <h3 class="text-[12px] font-semibold text-surface-600 mb-3">分类统计</h3>
              <div v-if="tagStatsData?.by_category?.length" class="space-y-2">
                <div v-for="cat in tagStatsData.by_category" :key="cat.label" class="flex items-center gap-3">
                  <span class="text-[11px] text-surface-500 w-12 shrink-0">{{ categoryLabel(cat.label) }}</span>
                  <div class="flex-1 bg-surface-100 rounded-full h-2"><div class="h-2 rounded-full bg-brand-500 transition-all duration-500" :style="{ width: (cat.count / Math.max(tagStatsData.total_tags, 1) * 100) + '%' }"></div></div>
                  <span class="text-[11px] font-semibold text-surface-600 w-6 text-right">{{ cat.count }}</span>
                </div>
              </div>
              <p v-else class="text-[11px] text-surface-300 text-center py-4">暂无数据</p>
            </div>
            <div class="bg-white rounded-xl border border-surface-200 p-4 content-card">
              <h3 class="text-[12px] font-semibold text-surface-600 mb-3">按分类浏览</h3>
              <div v-for="cat in categories" :key="cat" class="mb-3">
                <p class="text-[10px] font-semibold text-surface-400 uppercase tracking-wider mb-1.5">{{ categoryLabel(cat) }}</p>
                <div class="flex flex-wrap gap-1.5">
                  <div v-for="tag in groupedTags[cat]||[]" :key="tag.tag_id" class="group inline-flex items-center gap-1.5 px-2.5 py-1 text-[10px] rounded-lg border cursor-default transition-all hover:shadow-sm" :style="{'background':tagHeatColor(tag)+'20','border-color':tagHeatColor(tag)+'40','color':tagHeatColor(tag)}"><span class="w-1.5 h-1.5 rounded-full shrink-0" :style="{'background':tagHeatColor(tag)}"></span><span>{{ tag.name }}</span><button @click="handleDeleteTag(tag.tag_id)" class="ml-0.5 w-3.5 h-3.5 flex items-center justify-center rounded-full opacity-0 group-hover:opacity-100 hover:bg-red-50 transition-all"><svg class="w-2.5 h-2.5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 18L18 6M6 6l12 12"/></svg></button></div>
                </div>
              </div>
              <p v-if="allTags.length===0" class="text-[11px] text-surface-300 text-center py-4">暂无标签</p>
            </div>
          </div>
        </div>
      </div>

      <div v-if="activeView === 'status'" class="p-6 space-y-5">
        <div>
          <h3 class="text-[12px] font-semibold text-surface-500 uppercase tracking-wider mb-3">数据摄入中台</h3>
          <div class="grid grid-cols-3 gap-3">
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-blue-600">{{ pipelineStatus?.source?.total ?? '-' }}</div><div class="text-[11px] text-surface-400">已摄入对象</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-amber-600">{{ pipelineStatus?.sync?.pending ?? '-' }}</div><div class="text-[11px] text-surface-400">待摄入</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-red-500">{{ pipelineStatus?.sync?.failed ?? '-' }}</div><div class="text-[11px] text-surface-400">失败</div></div>
          </div>
        </div>
        <div>
          <h3 class="text-[12px] font-semibold text-surface-500 uppercase tracking-wider mb-3">知识管线</h3>
          <div class="grid grid-cols-5 gap-3">
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-surface-900">{{ pipelineStatus?.sync?.pending ?? '-' }}</div><div class="text-[11px] text-surface-400">待同步</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-emerald-600">{{ pipelineStatus?.sync?.done ?? '-' }}</div><div class="text-[11px] text-surface-400">已同步</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-red-500">{{ pipelineStatus?.sync?.failed ?? '-' }}</div><div class="text-[11px] text-surface-400">同步失败</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-indigo-600">{{ pipelineStatus?.knowledge?.children ?? '-' }}</div><div class="text-[11px] text-surface-400">知识碎片</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-blue-600">{{ pipelineStatus?.knowledge?.vectorized ?? '-' }}</div><div class="text-[11px] text-surface-400">已向量化</div></div>
          </div>
        </div>
        <div>
          <h3 class="text-[12px] font-semibold text-surface-500 uppercase tracking-wider mb-3">标签管线</h3>
          <div class="grid grid-cols-3 gap-3">
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-amber-600">{{ pipelineStatus?.tag?.pending ?? '-' }}</div><div class="text-[11px] text-surface-400">总标签数</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-emerald-600">{{ pipelineStatus?.tag?.done ?? '-' }}</div><div class="text-[11px] text-surface-400">今天</div></div>
            <div class="rounded-xl border border-surface-200 p-4 stat-card"><div class="text-[20px] font-bold text-red-500">{{ pipelineStatus?.tag?.failed ?? '-' }}</div><div class="text-[11px] text-surface-400">失败</div></div>
          </div>
        </div>
        <div>
          <h3 class="text-[12px] font-semibold text-surface-500 uppercase tracking-wider mb-3">操作</h3>
          <div class="flex flex-wrap gap-2">
            <button @click="loadStats(); loadPipelineStatus()" class="px-3 py-1 text-[11px] rounded-lg bg-brand-600 text-white hover:bg-brand-700 transition flex items-center gap-1">
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>刷新
            </button>
            <button @click="handleClearKnowledge" class="px-3 py-1 text-[11px] rounded-lg border border-amber-300 text-amber-600 hover:bg-amber-50 transition">重置知识管线</button>
            <button @click="handleClearTagPipeline" class="px-3 py-1 text-[11px] rounded-lg border border-amber-300 text-amber-600 hover:bg-amber-50 transition">重置标签管线</button>
            <button @click="handleClearAll" class="px-3 py-1 text-[11px] rounded-lg border border-red-300 text-red-600 hover:bg-red-50 transition">重置全部数据</button>
            <span class="w-px h-6 bg-surface-200 self-center" />
            <button @click="handleRetryFailedSync" class="px-3 py-1 text-[11px] rounded-lg border border-blue-300 text-blue-600 hover:bg-blue-50 transition">重试失败同步</button>
            <button @click="handleRetryFailedCleanup" class="px-3 py-1 text-[11px] rounded-lg border border-blue-300 text-blue-600 hover:bg-blue-50 transition">重试失败清理</button>
            <router-link to="/inspector" class="px-3 py-1 text-[11px] rounded-lg border border-surface-300 text-surface-500 hover:bg-surface-50 transition flex items-center gap-1">
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>
              索引检查
            </router-link>
          </div>
        </div>

        <!-- Pipeline Debug -->
        <div v-if="debugData" class="mt-2">
          <h3 class="text-[12px] font-semibold text-surface-500 uppercase tracking-wider mb-3">管道调试</h3>
          <div class="grid grid-cols-5 gap-2 mb-2">
            <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-emerald-600">{{ debugData.stats.sync_ok }}</div><div class="text-[9px] text-surface-400">已嵌入</div></div>
            <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-blue-600">{{ debugData.stats.sync_pending }}</div><div class="text-[9px] text-surface-400">待处理</div></div>
            <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-red-500">{{ debugData.stats.sync_fail }}</div><div class="text-[9px] text-surface-400">失败</div></div>
            <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-amber-600">{{ debugData.stats.cleanup_pending }}</div><div class="text-[9px] text-surface-400">待清理</div></div>
            <div class="rounded border border-surface-200 p-2 text-center"><div class="text-[14px] font-bold text-slate-500">{{ debugData.stats.cleanup_done }}</div><div class="text-[9px] text-surface-400">已清理</div></div>
          </div>
          <div class="grid grid-cols-2 gap-3">
            <div>
              <h4 class="text-[10px] text-surface-400 mb-1">ai_sync_queue (最近 30 条)</h4>
              <div class="max-h-[200px] overflow-auto text-[8px] rounded border border-surface-200">
                <div v-for="r in debugData.sync_queue" :key="r.id" class="px-1.5 py-0.5 border-b border-surface-100 flex items-center gap-1"
                  :class="r.is_embedded===1?'text-emerald-600':r.is_embedded===-1?'text-red-500':'text-blue-600'">
                  <span class="font-mono">{{ r.id }}</span>
                  <span>{{ r.source_type }}/{{ r.source_id?.slice(0,8) }}...</span>
                  <span class="ml-auto" :title="r.last_error">{{ r.is_embedded===1?'✓':r.is_embedded===-1?'✗':'⏳' }}</span>
                </div>
              </div>
            </div>
            <div>
              <h4 class="text-[10px] text-surface-400 mb-1">cleanup_queue (最近 30 条)</h4>
              <div class="max-h-[200px] overflow-auto text-[8px] rounded border border-surface-200">
                <div v-for="r in debugData.cleanup_queue" :key="r.id" class="px-1.5 py-0.5 border-b border-surface-100 flex items-center gap-1"
                  :class="r.status===1?'text-emerald-600':r.status===-1?'text-red-500':'text-amber-600'">
                  <span class="font-mono">{{ r.id }}</span>
                  <span>{{ r.source_type }}/{{ r.source_id?.slice(0,8) }}...</span>
                  <span class="ml-auto">{{ r.status===1?'✓':r.status===-1?'✗':'⏳' }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
        <button @click="loadDebug" class="mt-2 px-2 py-1 text-[10px] rounded border border-surface-200 text-surface-500 hover:bg-surface-50">加载管道调试数据</button>
      </div>
    </div>
  </div>
</template>
<style scoped>
#kg-root .stat-card,
#kg-root .content-card { box-shadow: 0 1px 2px 0 rgba(0,0,0,0.05); transition-property: box-shadow, transform; transition-timing-function: ease; transition-duration: 200ms; }
#kg-root .stat-card:hover,
#kg-root .content-card:hover { transform: translateY(-2px); box-shadow: 0 4px 6px -1px rgba(0,0,0,0.1), 0 2px 4px -2px rgba(0,0,0,0.1); }
.dropdown-enter-active { transition: opacity 0.15s ease, transform 0.15s ease; }
.dropdown-leave-active { transition: opacity 0.1s ease, transform 0.1s ease; }
.dropdown-enter-from { opacity: 0; transform: translateY(-4px); }
.dropdown-leave-to { opacity: 0; transform: translateY(-4px); }
</style>
