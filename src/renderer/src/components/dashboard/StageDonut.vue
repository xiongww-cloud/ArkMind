<script setup lang="ts">
import { computed } from 'vue'
import VueApexCharts from 'vue3-apexcharts'

const props = defineProps<{
  data: Array<{ label: string; value: number; color: string }>
}>()

const chartOptions = computed(() => ({
  chart: {
    type: 'donut' as const,
    height: 220,
    toolbar: { show: false },
    animations: { enabled: false },
  },
  plotOptions: {
    pie: {
      donut: {
        size: '65%',
        labels: {
          show: true,
          name: { show: true, fontSize: '11px', color: '#64748b' },
          value: {
            show: true,
            fontSize: '13px',
            fontWeight: 'bold' as const,
            color: '#1e293b',
            formatter: (val: string) => `¥${Number(val).toLocaleString()}万`,
          },
          total: {
            show: true,
            label: '总预算',
            fontSize: '11px',
            color: '#94a3b8',
            formatter: () => {
              const total = props.data.reduce((s, d) => s + d.value, 0)
              return `¥${total.toLocaleString()}万`
            },
          },
        },
      },
    },
  },
  labels: props.data.map(d => d.label),
  colors: props.data.map(d => d.color),
  stroke: { width: 2, colors: ['#fff'] },
  legend: {
    show: true,
    position: 'bottom' as const,
    fontSize: '11px',
    colors: '#64748b',
    itemMargin: { horizontal: 8 },
    markers: { width: 8, height: 8, radius: 2 },
  },
  tooltip: {
    enabled: true,
    theme: 'light',
    y: { formatter: (val: number) => `¥${val.toLocaleString()}万` },
  },
  dataLabels: { enabled: false },
  states: { hover: { filter: { type: 'none' } } },
}))

const series = computed(() => props.data.map(d => d.value))
</script>

<template>
  <div class="flex items-center justify-center">
    <VueApexCharts
      v-if="data.length"
      :options="chartOptions"
      :series="series"
      type="donut"
      height="220"
      width="100%"
    />
    <p v-else class="text-[12px] text-surface-300 py-8 text-center">暂无数据</p>
  </div>
</template>
