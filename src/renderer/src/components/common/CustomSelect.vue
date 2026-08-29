<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'

interface Option {
  value: string
  label: string
}

const props = defineProps<{
  modelValue: string
  options: Option[]
  placeholder?: string
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
}>()

const open = ref(false)
const dropdownRef = ref<HTMLElement | null>(null)

function select(value: string) {
  emit('update:modelValue', value)
  open.value = false
}

function onClickOutside(e: MouseEvent) {
  if (dropdownRef.value && !dropdownRef.value.contains(e.target as Node)) {
    open.value = false
  }
}

onMounted(() => document.addEventListener('mousedown', onClickOutside))
onUnmounted(() => document.removeEventListener('mousedown', onClickOutside))
</script>

<template>
  <div ref="dropdownRef" class="custom-select">
    <button
      class="select-trigger"
      :class="{ active: open }"
      @click="open = !open"
    >
      <span class="select-label">{{ options.find((o) => o.value === modelValue)?.label || placeholder || '请选择' }}</span>
      <svg class="select-arrow" :class="{ rotated: open }" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path d="M19 9l-7 7-7-7" />
      </svg>
    </button>
    <Transition name="dropdown">
      <div v-if="open" class="select-dropdown">
        <button
          v-for="opt in options"
          :key="opt.value"
          class="select-option"
          :class="{ selected: opt.value === modelValue }"
          @click="select(opt.value)"
        >
          {{ opt.label }}
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.custom-select {
  position: relative;
}

.select-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: #fff;
  font-size: 11px;
  color: #374151;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
}

.select-trigger:hover {
  border-color: #d1d5db;
  background: #f9fafb;
}

.select-trigger.active {
  border-color: #2563eb;
  box-shadow: 0 0 0 2px rgba(37, 99, 235, 0.1);
}

.select-label {
  flex: 1;
  text-align: left;
}

.select-arrow {
  width: 12px;
  height: 12px;
  color: #9ca3af;
  transition: transform 0.15s;
  flex-shrink: 0;
}

.select-arrow.rotated {
  transform: rotate(180deg);
}

.select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  min-width: 120px;
  background: #fff;
  border: 1px solid #e5e7eb;
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  padding: 4px;
  z-index: 100;
}

.select-option {
  display: block;
  width: 100%;
  padding: 6px 10px;
  border: none;
  background: none;
  font-size: 12px;
  color: #374151;
  text-align: left;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.1s;
  white-space: nowrap;
}

.select-option:hover {
  background: #f3f4f6;
}

.select-option.selected {
  background: #eff6ff;
  color: #2563eb;
  font-weight: 500;
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
