<script setup lang="ts">
import { ref, computed, watch } from 'vue'

interface Todo {
  id: string; title: string; content: string; done: boolean; createdAt: string
  dueDate?: string; priority?: 'P1'|'P2'|'P3'; repeat?: string; projectId?: string; projectName?: string
}

const props = defineProps<{
  todos: Todo[]
  selectedTodoId: string | null
}>()

const emit = defineEmits<{
  selectDate: [date: string]
  selectTodo: [id: string]
  createTodo: [date: string]
  toggleDone: [id: string]
}>()

const WEEKDAYS = ['一', '二', '三', '四', '五', '六', '日']
const PRIORITY_COLOR: Record<string, string> = { P1: '#ef4444', P2: '#f59e0b', P3: '#3b82f6' }

const cursor = ref(new Date())
const selectedDate = ref(fmtKey(new Date()))

function fmtKey(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function prevMonth() { cursor.value = new Date(cursor.value.getFullYear(), cursor.value.getMonth() - 1, 1) }
function nextMonth() { cursor.value = new Date(cursor.value.getFullYear(), cursor.value.getMonth() + 1, 1) }
function goToday() { cursor.value = new Date(); selectedDate.value = fmtKey(new Date()) }

const monthLabel = computed(() => `${cursor.value.getFullYear()}年${cursor.value.getMonth() + 1}月`)

const todayKey = fmtKey(new Date())

const days = computed(() => {
  const y = cursor.value.getFullYear()
  const m = cursor.value.getMonth()
  const first = new Date(y, m, 1)
  const last = new Date(y, m + 1, 0)
  const startDay = (first.getDay() + 6) % 7 // Monday=0
  const result: { date: number; key: string; isCurrentMonth: boolean; isToday: boolean; isSelected: boolean; todos: Todo[] }[] = []

  for (let i = 0; i < startDay; i++) {
    const d = new Date(y, m, -(startDay - 1 - i))
    const key = fmtKey(d)
    result.push({ date: d.getDate(), key, isCurrentMonth: false, isToday: key === todayKey, isSelected: key === selectedDate.value, todos: [] })
  }

  const todosByDate = props.todos.reduce((acc, t) => {
    if (!t.dueDate) return acc
    const k = t.dueDate.slice(0, 10)
    if (!acc[k]) acc[k] = []
    acc[k].push(t)
    return acc
  }, {} as Record<string, Todo[]>)

  for (let i = 1; i <= last.getDate(); i++) {
    const key = `${y}-${String(m + 1).padStart(2, '0')}-${String(i).padStart(2, '0')}`
    result.push({
      date: i, key, isCurrentMonth: true,
      isToday: key === todayKey,
      isSelected: key === selectedDate.value,
      todos: todosByDate[key] || [],
    })
  }

  const remaining = 42 - result.length
  for (let i = 1; i <= remaining; i++) {
    const d = new Date(y, m + 1, i)
    const key = fmtKey(d)
    result.push({ date: d.getDate(), key, isCurrentMonth: false, isToday: key === todayKey, isSelected: key === selectedDate.value, todos: [] })
  }

  return result
})

const selectedDateTodos = computed(() => {
  return props.todos.filter(t => {
    if (!t.dueDate) return false
    return t.dueDate.slice(0, 10) === selectedDate.value
  }).sort((a, b) => {
    if (a.done !== b.done) return a.done ? 1 : -1
    const pa = a.priority === 'P1' ? 1 : a.priority === 'P2' ? 2 : a.priority === 'P3' ? 3 : 9
    const pb = b.priority === 'P1' ? 1 : b.priority === 'P2' ? 2 : b.priority === 'P3' ? 3 : 9
    return pa - pb
  })
})

function fmtDateLabel(dateKey: string): string {
  const d = new Date(dateKey + 'T00:00:00')
  const weekdays = ['周日', '周一', '周二', '周三', '周四', '周五', '周六']
  return `${d.getMonth() + 1}月${d.getDate()}日 ${weekdays[d.getDay()]}`
}
</script>

<template>
  <div class="flex flex-col h-full select-none">
    <!-- Month Navigation -->
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <div class="flex items-center gap-2">
        <span class="text-[12px] font-semibold text-surface-700">{{ monthLabel }}</span>
        <button @click="goToday" class="px-1.5 py-0.5 text-[9px] text-brand-600 hover:bg-brand-50 rounded transition-colors font-medium">今天</button>
      </div>
      <div class="flex items-center gap-0.5">
        <button @click="prevMonth" class="w-6 h-6 flex items-center justify-center rounded hover:bg-surface-100 text-surface-400 hover:text-surface-600 transition-colors">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M15.75 19.5L8.25 12l7.5-7.5"/></svg>
        </button>
        <button @click="nextMonth" class="w-6 h-6 flex items-center justify-center rounded hover:bg-surface-100 text-surface-400 hover:text-surface-600 transition-colors">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M8.25 4.5l7.5 7.5-7.5 7.5"/></svg>
        </button>
      </div>
    </div>

    <!-- Weekday Headers -->
    <div class="grid grid-cols-7 border-b border-surface-100">
      <div v-for="w in WEEKDAYS" :key="w" class="text-center text-[10px] text-surface-400 py-1.5 font-medium">{{ w }}</div>
    </div>

    <!-- Day Grid -->
    <div class="grid grid-cols-7">
      <div v-for="cell in days" :key="cell.key"
        class="relative flex flex-col items-center py-1.5 cursor-pointer transition-colors"
        :class="{
          'bg-brand-50': cell.isSelected && cell.isCurrentMonth,
          'hover:bg-surface-50': !cell.isSelected && cell.isCurrentMonth,
          'bg-surface-50/50': !cell.isCurrentMonth,
        }"
        @click="selectedDate = cell.key; emit('selectDate', cell.key)"
      >
        <span class="text-[11px] leading-6 w-6 h-6 flex items-center justify-center rounded-full"
          :class="{
            'bg-brand-600 text-white font-semibold': cell.isToday && cell.isSelected,
            'bg-brand-100 text-brand-700 font-medium': cell.isToday && !cell.isSelected,
            'text-surface-900 font-medium': cell.isCurrentMonth && !cell.isToday,
            'text-surface-300': !cell.isCurrentMonth,
            'bg-brand-50 ring-1 ring-brand-400': cell.isSelected && !cell.isToday && cell.isCurrentMonth,
          }"
        >{{ cell.date }}</span>
        <!-- Todo indicator dots -->
        <div v-if="cell.todos.length > 0" class="flex items-center gap-0.5 mt-0.5 h-2">
          <span v-for="t in cell.todos.slice(0, 4)" :key="t.id"
            class="w-1.5 h-1.5 rounded-full"
            :style="{ background: t.done ? '#d1d5db' : (t.priority ? PRIORITY_COLOR[t.priority] : '#94a3b8') }"
          />
          <span v-if="cell.todos.length > 4" class="text-[8px] text-surface-400 leading-none">+{{ cell.todos.length - 4 }}</span>
        </div>
      </div>
    </div>

    <!-- Selected Date Header + Todo List -->
    <div class="flex-1 overflow-y-auto border-t border-surface-100">
      <div class="flex items-center justify-between px-3 py-2 sticky top-0 bg-white/90 backdrop-blur-sm z-10 border-b border-surface-50">
        <span class="text-[11px] font-semibold text-surface-600">{{ fmtDateLabel(selectedDate) }}</span>
        <button @click="emit('createTodo', selectedDate)" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors" v-tooltip="'在此日期新建待办'">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>
        </button>
      </div>
      <div class="p-2 space-y-1">
        <div v-if="selectedDateTodos.length === 0" class="text-center py-6">
          <p class="text-[11px] text-surface-300">暂无待办</p>
        </div>
        <div v-for="todo in selectedDateTodos" :key="todo.id"
          class="flex items-center gap-2.5 px-2.5 py-2 rounded-lg cursor-pointer group transition-all border"
          :class="[
            selectedTodoId === todo.id ? 'border-brand-300 bg-brand-50/60 shadow-sm' : 'border-transparent hover:bg-surface-50',
            todo.done ? 'opacity-50' : ''
          ]"
          @click="emit('selectTodo', todo.id)"
        >
          <button class="w-4 h-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all"
            :style="{ borderColor: todo.done ? '#d1d5db' : (todo.priority ? PRIORITY_COLOR[todo.priority] : '#cbd5e1') }"
            :class="todo.done ? 'bg-surface-200' : ''"
            @click.stop="emit('toggleDone', todo.id)"
          >
            <svg v-if="todo.done" class="w-2.5 h-2.5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M4.5 12.75l6 6 9-13.5"/></svg>
          </button>
          <div class="flex-1 min-w-0">
            <p class="text-[11px] font-medium truncate" :class="todo.done ? 'text-surface-400 line-through' : 'text-surface-700'">{{ todo.title }}</p>
            <div class="flex items-center gap-1.5 mt-0.5">
              <span v-if="todo.projectName" class="text-[9px] text-surface-400">{{ todo.projectName }}</span>
              <span v-if="todo.repeat" class="inline-flex items-center px-1 py-0.5 rounded text-[8px] font-medium"
                :class="todo.repeat==='daily'?'bg-blue-50 text-blue-600':todo.repeat==='weekly'?'bg-purple-50 text-purple-600':'bg-green-50 text-green-600'">
                {{ todo.repeat==='daily'?'每天':todo.repeat==='weekly'?'每周':'每月' }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
