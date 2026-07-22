<script setup lang="ts">
import { computed } from 'vue'
import VueApexCharts from 'vue3-apexcharts'

const props = defineProps<{
  labels: string[]
  newBudget: number[]
  wonBudget: number[]
}>()

const chartOptions = computed(() => ({
  chart: {
    type: 'line' as const,
    height: 220,
    toolbar: { show: false },
    animations: { enabled: false },
    parentHeightOffset: 0,
  },
  stroke: {
    curve: 'smooth' as const,
    width: [0, 2.5],
  },
  fill: {
    opacity: [0.85, 1],
  },
  colors: ['#3b82f6', '#10b981'],
  plotOptions: {
    bar: {
      columnWidth: '50%',
      borderRadius: 4,
      borderRadiusApplication: 'end' as const,
    },
  },
  xaxis: {
    categories: props.labels,
    labels: { style: { fontSize: '10px', colors: '#94a3b8' } },
    axisBorder: { show: false },
    axisTicks: { show: false },
  },
  yaxis: [
    {
      title: { text: '新增预算 (万)', style: { fontSize: '10px', color: '#94a3b8' } },
      labels: { style: { fontSize: '10px', colors: '#94a3b8' }, formatter: (v: number) => v >= 1000 ? `${(v / 1000).toFixed(0)}k` : `${v}` },
    },
    {
      opposite: true,
      title: { text: '中标金额 (万)', style: { fontSize: '10px', color: '#94a3b8' } },
      labels: { style: { fontSize: '10px', colors: '#94a3b8' }, formatter: (v: number) => v >= 1000 ? `${(v / 1000).toFixed(0)}k` : `${v}` },
    },
  ],
  grid: { borderColor: '#f1f5f9', strokeDashArray: 4, padding: { left: 4, right: 4 } },
  legend: {
    show: true,
    position: 'top' as const,
    horizontalAlign: 'right' as const,
    fontSize: '11px',
    markers: { width: 8, height: 8, radius: 2 },
    itemMargin: { horizontal: 6 },
  },
  tooltip: { enabled: true, theme: 'light', shared: true },
  dataLabels: { enabled: false },
}))

const series = computed(() => [
  { name: '新增预算', type: 'column', data: props.newBudget },
  { name: '中标金额', type: 'line', data: props.wonBudget },
])
</script>

<template>
  <VueApexCharts
    :options="chartOptions"
    :series="series"
    type="line"
    height="220"
  />
</template>
