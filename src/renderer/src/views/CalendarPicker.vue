<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'

const props = defineProps<{ modelValue: string; repeatValue?: string; showRepeat?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [v: string]; 'update:repeatValue': [v: string]; close: [] }>()

const WEEKDAYS = ['一', '二', '三', '四', '五', '六', '日']

function d(v: string) { const t = new Date(v); return isNaN(t.getTime()) ? new Date() : t }
const cursor = ref(d(props.modelValue))

const selectedHour = ref(cursor.value.getHours())
const selectedMinute = ref(cursor.value.getMinutes())
const timeOpen = ref(false)
const timeInput = ref(`${String(selectedHour.value).padStart(2, '0')}:${String(selectedMinute.value).padStart(2, '0')}`)

function prevMonth() { cursor.value.setMonth(cursor.value.getMonth() - 1); cursor.value = new Date(cursor.value) }
function nextMonth() { cursor.value.setMonth(cursor.value.getMonth() + 1); cursor.value = new Date(cursor.value) }

const monthLabel = computed(() => `${cursor.value.getFullYear()}年${cursor.value.getMonth() + 1}月`)

const days = computed(() => {
  const y = cursor.value.getFullYear()
  const m = cursor.value.getMonth()
  const first = new Date(y, m, 1)
  const last  = new Date(y, m + 1, 0)
  const start = first.getDay() || 7
  const result: { day: number; key: string; isCurrentMonth: boolean; isToday: boolean; isSelected: boolean }[] = []
  for (let i = 1; i < start; i++) result.push({ day: 0, key: `b${i}`, isCurrentMonth: false, isToday: false, isSelected: false })
  const today = new Date(); today.setHours(0,0,0,0)
  const sel = d(props.modelValue); sel.setHours(0,0,0,0)
  for (let i = 1; i <= last.getDate(); i++) {
    const date = new Date(y, m, i); date.setHours(0,0,0,0)
    result.push({
      day: i, key: `d${i}`, isCurrentMonth: true,
      isToday: date.getTime() === today.getTime(),
      isSelected: props.modelValue ? date.getTime() === sel.getTime() : false,
    })
  }
  return result
})

const selectedDay = ref(props.modelValue ? d(props.modelValue).getDate() : 0)

function selectDay(day: number) { if (!day) return; selectedDay.value = day }

function openTime() { timeOpen.value = !timeOpen.value }
function parseTimeInput(v: string) {
  timeInput.value = v
  const match = v.match(/^(\d{1,2}):(\d{2})$/)
  if (match) {
    let h = parseInt(match[1], 10)
    if (h < 0 || h > 23) h = 0
    selectedHour.value = h
    selectedMinute.value = parseInt(match[2], 10)
  }
}

const HOURS = Array.from({ length: 24 }, (_, i) => i)
const MINUTES = [0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]

function selectHour(h: number) { selectedHour.value = h; syncTimeInput() }
function selectMinute(m: number) { selectedMinute.value = m; syncTimeInput(); timeOpen.value = false }
function syncTimeInput() { timeInput.value = `${String(selectedHour.value).padStart(2, '0')}:${String(selectedMinute.value).padStart(2, '0')}` }

const REPEAT_ITEMS = [
  { value: '', label: '无' },
  { value: 'daily', label: '每天' },
  { value: 'weekly', label: '每周' },
  { value: 'monthly', label: '每月' },
]
const localRepeat = ref(props.repeatValue || '')
watch(() => props.repeatValue, (v) => { localRepeat.value = v || '' })
function selectRepeat(v: string) {
  localRepeat.value = v
  emit('update:repeatValue', v)
}

function confirmDateTime() {
  if (!selectedDay.value) return
  const y = cursor.value.getFullYear()
  const m = cursor.value.getMonth()
  // 空格分隔的 SQLite 原生友好格式（与 now_db 约定一致；datetime() 比较兼容）
  const val = `${y}-${String(m + 1).padStart(2, '0')}-${String(selectedDay.value).padStart(2, '0')} ${String(selectedHour.value).padStart(2, '0')}:${String(selectedMinute.value).padStart(2, '0')}:00`
  emit('update:modelValue', val)
  emit('close')
}

onMounted(() => { cursor.value = d(props.modelValue) })
</script>

<template>
  <div class="w-[260px] p-4 select-none bg-white/85 backdrop-blur-xl rounded-xl shadow-2xl border border-white/30">
    <div class="flex items-center justify-between mb-3">
      <span class="text-sm font-medium text-slate-700">{{ monthLabel }}</span>
      <div class="flex items-center gap-0.5">
        <button @click="prevMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
          <svg class="w-4 h-4 text-slate-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M15 19l-7-7 7-7"/></svg>
        </button>
        <button @click="nextMonth" class="p-1 rounded-md hover:bg-slate-100 transition-colors">
          <svg class="w-4 h-4 text-slate-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
        </button>
      </div>
    </div>

    <div class="grid grid-cols-7 mb-1">
      <div v-for="w in WEEKDAYS" :key="w" class="text-center text-[11px] text-slate-400 py-1">{{ w }}</div>
    </div>

    <div class="grid grid-cols-7 gap-0.5 mb-2">
      <div v-for="cell in days" :key="cell.key" class="aspect-square flex items-center justify-center">
        <button
          v-if="cell.day"
          @click="selectDay(cell.day)"
          class="w-8 h-8 rounded-full text-xs transition-colors"
          :class="{
            'bg-blue-600 text-white font-medium': cell.day === selectedDay && cell.isCurrentMonth,
            'text-blue-600 font-medium hover:bg-blue-50': cell.isToday && !(cell.day === selectedDay && cell.isCurrentMonth),
            'text-slate-600 hover:bg-slate-100': cell.isCurrentMonth && !cell.isToday && !(cell.day === selectedDay && cell.isCurrentMonth),
            'text-slate-300': !cell.isCurrentMonth,
          }"
        >{{ cell.day }}</button>
      </div>
    </div>

    <div class="mt-3 pt-3 border-t border-slate-100 space-y-2">
      <!-- Repeat -->
      <div v-if="showRepeat !== false" class="flex items-center gap-1">
        <span class="text-[11px] text-slate-400 w-[28px] shrink-0">重复</span>
        <div class="flex gap-1 flex-1">
          <button v-for="ri in REPEAT_ITEMS" :key="ri.value" @click.stop="selectRepeat(ri.value)"
            class="flex-1 h-6 text-[11px] rounded transition-colors"
            :class="localRepeat === ri.value ? 'bg-blue-600 text-white font-medium shadow-sm' : 'text-slate-500 hover:bg-slate-100 border border-slate-200'"
          >{{ ri.label }}</button>
        </div>
      </div>
      <!-- Time -->
      <div class="relative">
        <div class="flex items-center gap-2">
          <span class="text-[11px] text-slate-400 w-[28px]">时间</span>
          <input
            :value="timeInput"
            @input="parseTimeInput(($event.target as HTMLInputElement).value)"
            @focus="timeOpen = true"
            @click.stop="timeOpen = true"
            class="w-[62px] border border-slate-200 rounded px-2 py-1 text-[11px] text-center font-mono outline-none focus:border-blue-400"
            placeholder="HH:MM"
            type="text"
            maxlength="5"
          />
          <button @click="confirmDateTime" :disabled="!selectedDay" class="flex-1 px-3 py-1 text-[11px] bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-30 disabled:cursor-not-allowed transition-colors">确认</button>
        </div>
        <div v-if="timeOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-slate-100 p-2 w-[180px]" @click.stop>
          <div class="flex gap-2">
            <div class="flex-1">
              <div class="text-[10px] text-slate-400 mb-1 text-center">时</div>
              <div class="max-h-[120px] overflow-y-auto space-y-0.5">
                <button v-for="h in HOURS" :key="h" @click="selectHour(h)" class="w-full py-0.5 rounded text-[11px] text-center transition-colors" :class="selectedHour === h ? 'bg-blue-600 text-white font-medium' : 'text-slate-600 hover:bg-slate-100'">{{ String(h).padStart(2,'0') }}</button>
              </div>
            </div>
            <div class="flex-1">
              <div class="text-[10px] text-slate-400 mb-1 text-center">分</div>
              <div class="max-h-[120px] overflow-y-auto space-y-0.5">
                <button v-for="mn in MINUTES" :key="mn" @click="selectMinute(mn)" class="w-full py-0.5 rounded text-[11px] text-center transition-colors" :class="selectedMinute === mn ? 'bg-blue-600 text-white font-medium' : 'text-slate-600 hover:bg-slate-100'">{{ String(mn).padStart(2,'0') }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
