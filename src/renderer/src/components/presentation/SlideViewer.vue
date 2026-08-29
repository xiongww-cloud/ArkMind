<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue'

const SLIDE_W = 1920
const SLIDE_H = 1080

const props = defineProps<{
  srcdoc: string
  pageIndex: number
}>()

const emit = defineEmits<{
  (e: 'element-clicked', info: ElementInfo): void
  (e: 'slide-changed', index: number, total: number): void
  (e: 'slides-info', total: number, titles: string[]): void
}>()

export interface ElementInfo {
  tagName: string
  id: string
  className: string
  text: string
  rect: DOMRect
  outerHTML: string
}

const frameRef = ref<HTMLIFrameElement | null>(null)
const viewportRef = ref<HTMLElement | null>(null)
const frameLoaded = ref(false)
const scale = ref(1)

function recalcScale() {
  const vp = viewportRef.value
  if (!vp) return
  const w = vp.clientWidth
  const h = vp.clientHeight
  if (w <= 0 || h <= 0) return
  scale.value = Math.min(w / SLIDE_W, h / SLIDE_H)
}

let ro: ResizeObserver | null = null

onMounted(() => {
  window.addEventListener('message', onMessage)
  nextTick(() => {
    recalcScale()
    ro = new ResizeObserver(recalcScale)
    if (viewportRef.value) ro.observe(viewportRef.value)
  })
})
onUnmounted(() => {
  window.removeEventListener('message', onMessage)
  ro?.disconnect()
})

function onFrameLoad() {
  frameLoaded.value = true
  recalcScale()
  frameRef.value?.contentWindow?.postMessage({ type: 'GET_SLIDES_INFO' }, '*')
  navigateTo(props.pageIndex)
}

function navigateTo(index: number) {
  if (!frameLoaded.value) return
  frameRef.value?.contentWindow?.postMessage({ type: 'NAVIGATE_TO', index }, '*')
}

function onMessage(e: MessageEvent) {
  const data = e.data
  if (!data || typeof data !== 'object') return
  if (e.source !== frameRef.value?.contentWindow) return
  switch (data.type) {
    case 'ELEMENT_CLICKED':
      emit('element-clicked', {
        tagName: data.tagName,
        id: data.id,
        className: data.className,
        text: data.text,
        rect: data.rect,
        outerHTML: data.outerHTML
      })
      break
    case 'SLIDE_CHANGED':
      emit('slide-changed', data.index, data.total)
      break
    case 'SLIDES_INFO':
      emit('slides-info', data.total, data.titles)
      break
  }
}

watch(() => props.pageIndex, (idx) => {
  navigateTo(idx)
})

defineExpose({ navigateTo })
</script>

<template>
  <div ref="viewportRef" class="slide-viewport">
    <iframe
      ref="frameRef"
      :srcdoc="srcdoc"
      class="slide-frame"
      sandbox="allow-scripts allow-same-origin"
      scrolling="no"
      :style="{ width: SLIDE_W + 'px', height: SLIDE_H + 'px', transform: `scale(${scale})`, transformOrigin: 'top left' }"
      @load="onFrameLoad"
    />
  </div>
</template>

<style scoped>
.slide-viewport {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  overflow: hidden;
  border-radius: 8px;
  background: #f3f4f6;
}
.slide-frame {
  border: none;
  display: block;
}
</style>
