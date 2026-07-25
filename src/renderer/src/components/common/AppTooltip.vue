<script setup lang="ts">
import { ref, computed, nextTick, watch, onBeforeUnmount } from 'vue'

const props = defineProps({
  /** Full text content to display in the tooltip */
  text: { type: String, default: '' },
  /** Tooltip position relative to slot content: 'top' | 'bottom' */
  position: { type: String, default: 'top' },
  /** Render wrapper as block instead of inline-block */
  block: { type: Boolean, default: false },
  /** Only show tooltip when content is actually truncated */
  auto: { type: Boolean, default: false },
  /** Show arrow indicator */
  arrow: { type: Boolean, default: false },
  /** Max width of tooltip */
  maxWidth: { type: String, default: '280px' },
  /** Use whitespace-nowrap (for single-line truncated text) */
  nowrap: { type: Boolean, default: false },
})

const visible = ref(false)
const tipX = ref(0)
const tipY = ref(0)
const placement = ref<'top' | 'bottom'>('top')
const triggerRef = ref<HTMLElement | null>(null)
const tooltipRef = ref<HTMLElement | null>(null)
const isTruncated = ref(false)
const scrollCleanupFns: Array<() => void> = []

// --- Truncation detection (auto mode) ---
function checkTruncation() {
  if (!props.auto || !triggerRef.value) return
  const el = triggerRef.value
  isTruncated.value = el.scrollWidth > el.clientWidth || el.scrollHeight > el.clientHeight
}

const showCondition = computed(() => {
  if (!props.text) return false
  if (props.auto) return isTruncated.value
  return true
})

// --- Position calculation ---
function calcPosition() {
  if (!triggerRef.value || !showCondition.value) return
  const rect = triggerRef.value.getBoundingClientRect()
  const gap = 8

  // Wait for tooltip DOM to measure height
  const tipEl = tooltipRef.value
  const tipH = tipEl ? tipEl.offsetHeight : 30
  const tipW = tipEl ? tipEl.offsetWidth : 100

  // Determine vertical placement
  let top: number
  let place: 'top' | 'bottom'

  if (props.position === 'bottom') {
    place = 'bottom'
    top = rect.bottom + gap
  } else {
    // Default: try top first, flip if not enough space
    const topSpace = rect.top - gap
    if (topSpace >= tipH) {
      place = 'top'
      top = rect.top - tipH - gap
    } else {
      place = 'bottom'
      top = rect.bottom + gap
    }
  }

  // Horizontal: center on trigger
  let left = rect.left + rect.width / 2 - tipW / 2

  // Boundary clamping
  const vpPad = 8
  if (left < vpPad) left = vpPad
  if (left + tipW > window.innerWidth - vpPad) left = window.innerWidth - tipW - vpPad
  if (left < vpPad) left = vpPad // in case tooltip is wider than viewport

  placement.value = place
  tipX.value = left
  tipY.value = top
}

// --- Scroll close ---
function collectScrollAncestors(el: HTMLElement): HTMLElement[] {
  const ancestors: HTMLElement[] = []
  let cur = el.parentElement
  while (cur && cur !== document.body) {
    const style = getComputedStyle(cur)
    if (style.overflow === 'auto' || style.overflow === 'scroll' ||
        style.overflowY === 'auto' || style.overflowY === 'scroll') {
      ancestors.push(cur)
    }
    cur = cur.parentElement
  }
  return ancestors
}

function addScrollListeners() {
  if (!triggerRef.value) return
  const ancestors = collectScrollAncestors(triggerRef.value)
  for (const a of ancestors) {
    const handler = () => hideTooltip()
    a.addEventListener('scroll', handler, { passive: true })
    scrollCleanupFns.push(() => a.removeEventListener('scroll', handler))
  }
}

function clearScrollListeners() {
  scrollCleanupFns.forEach(fn => fn())
  scrollCleanupFns.length = 0
}

// --- Show / Hide ---
function showTooltip() {
  if (!showCondition.value) return
  visible.value = true
}

function hideTooltip() {
  visible.value = false
  clearScrollListeners()
}

// Recalculate position after tooltip DOM is ready (including Teleport)
watch(visible, (val) => {
  if (val) {
    setTimeout(() => {
      calcPosition()
      addScrollListeners()
    }, 30)
  }
})

// --- Trigger ref bridge ---
function setTriggerRef(el: any) {
  const node = el?.$el || el
  triggerRef.value = node as HTMLElement | null
  if (props.auto) nextTick(checkTruncation)
}

// --- Lifecycle ---
onBeforeUnmount(() => {
  hideTooltip()
})
</script>

<template>
  <!-- Trigger area -->
  <div class="inline-block max-w-full" :class="block ? 'block' : ''">
    <div ref="setTriggerRef"
         @mouseenter="showTooltip"
         @mouseleave="hideTooltip">
      <slot />
    </div>
  </div>

  <!-- Tooltip (Teleported to body) -->
  <Teleport to="body">
    <div v-if="visible && text"
         ref="tooltipRef"
         class="fixed z-[9999] pointer-events-none"
         :style="{ left: tipX + 'px', top: tipY + 'px' }">
      <div class="bg-gray-800 text-white text-[11px] leading-relaxed rounded shadow-lg px-2 py-1"
           :class="nowrap ? 'whitespace-nowrap' : 'break-words'"
           :style="{ maxWidth: maxWidth }">
        {{ text }}
      </div>
      <div v-if="arrow"
           class="absolute border-4 border-transparent"
           :class="placement === 'bottom'
             ? 'border-t-gray-800 -top-1 left-1/2 -translate-x-1/2'
             : 'border-b-gray-800 -bottom-1 left-1/2 -translate-x-1/2'" />
    </div>
  </Teleport>
</template>
