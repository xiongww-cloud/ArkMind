<script setup lang="ts">
import { useRouter } from 'vue-router'
import { useNavigation } from '@/composables/useNavigation'
import { useDashboardData } from '@/composables/useDashboardData'
import AppIcon from '@/components/common/AppIcon.vue'
import KpiCard from '@/components/dashboard/KpiCard.vue'
import PipelineFunnel from '@/components/dashboard/PipelineFunnel.vue'
import StageDonut from '@/components/dashboard/StageDonut.vue'
import TrendChart from '@/components/dashboard/TrendChart.vue'
import TopCustomers from '@/components/dashboard/TopCustomers.vue'
import KnowledgeGrowth from '@/components/dashboard/KnowledgeGrowth.vue'

const router = useRouter()
const { navigateTo } = useNavigation()

const {
  greeting, dateStr,
  activeProjectCount, pipelineTotalBudget,
  newThisMonthCount, newBudgetDelta,
  wonThisMonthCount, wonThisMonthBudget, wonBudgetDelta, winRate,
  expectedOrderBudget, expectedOrders,
  overdueTodos, urgentBids, urgentCount,
  pipelineStats, maxPipelineCount,
  stageDonutData,
  trendLabels, trendNewBudget, trendWonBudget,
  topCustomers, maxCustomerBudget,
  todayTodos, priorityDot, priorityStyle,
  deadlines,
  recentNotes,
  kbTotalChunks, kbByType, kbObjectTotal,
  kbHotCount, kbColdCount, kbHotRatio, kbColdRatio, kbAvgWeight, kbTagCount,
  kbDailyGrowth,
  bizOverview,
  formatBudget, goToProject,
} = useDashboardData()
</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-y-auto">
    <div class="p-6 space-y-5 max-w-[1200px] mx-auto">

      <!-- ══════════ Row 0: Header ══════════ -->
      <header>
        <h1 class="text-2xl font-bold tracking-tight text-surface-900">{{ greeting }} ☀️</h1>
        <p class="text-surface-400 text-[13px] mt-1">{{ dateStr }}</p>
        <div class="flex items-center gap-3 mt-3 flex-wrap">
          <button
            v-for="b in bizOverview" :key="b.label"
            @click="router.push(b.route)"
            class="inline-flex items-center gap-1.5 px-3 py-1.5 bg-surface-50 hover:bg-surface-100 rounded-full text-[11px] text-surface-500 hover:text-surface-700 transition-colors"
          >
            <span class="font-semibold text-surface-700">{{ b.count }}</span>
            <span>{{ b.label }}</span>
          </button>
        </div>
      </header>

      <!-- ══════════ Row 1: KPI Cards ══════════ -->
      <div class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
        <KpiCard
          icon="folder-open"
          icon-color="bg-brand-50 text-brand-600"
          label="活跃项目"
          :value="activeProjectCount"
          :sub="`本月新增 ${newThisMonthCount}`"
        />
        <KpiCard
          icon="bar-chart"
          icon-color="bg-emerald-50 text-emerald-600"
          label="管道金额"
          :value="formatBudget(pipelineTotalBudget) || '¥0'"
          :sub="`本月新增 ¥${formatBudget(newThisMonthBudget)}`"
          :delta="newBudgetDelta"
        />
        <KpiCard
          icon="sparkles"
          icon-color="bg-amber-50 text-amber-600"
          label="本月中标"
          :value="`${wonThisMonthCount}个`"
          :sub="`¥${formatBudget(wonThisMonthBudget)} · 胜率${winRate}%`"
          :delta="wonBudgetDelta"
        />
        <KpiCard
          icon="clock"
          icon-color="bg-indigo-50 text-indigo-600"
          label="预计下单 (30天)"
          :value="expectedOrders.length"
          :sub="`¥${formatBudget(expectedOrderBudget)}`"
        />
        <KpiCard
          icon="alert-circle"
          :icon-color="urgentCount > 0 ? 'bg-red-50 text-red-600' : 'bg-surface-50 text-surface-500'"
          label="紧急事项"
          :value="urgentCount"
          :sub="`逾期 ${overdueTodos.length} · 投标 ${urgentBids.length}`"
          :urgent="urgentCount > 0"
        />
      </div>

      <!-- ══════════ Row 2: Pipeline Funnel + Stage Donut ══════════ -->
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-4 items-stretch">
        <div class="lg:col-span-7 bg-white rounded-2xl border border-surface-200 p-4 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center justify-between mb-3">
            <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
              <AppIcon name="bar-chart" size="15" class="text-surface-400" />
              Pipeline 漏斗
            </h2>
            <button @click="router.push('/projects')" class="text-[11px] text-brand-600 hover:underline">查看全部</button>
          </div>
          <PipelineFunnel :stages="pipelineStats" :max-count="maxPipelineCount" />
        </div>
        <div class="lg:col-span-5 bg-white rounded-2xl border border-surface-200 p-4 shadow-sm hover:shadow-md transition-shadow">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5 mb-3">
            <AppIcon name="grid" size="15" class="text-surface-400" />
            阶段金额分布
          </h2>
          <StageDonut :data="stageDonutData" />
        </div>
      </div>

      <!-- ══════════ Row 3: Trend + Top Customers ══════════ -->
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-4">
        <div class="lg:col-span-7 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5 mb-4">
            <AppIcon name="sparkles" size="15" class="text-surface-400" />
            近 6 个月趋势
          </h2>
          <TrendChart :labels="trendLabels" :new-budget="trendNewBudget" :won-budget="trendWonBudget" />
        </div>
        <div class="lg:col-span-5 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5 mb-4">
            <AppIcon name="building" size="15" class="text-surface-400" />
            客户 Top5
          </h2>
          <TopCustomers :customers="topCustomers" :max-budget="maxCustomerBudget" />
        </div>
      </div>

      <!-- ══════════ Row 4: Todos + Deadlines + Notes ══════════ -->
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-4">
        <!-- Todos -->
        <div class="lg:col-span-4 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
              <AppIcon name="list" size="15" class="text-surface-400" />
              待办事项
            </h2>
            <button @click="router.push('/todos')" class="text-[11px] text-brand-600 hover:underline">全部</button>
          </div>
          <div v-if="todayTodos.length" class="space-y-1">
            <div v-for="t in todayTodos" :key="t.id"
              class="flex items-center gap-2 px-2.5 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
              @click="router.push('/todos')">
              <div class="w-2 h-2 rounded-full flex-shrink-0" :class="priorityDot[t.priority] || 'bg-surface-300'"></div>
              <span class="flex-1 text-[12px] text-surface-800 truncate">{{ t.title || '未命名' }}</span>
              <span v-if="t.priority" class="text-[10px] font-medium px-1.5 py-0.5 rounded flex-shrink-0" :class="priorityStyle[t.priority]">{{ t.priority }}</span>
              <span v-if="t.due_date" class="text-[10px] text-surface-400 flex-shrink-0">{{ t.due_date.slice(5, 10) }}</span>
            </div>
          </div>
          <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无待办</p>
        </div>

        <!-- Deadlines -->
        <div class="lg:col-span-4 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
              <AppIcon name="clock" size="15" class="text-surface-400" />
              关键截止
            </h2>
          </div>
          <div v-if="deadlines.length" class="space-y-1.5">
            <div v-for="d in deadlines" :key="d.id + d.type"
              class="flex items-center gap-2 px-2.5 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
              @click="goToProject(d.id)">
              <div class="w-1.5 h-1.5 rounded-full flex-shrink-0" :class="d.urgent ? 'bg-red-500 animate-pulse' : d.medium ? 'bg-amber-400' : 'bg-surface-300'"></div>
              <span class="flex-1 text-[12px] text-surface-700 truncate">{{ d.name }}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded flex-shrink-0"
                :class="d.type === '投标' ? 'bg-amber-50 text-amber-600' : 'bg-blue-50 text-blue-600'">
                {{ d.type }}
              </span>
              <span class="text-[11px] font-semibold flex-shrink-0" :class="d.urgent ? 'text-red-600' : d.medium ? 'text-amber-600' : 'text-surface-500'">
                {{ d.date }}
              </span>
            </div>
          </div>
          <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无临近截止</p>
        </div>

        <!-- Recent Notes -->
        <div class="lg:col-span-4 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
              <AppIcon name="file-doc" size="15" class="text-surface-400" />
              最近笔记
            </h2>
            <button @click="router.push('/notes')" class="text-[11px] text-brand-600 hover:underline">全部</button>
          </div>
          <div v-if="recentNotes.length" class="space-y-1">
            <div v-for="n in recentNotes" :key="n.id"
              class="px-2.5 py-2 rounded-xl hover:bg-surface-50 transition-colors cursor-pointer"
              @click="navigateTo('note', n.id)">
              <div class="flex items-center justify-between">
                <span class="text-[12px] text-surface-800 font-medium truncate">{{ n.title }}</span>
                <span class="text-[10px] text-surface-400 flex-shrink-0 ml-2">{{ n.updated }}</span>
              </div>
              <p v-if="n.preview" class="text-[11px] text-surface-400 mt-0.5 line-clamp-1">{{ n.preview }}</p>
            </div>
          </div>
          <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无笔记</p>
        </div>
      </div>

      <!-- ══════════ Row 5: Knowledge Base ══════════ -->
      <div class="grid grid-cols-1 lg:grid-cols-12 gap-4">
        <!-- KB Health -->
        <div class="lg:col-span-5 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5">
              <AppIcon name="sparkles" size="15" class="text-surface-400" />
              知识库概览
            </h2>
            <button @click="router.push('/knowledge')" class="text-[11px] text-brand-600 hover:underline">进入</button>
          </div>
          <div v-if="kbTotalChunks > 0" class="space-y-4">
            <!-- Two stat cards -->
            <div class="grid grid-cols-2 gap-3">
              <div class="bg-surface-50 rounded-xl p-3 text-center">
                <p class="text-[11px] text-surface-400 mb-1">知识碎片</p>
                <p class="text-[20px] font-bold text-surface-900">{{ kbTotalChunks.toLocaleString() }}</p>
              </div>
              <div class="bg-surface-50 rounded-xl p-3 text-center">
                <p class="text-[11px] text-surface-400 mb-1">关联对象</p>
                <p class="text-[20px] font-bold text-surface-900">{{ kbObjectTotal.toLocaleString() }}</p>
              </div>
            </div>

            <!-- Hot/Cold ratio bar -->
            <div>
              <div class="flex items-center justify-between mb-1.5">
                <span class="text-[11px] text-surface-500">冷热分布</span>
                <span class="text-[10px] text-surface-400">均权 {{ kbAvgWeight }}</span>
              </div>
              <div class="w-full h-2 bg-surface-100 rounded-full overflow-hidden flex">
                <div class="h-full bg-emerald-400 transition-all" :style="{ width: kbHotRatio + '%' }"></div>
                <div class="h-full bg-surface-300 transition-all" :style="{ width: kbColdRatio + '%' }"></div>
              </div>
              <div class="flex items-center justify-between mt-1">
                <span class="text-[10px] text-emerald-500">热门 {{ kbHotCount }}</span>
                <span class="text-[10px] text-surface-400">标签 {{ kbTagCount }}</span>
                <span class="text-[10px] text-surface-400">冷数据 {{ kbColdCount }}</span>
              </div>
            </div>

            <!-- By type with mini bars -->
            <div v-if="kbByType.length">
              <p class="text-[11px] text-surface-500 mb-2">碎片类型分布</p>
              <div class="space-y-2">
                <div v-for="t in kbByType" :key="t.type" class="group">
                  <div class="flex items-center justify-between mb-0.5">
                    <span class="text-[11px] text-surface-600">{{ t.label }}</span>
                    <span class="text-[10px] text-surface-400">{{ t.count }}</span>
                  </div>
                  <div class="w-full h-1.5 bg-surface-100 rounded-full overflow-hidden">
                    <div class="h-full bg-brand-400 rounded-full transition-all group-hover:bg-brand-500"
                      :style="{ width: Math.max((t.count / kbByType[0].count) * 100, 4) + '%' }"></div>
                  </div>
                </div>
              </div>
            </div>
          </div>
          <p v-else class="text-[12px] text-surface-300 py-6 text-center">暂无数据</p>
        </div>

        <!-- Activity Heatmap -->
        <div class="lg:col-span-7 bg-white rounded-2xl border border-surface-200 p-5 shadow-sm hover:shadow-md transition-shadow">
          <h2 class="text-[13px] font-semibold text-surface-700 flex items-center gap-1.5 mb-4">
            <AppIcon name="sparkles" size="15" class="text-surface-400" />
            知识增长趋势
          </h2>
          <KnowledgeGrowth :data="kbDailyGrowth" />
        </div>
      </div>

    </div>
  </div>
</template>
