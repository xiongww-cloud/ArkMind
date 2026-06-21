<script setup lang="ts">
import { ref, computed } from 'vue'

const props = defineProps({
  /** Full text content to display in the tooltip */
  text: { type: String, default: '' },
  /** Tooltip position relative to slot content: 'top' | 'bottom' */
  position: { type: String, default: 'top' },
  /** Horizontal alignment: 'center' | 'left' | 'right' */
  align: { type: String, default: 'center' },
  /** Render wrapper as block instead of inline-block */
  block: { type: Boolean, default: false },
})

const wrapperRef = ref(null)

const positionClass = computed(() => {
  return props.position === 'bottom'
    ? 'top-full mt-2'
    : 'bottom-full mb-2'
})

const alignClass = computed(() => {
  if (props.align === 'left') return 'left-0'
  if (props.align === 'right') return 'right-0'
  return 'left-1/2 -translate-x-1/2'
})

const arrowClass = computed(() => {
  const arrowPlacement = props.align === 'right' ? 'right-4' : props.align === 'left' ? 'left-4' : 'left-1/2 -translate-x-1/2'
  return props.position === 'bottom'
    ? `absolute -top-1 ${arrowPlacement} border-4 border-transparent border-b-slate-800`
    : `absolute -bottom-1 ${arrowPlacement} border-4 border-transparent border-t-slate-800`
})
</script>

<template>
  <div ref="wrapperRef" class="group/tip relative max-w-full" :class="block ? 'block' : 'inline-block'">
    <slot />
    <div
      v-if="text"
      class="absolute z-50 w-max max-w-[280px] opacity-0 invisible group-hover/tip:opacity-100 group-hover/tip:visible transition-opacity duration-200 pointer-events-none"
      :class="[positionClass, alignClass]"
    >
      <div class="bg-slate-800 text-white text-xs leading-relaxed rounded-md shadow-lg px-2.5 py-1.5 break-words">
        {{ text }}
      </div>
      <div :class="arrowClass" />
    </div>
  </div>
</template>
