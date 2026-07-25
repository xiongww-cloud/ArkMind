<script setup lang="ts">
import { computed } from 'vue'
import VueApexCharts from 'vue3-apexcharts'

const props = defineProps<{
  data: Array<{ label: string; cnt: number }>
}>()

const chartOptions = computed(() => ({
  chart: {
    type: 'area' as const,
    height: 180,
    toolbar: { show: false },
    sparkline: { enabled: false },
    animations: { enabled: false },
    parentHeightOffset: 0,
  },
  stroke: { curve: 'smooth' as const, width: 2 },
  fill: {
    type: 'gradient',
    gradient: { shadeIntensity: 1, opacityFrom: 0.25, opacityTo: 0.05, stops: [0, 100] },
  },
  colors: ['#3b82f6'],
  dataLabels: { enabled: false },
  grid: { borderColor: '#f1f5f9', strokeDashArray: 4, padding: { left: 4, right: 4 } },
  xaxis: {
    categories: props.data.map(d => d.label || ''),
    labels: {
      style: { fontSize: '10px', colors: '#94a3b8' },
      rotate: -45,
      rotateAlways: false,
    },
    tickAmount: Math.min(props.data.length, 12),
    axisBorder: { show: false },
    axisTicks: { show: false },
  },
  yaxis: {
    labels: { style: { fontSize: '10px', colors: '#94a3b8' }, min: 0 },
    min: 0,
  },
  tooltip: {
    enabled: true,
    theme: 'light',
    y: { formatter: (val: number) => `${val} 碎片` },
  },
}))

const series = computed(() => [{
  name: '累计碎片',
  data: props.data.map(d => d.cnt || 0),
}])
</script>

<template>
  <div class="-mt-4">
    <VueApexCharts
      v-if="data.length"
      :options="chartOptions"
      :series="series"
      type="area"
      height="180"
    />
    <p v-else class="text-[12px] text-surface-300 py-8 text-center">暂无增长数据</p>
  </div>
</template>
