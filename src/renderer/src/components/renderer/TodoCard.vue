<template>
  <div class="todo-card" @click="handleClick">
    <div class="todo-header">
      <span class="todo-checkbox" :class="{ checked: done }" @click.stop="toggleDone">
        {{ done ? '✅' : '⬜' }}
      </span>
      <span class="todo-title" :class="{ done: done }">{{ title }}</span>
      <span class="todo-priority" :class="priorityClass">{{ priority }}</span>
    </div>
    <div class="todo-info">
      <div v-if="dueDate" class="todo-due">
        <span class="label">截止：</span>
        <span class="value">{{ formatDate(dueDate) }}</span>
      </div>
      <div v-if="content" class="todo-content">{{ truncate(content, 50) }}</div>
    </div>
    <div class="todo-footer">
      <span class="todo-id">ID: {{ id }}</span>
      <span class="todo-action">查看详情 →</span>
    </div>
  </div>
</template>

<script setup lang="ts">
const props = defineProps<{
  id: string
  title: string
  done?: boolean
  priority?: string
  dueDate?: string
  content?: string
}>()

const emit = defineEmits<{
  (e: 'click', id: string): void
  (e: 'toggle-done', id: string): void
}>()

const priorityClass = computed(() => {
  switch (props.priority) {
    case 'P1': return 'priority-high'
    case 'P2': return 'priority-medium'
    case 'P3': return 'priority-low'
    default: return 'priority-default'
  }
})

function formatDate(dateStr: string): string {
  const date = new Date(dateStr)
  return date.toLocaleDateString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit' })
}

function truncate(text: string, maxLen: number): string {
  return text.length > maxLen ? text.slice(0, maxLen) + '...' : text
}

function handleClick() {
  emit('click', props.id)
}

function toggleDone() {
  emit('toggle-done', props.id)
}
</script>

<style scoped>
.todo-card {
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  padding: 12px;
  margin-bottom: 8px;
  cursor: pointer;
  transition: all 0.2s;
  background: white;
}

.todo-card:hover {
  border-color: #3b82f6;
  box-shadow: 0 2px 8px rgba(59, 130, 246, 0.1);
}

.todo-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}

.todo-checkbox {
  font-size: 16px;
  cursor: pointer;
}

.todo-title {
  font-weight: 600;
  color: #1f2937;
  flex: 1;
}

.todo-title.done {
  text-decoration: line-through;
  color: #9ca3af;
}

.todo-priority {
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 12px;
  font-weight: 500;
}

.priority-high { background: #fee2e2; color: #991b1b; }
.priority-medium { background: #fef3c7; color: #92400e; }
.priority-low { background: #d1fae5; color: #065f46; }
.priority-default { background: #f3f4f6; color: #6b7280; }

.todo-info {
  font-size: 13px;
  color: #6b7280;
}

.todo-due {
  margin-bottom: 4px;
}

.todo-due .label {
  color: #9ca3af;
}

.todo-due .value {
  font-weight: 500;
  color: #374151;
}

.todo-content {
  color: #9ca3af;
  line-height: 1.4;
}

.todo-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid #f3f4f6;
  font-size: 12px;
  color: #9ca3af;
}

.todo-action {
  color: #3b82f6;
  font-weight: 500;
}
</style>
