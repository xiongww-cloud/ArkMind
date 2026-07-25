<template>
  <div class="project-card" @click="handleClick">
    <div class="project-header">
      <span class="project-icon">📋</span>
      <span class="project-name">{{ name }}</span>
      <span class="project-status" :class="statusClass">{{ status }}</span>
    </div>
    <div class="project-info">
      <div v-if="budget" class="project-budget">
        <span class="label">预算：</span>
        <span class="value">{{ formatBudget(budget) }}</span>
      </div>
      <div v-if="description" class="project-desc">{{ truncate(description, 50) }}</div>
    </div>
    <div class="project-footer">
      <span class="project-id">ID: {{ id }}</span>
      <span class="project-action">查看详情 →</span>
    </div>
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{
  id: string
  name: string
  status?: string
  budget?: number
  description?: string
}>()

const emit = defineEmits<{
  (e: 'click', id: string): void
}>()

const statusClass = computed(() => {
  switch (props.status) {
    case 'Won': return 'status-won'
    case 'Lost': return 'status-lost'
    case 'SolutionDesign': return 'status-design'
    case 'Ordered': return 'status-ordered'
    case 'Lead': return 'status-lead'
    default: return 'status-default'
  }
})

const statusText = computed(() => {
  switch (props.status) {
    case 'Won': return '已中标'
    case 'Lost': return '已失标'
    case 'SolutionDesign': return '方案设计'
    case 'Ordered': return '已下单'
    case 'Lead': return '线索'
    default: return props.status || '未知'
  }
})

function formatBudget(value: number): string {
  if (value >= 10000) return `${(value / 10000).toFixed(1)}亿`
  if (value >= 1) return `${value}万`
  return `${value}元`
}

function truncate(text: string, maxLen: number): string {
  return text.length > maxLen ? text.slice(0, maxLen) + '...' : text
}

function handleClick() {
  emit('click', props.id)
}
</script>

<style scoped>
.project-card {
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  padding: 12px;
  margin-bottom: 8px;
  cursor: pointer;
  transition: all 0.2s;
  background: white;
}

.project-card:hover {
  border-color: #3b82f6;
  box-shadow: 0 2px 8px rgba(59, 130, 246, 0.1);
}

.project-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.project-icon {
  font-size: 16px;
}

.project-name {
  font-weight: 600;
  color: #1f2937;
  flex: 1;
}

.project-status {
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 12px;
  font-weight: 500;
}

.status-won { background: #d1fae5; color: #065f46; }
.status-lost { background: #fee2e2; color: #991b1b; }
.status-design { background: #dbeafe; color: #1e40af; }
.status-ordered { background: #fef3c7; color: #92400e; }
.status-lead { background: #f3f4f6; color: #374151; }
.status-default { background: #f3f4f6; color: #6b7280; }

.project-info {
  font-size: 13px;
  color: #6b7280;
}

.project-budget {
  margin-bottom: 4px;
}

.project-budget .label {
  color: #9ca3af;
}

.project-budget .value {
  font-weight: 500;
  color: #374151;
}

.project-desc {
  color: #9ca3af;
  line-height: 1.4;
}

.project-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid #f3f4f6;
  font-size: 12px;
  color: #9ca3af;
}

.project-action {
  color: #3b82f6;
  font-weight: 500;
}
</style>
