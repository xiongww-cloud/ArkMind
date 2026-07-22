<script setup lang="ts">
import { computed } from 'vue'
import { useProjectStore } from '@/stores/project'
import { STATUS_LABELS, STATUS_DOT_COLORS } from '@/composables/useProjectStatus'

const props = defineProps<{ id: string }>()

const store = useProjectStore()
const project = computed(() => store.getProjectById(props.id) as any)

const statusLabels = STATUS_LABELS
const statusColors = STATUS_DOT_COLORS
</script>

<template>
  <div v-if="project" class="flex-1 overflow-y-auto p-6">
    <div class="flex items-start gap-4 mb-6">
      <div class="w-2.5 h-2.5 rounded-full mt-2 flex-shrink-0" :class="statusColors[project.status] || 'bg-gray-400'" />
      <div class="flex-1 min-w-0">
        <h2 class="text-lg font-bold text-surface-900 mb-1">{{ project.name }}</h2>
        <span class="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-medium bg-surface-100 text-surface-600">
          {{ statusLabels[project.status] || project.status }}
        </span>
      </div>
    </div>

    <div class="grid grid-cols-2 gap-3 mb-6">
      <div class="bg-surface-50 rounded-xl p-3.5">
        <p class="text-[11px] text-surface-400 mb-0.5">客户</p>
        <p class="text-sm font-medium text-surface-700 truncate">—</p>
      </div>
      <div class="bg-surface-50 rounded-xl p-3.5">
        <p class="text-[11px] text-surface-400 mb-0.5">预算</p>
        <p class="text-sm font-medium text-surface-700">{{ project.budget ? '¥' + Number(project.budget).toLocaleString() : '—' }}</p>
      </div>
      <div class="bg-surface-50 rounded-xl p-3.5">
        <p class="text-[11px] text-surface-400 mb-0.5">投标时间</p>
        <p class="text-sm font-medium text-surface-700">{{ (project.bid_time || project.bidTime || '').slice(0, 10) || '—' }}</p>
      </div>
      <div class="bg-surface-50 rounded-xl p-3.5">
        <p class="text-[11px] text-surface-400 mb-0.5">创建时间</p>
        <p class="text-sm font-medium text-surface-700">{{ (project.created_at || '').slice(0, 10) || '—' }}</p>
      </div>
    </div>

    <div v-if="project.description" class="bg-surface-50 rounded-xl p-4">
      <p class="text-[11px] text-surface-400 mb-2">项目描述</p>
      <p class="text-sm text-surface-600 leading-relaxed">{{ project.description }}</p>
    </div>
  </div>
  <div v-else class="flex-1 flex items-center justify-center">
    <p class="text-sm text-surface-400">项目不存在或已被删除</p>
  </div>
</template>
