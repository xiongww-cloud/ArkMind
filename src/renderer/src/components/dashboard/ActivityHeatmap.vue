<script setup lang="ts">
import { computed } from 'vue'
import VueApexCharts from 'vue3-apexcharts'

const props = defineProps<{
  heatmap: Array<{ date?: string; day?: string; count?: number; updates?: number; accesses?: number }>
}>()

const chartData = computed(() => {
  const items = props.heatmap || []
  return items.map((h) => ({
    x: (h.date || h.day || '').slice(-5),
    y: (h.count || h.updates || 0) + (h.accesses || 0),
  }))
})

const maxVal = computed(() => Math.max(...chartData.value.map(d => d.y), 1))

const chartOptions = computed(() => ({
  chart: {
    type: 'heatmap' as const,
    height: 130,
    toolbar: { show: false },
    animations: { enabled: false },
    parentHeightOffset: 0,
  },
  plotOptions: {
    heatmap: {
      shadeIntensity: 0.6,
      borderRadius: 3,
      colorScale: {
        ranges: [
          { from: 0, to: 0, color: '#f1f5f9', name: '无活动' },
          { from: 1, to: Math.ceil(maxVal.value * 0.33), color: '#bfdbfe', name: '低' },
          { from: Math.ceil(maxVal.value * 0.33) + 1, to: Math.ceil(maxVal.value * 0.66), color: '#60a5fa', name: '中' },
          { from: Math.ceil(maxVal.value * 0.66) + 1, to: maxVal.value * 10, color: '#2563eb', name: '高' },
        ],
      },
    },
  },
  dataLabels: { enabled: false },
  xaxis: {
    labels: { show: false },
    axisBorder: { show: false },
    axisTicks: { show: false },
  },
  yaxis: {
    labels: { show: false },
    reversed: true,
  },
  tooltip: {
    enabled: true,
    theme: 'light',
    y: { formatter: (val: number) => `${val} 次活动` },
  },
  grid: { show: false },
}))

const series = computed(() => {
  const data = chartData.value
  return [{ name: '活动量', data }]
})
</script>

<template>
  <VueApexCharts
    v-if="heatmap.length"
    :options="chartOptions"
    :series="series"
    type="heatmap"
    height="130"
  />
  <p v-else class="text-[12px] text-surface-300 py-4 text-center">暂无活动数据</p>
</template>
