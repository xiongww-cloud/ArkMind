<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import AppIcon from './AppIcon.vue'

interface Option {
  id: string
  name: string
  address?: string
  [key: string]: any
}

const props = withDefaults(defineProps<{
  modelValue: string | string[]
  options: Option[]
  placeholder?: string
  searchPlaceholder?: string
  multiple?: boolean
  disabled?: boolean
}>(), {
  placeholder: '请选择',
  searchPlaceholder: '搜索...',
  multiple: false,
  disabled: false,
})

const emit = defineEmits<{
  (e: 'update:modelValue', value: string | string[]): void
}>()

const isOpen = ref(false)
const searchQuery = ref('')
const containerRef = ref<HTMLElement | null>(null)

const filteredOptions = computed(() => {
  if (!searchQuery.value.trim()) return props.options
  const q = searchQuery.value.toLowerCase()
  return props.options.filter(opt =>
    opt.name.toLowerCase().includes(q) ||
    (opt.address && opt.address.toLowerCase().includes(q))
  )
})

const selectedLabels = computed(() => {
  if (props.multiple) {
    const ids = props.modelValue as string[]
    return props.options.filter(opt => ids.includes(opt.id))
  }
  const opt = props.options.find(o => o.id === props.modelValue)
  return opt ? [opt] : []
})

function toggleOption(option: Option) {
  if (props.multiple) {
    const ids = [...(props.modelValue as string[])]
    const idx = ids.indexOf(option.id)
    if (idx >= 0) ids.splice(idx, 1)
    else ids.push(option.id)
    emit('update:modelValue', ids)
  } else {
    emit('update:modelValue', option.id === props.modelValue ? '' : option.id)
    isOpen.value = false
  }
  searchQuery.value = ''
}

function removeOption(option: Option) {
  if (!props.multiple) return
  const ids = (props.modelValue as string[]).filter(id => id !== option.id)
  emit('update:modelValue', ids)
}

function handleClickOutside(e: MouseEvent) {
  if (containerRef.value && !containerRef.value.contains(e.target as Node)) {
    isOpen.value = false
    searchQuery.value = ''
  }
}

onMounted(() => document.addEventListener('mousedown', handleClickOutside))
onUnmounted(() => document.removeEventListener('mousedown', handleClickOutside))
</script>

<template>
  <div ref="containerRef" class="relative">
    <!-- Trigger -->
    <div
      @click="!disabled && (isOpen = !isOpen)"
      class="flex items-center gap-1 min-h-[28px] px-2 py-1 border border-surface-200 rounded-lg bg-white cursor-pointer hover:border-surface-300 transition-colors"
      :class="{ 'opacity-50 cursor-not-allowed': disabled }"
    >
      <!-- Selected chips -->
      <div v-if="selectedLabels.length" class="flex flex-wrap gap-1 flex-1 min-w-0">
        <span
          v-for="opt in selectedLabels"
          :key="opt.id"
          class="inline-flex items-center gap-1 px-1.5 py-0.5 bg-brand-50 text-brand-700 rounded text-[10px]"
        >
          {{ opt.name }}
          <button
            v-if="multiple"
            @click.stop="removeOption(opt)"
            class="hover:text-brand-900"
          >
            <AppIcon name="x" class="w-2.5 h-2.5" />
          </button>
        </span>
      </div>
      <span v-else class="flex-1 min-w-0 text-[11px] text-surface-400">{{ placeholder }}</span>
      <AppIcon name="chevron-down" class="w-3 h-3 text-surface-400 flex-shrink-0" />
    </div>

    <!-- Dropdown -->
    <div
      v-show="isOpen"
      class="absolute top-full left-0 mt-1 w-full bg-white border border-surface-200 rounded-lg shadow-lg z-50 max-h-[200px] overflow-hidden flex flex-col"
    >
      <!-- Search -->
      <div class="p-2 border-b border-surface-100">
        <input
          v-model="searchQuery"
          type="text"
          class="w-full px-2 py-1 text-[11px] border border-surface-200 rounded bg-white outline-none focus:border-brand-400"
          :placeholder="searchPlaceholder"
          @click.stop
        />
      </div>
      <!-- Options -->
      <div class="overflow-y-auto flex-1">
        <div
          v-for="opt in filteredOptions"
          :key="opt.id"
          @click="toggleOption(opt)"
          class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer transition-colors"
          :class="{ 'bg-brand-50': (multiple ? (modelValue as string[]).includes(opt.id) : modelValue === opt.id) }"
        >
          <div
            v-if="multiple"
            class="w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0"
            :class="(modelValue as string[]).includes(opt.id) ? 'bg-brand-500 border-brand-500' : 'border-surface-300'"
          >
            <AppIcon v-if="(modelValue as string[]).includes(opt.id)" name="check" class="w-2.5 h-2.5 text-white" />
          </div>
          <div class="flex-1 min-w-0">
            <p class="text-[11px] text-surface-700 truncate">{{ opt.name }}</p>
            <p v-if="opt.address" class="text-[10px] text-surface-400 truncate">{{ opt.address }}</p>
          </div>
        </div>
        <div v-if="filteredOptions.length === 0" class="px-3 py-4 text-center text-[11px] text-surface-400">
          无匹配结果
        </div>
      </div>
    </div>
  </div>
</template>
