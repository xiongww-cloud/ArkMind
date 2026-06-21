<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'

const props = defineProps<{ modelValue: string; repeatValue?: string }>()
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
  const iso = `${y}-${String(m + 1).padStart(2, '0')}-${String(selectedDay.value).padStart(2, '0')}T${String(selectedHour.value).padStart(2, '0')}:${String(selectedMinute.value).padStart(2, '0')}:00`
  emit('update:modelValue', iso)
  emit('close')
}

onMounted(() => { cursor.value = d(props.modelValue) })
</script>

<template>
  <div class="w-[230px] p-3 select-none">
    <div class="flex items-center justify-between mb-2">
      <span class="text-[12px] font-medium text-surface-700">{{ monthLabel }}</span>
      <div class="flex items-center gap-0.5">
        <button @click="prevMonth" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:bg-surface-100 hover:text-surface-600 transition-colors">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M15 19l-7-7 7-7"/></svg>
        </button>
        <button @click="nextMonth" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:bg-surface-100 hover:text-surface-600 transition-colors">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
        </button>
      </div>
    </div>

    <div class="grid grid-cols-7 gap-0.5 mb-1">
      <div v-for="w in WEEKDAYS" :key="w" class="text-center text-[10px] font-medium text-surface-400 py-1">{{ w }}</div>
    </div>

    <div class="grid grid-cols-7 gap-0.5 mb-2">
      <div
        v-for="cell in days" :key="cell.key"
        class="flex items-center justify-center h-7 text-[11px] rounded transition-colors"
        :class="{
          'text-surface-200': !cell.isCurrentMonth,
          'text-surface-700 hover:bg-surface-100 cursor-pointer': cell.isCurrentMonth && !cell.isToday && !(cell.day === selectedDay && cell.isCurrentMonth),
          'bg-brand-600 text-white font-medium shadow-sm': cell.day === selectedDay && cell.isCurrentMonth,
          'text-brand-600 font-semibold ring-1 ring-brand-300 rounded': cell.isToday && !(cell.day === selectedDay && cell.isCurrentMonth),
        }"
        @click="cell.isCurrentMonth && cell.day ? selectDay(cell.day) : null"
      >{{ cell.day || '' }}</div>
    </div>

    <div class="border-t border-surface-100 pt-2 space-y-2">
      <!-- Repeat -->
      <div class="flex items-center gap-1">
        <span class="text-[10px] font-medium text-surface-400 w-[28px] shrink-0">重复</span>
        <div class="flex gap-1 flex-1">
          <button v-for="ri in REPEAT_ITEMS" :key="ri.value" @click.stop="selectRepeat(ri.value)"
            class="flex-1 h-6 text-[10px] rounded transition-colors"
            :class="localRepeat === ri.value ? 'bg-brand-600 text-white font-medium shadow-sm' : 'text-surface-500 hover:bg-surface-100 border border-surface-200'"
          >{{ ri.label }}</button>
        </div>
      </div>
      <!-- Time -->
      <div class="relative">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-medium text-surface-400 w-[28px]">时间</span>
          <input
            :value="timeInput"
            @input="parseTimeInput(($event.target as HTMLInputElement).value)"
            @focus="timeOpen = true"
            @click.stop="timeOpen = true"
            class="w-[62px] border border-surface-200 rounded px-2 py-1 text-[11px] text-center font-mono outline-none focus:border-brand-400"
            placeholder="HH:MM"
            type="text"
            maxlength="5"
          />
          <button @click="confirmDateTime" :disabled="!selectedDay" class="flex-1 px-3 py-1 text-[10px] bg-brand-600 text-white rounded hover:bg-brand-700 disabled:opacity-30 disabled:cursor-not-allowed transition-colors">确认</button>
        </div>
        <div v-if="timeOpen" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 p-2 w-[180px]" @click.stop>
          <div class="flex gap-2">
            <div class="flex-1">
              <div class="text-[9px] font-medium text-surface-400 mb-1 text-center">时</div>
              <div class="max-h-[120px] overflow-y-auto space-y-0.5">
                <button v-for="h in HOURS" :key="h" @click="selectHour(h)" class="w-full py-0.5 rounded text-[10px] text-center transition-colors" :class="selectedHour === h ? 'bg-brand-600 text-white font-medium' : 'text-surface-600 hover:bg-surface-100'">{{ String(h).padStart(2,'0') }}</button>
              </div>
            </div>
            <div class="flex-1">
              <div class="text-[9px] font-medium text-surface-400 mb-1 text-center">分</div>
              <div class="max-h-[120px] overflow-y-auto space-y-0.5">
                <button v-for="mn in MINUTES" :key="mn" @click="selectMinute(mn)" class="w-full py-0.5 rounded text-[10px] text-center transition-colors" :class="selectedMinute === mn ? 'bg-brand-600 text-white font-medium' : 'text-surface-600 hover:bg-surface-100'">{{ String(mn).padStart(2,'0') }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
