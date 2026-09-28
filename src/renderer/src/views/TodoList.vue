<script setup lang="ts">
interface Todo { id: string; title: string; content: string; done: boolean; createdAt: string; dueDate?: string; priority?: 'P1'|'P2'|'P3'; projectId?: string; projectName?: string }

const PRIO = { color: { P1: '#ef4444', P2: '#f59e0b', P3: '#3b82f6' } as Record<string,string> }

defineProps<{ listWidth: number; selectedId: string | null; todos: Todo[] }>()
const emit = defineEmits<{ select: [id: string]; resizeStart: [e: MouseEvent]; toggleDone: [id: string] }>()

function fmtDate(d?: string): string {
  if (!d) return '无'
  try {
    const date = new Date(d); if (isNaN(date.getTime())) return '无'
    const today = new Date(); today.setHours(0,0,0,0)
    const tomorrow = new Date(today); tomorrow.setDate(tomorrow.getDate()+1)
    const prefix = date.getTime() === today.getTime() ? '今天' : date.getTime() === tomorrow.getTime() ? '明天' : ''
    const datePart = `${date.getFullYear()}/${date.getMonth()+1}/${date.getDate()}`
    const timePart = d.includes('T') ? ` ${String(date.getHours()).padStart(2,'0')}:${String(date.getMinutes()).padStart(2,'0')}` : ''
    const full = datePart + timePart
    return prefix ? `${prefix} · ${full}` : full
  } catch { return '无' }
}

function isOverdue(t: Todo): boolean {
  if (!t.dueDate || t.done) return false
  const d = new Date(t.dueDate); if (isNaN(d.getTime())) return false
  const today = new Date(); today.setHours(0,0,0,0)
  return d.getTime() < today.getTime()
}
</script>

<template>
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{ width: listWidth + 'px' }">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="emit('resizeStart', $event)"></div>
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">待办</span>
      <button class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" v-tooltip="'新建待办'">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>
      </button>
    </div>
    <div class="flex-1 overflow-y-auto p-2 space-y-1.5">
      <div
        v-for="todo in todos" :key="todo.id"
        class="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden"
        :class="[
          todo.done ? 'bg-surface-50/50 border-surface-100' : 'border-surface-100 hover:border-surface-300 hover:shadow-md',
          selectedId === todo.id ? 'shadow-lg scale-[1.02] z-10' : 'hover:shadow-lg hover:scale-[1.02] hover:z-10',
          selectedId === todo.id && todo.priority ? 'bg-blue-50/70' : selectedId === todo.id ? 'bg-gray-50' : ''
        ]"
        :style="selectedId === todo.id && todo.priority ? { borderColor: PRIO.color[todo.priority] } : selectedId === todo.id ? { borderColor: '#d1d5db' } : {}"
        @click="emit('select', todo.id)"
      >
        <div v-if="selectedId === todo.id" class="absolute left-0 top-0 bottom-0 w-[3px] rounded-r" :style="{ background: todo.priority ? PRIO.color[todo.priority] : '#9ca3af' }"></div>

        <button
          class="w-5 h-5 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all duration-200 hover:shadow-md"
          :style="todo.done ? { borderColor: todo.priority ? PRIO.color[todo.priority] : '#d1d5db', background: todo.priority ? PRIO.color[todo.priority] : '#9ca3af' } : { borderColor: todo.priority ? PRIO.color[todo.priority] : '#d1d5db' }"
          @click.stop="emit('toggleDone', todo.id)"
        >
          <svg v-if="todo.done" class="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
        </button>

        <div class="flex-1 min-w-0">
          <p class="text-[12px] font-medium truncate transition-colors" :class="todo.done ? 'text-surface-400 line-through' : 'text-surface-800'">{{ todo.title }}</p>
          <p class="text-[9px] text-surface-500 mt-1"><span class="text-surface-400">项目：</span>{{ todo.projectName || '无' }}</p>
          <p class="text-[9px]" :class="isOverdue(todo) ? 'text-red-500' : 'text-surface-500'" v-text="fmtDate(todo.dueDate)" />
        </div>
      </div>

      <div v-if="todos.length === 0" class="text-center py-8">
        <p class="text-xs text-surface-400">暂无待办事项</p>
      </div>
    </div>
  </div>
</template>
