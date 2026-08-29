<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import type { Slide } from '@/types/presentation'
import PresentationFrame from './PresentationFrame.vue'

const SLIDE_W = 1920
const SLIDE_H = 1080

const props = defineProps<{
  slides: Slide[]
  theme?: string
  startIndex?: number
  htmlContent?: string
}>()

const emit = defineEmits<{
  (e: 'close'): void
}>()

const current = ref(props.startIndex ?? 0)
const stageRef = ref<HTMLElement | null>(null)
const frameRef = ref<HTMLIFrameElement | null>(null)
const frameLoaded = ref(false)
const scale = ref(1)

const total = computed(() => {
  if (props.htmlContent) {
    const parser = new DOMParser()
    const doc = parser.parseFromString(props.htmlContent, 'text/html')
    return doc.querySelectorAll('.slide').length || 1
  }
  return props.slides.length
})

const isHtmlMode = computed(() => !!props.htmlContent)

function recalcScale() {
  const stage = stageRef.value
  if (!stage) return
  const w = stage.clientWidth
  const h = stage.clientHeight
  if (w <= 0 || h <= 0) return
  scale.value = Math.min(w / SLIDE_W, h / SLIDE_H, 1)
}

let ro: ResizeObserver | null = null

onMounted(() => {
  nextTick(() => {
    recalcScale()
    ro = new ResizeObserver(recalcScale)
    if (stageRef.value) ro.observe(stageRef.value)
  })
})
onUnmounted(() => {
  ro?.disconnect()
})

let navigateTimer: ReturnType<typeof setTimeout> | null = null
function navigateTo(index: number) {
  if (!frameLoaded.value) return
  if (navigateTimer) clearTimeout(navigateTimer)
  navigateTimer = setTimeout(() => {
    frameRef.value?.contentWindow?.postMessage({ type: 'NAVIGATE_TO', index }, '*')
  }, 50)
}

function next() {
  if (current.value < total.value - 1) {
    current.value++
    navigateTo(current.value)
  }
}
function prev() {
  if (current.value > 0) {
    current.value--
    navigateTo(current.value)
  }
}

function onFrameLoad() {
  frameLoaded.value = true
  navigateTo(current.value)
}

function onKey(e: KeyboardEvent) {
  switch (e.key) {
    case 'ArrowRight':
    case 'ArrowDown':
    case ' ':
    case 'PageDown':
      e.preventDefault()
      next()
      break
    case 'ArrowLeft':
    case 'ArrowUp':
    case 'PageUp':
      e.preventDefault()
      prev()
      break
    case 'Escape':
      e.preventDefault()
      emit('close')
      break
    case 'Home':
      current.value = 0
      navigateTo(0)
      break
    case 'End':
      current.value = total.value - 1
      navigateTo(current.value)
      break
  }
}

function onStageClick(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  const x = e.clientX - rect.left
  if (x < rect.width * 0.28) prev()
  else next()
}

watch(
  () => props.startIndex,
  (v) => {
    if (typeof v === 'number') {
      current.value = v
      navigateTo(v)
    }
  }
)

onMounted(() => {
  window.addEventListener('keydown', onKey, true)
  document.body.style.overflow = 'hidden'
  nextTick(() => stageRef.value?.focus())
})
onUnmounted(() => {
  window.removeEventListener('keydown', onKey, true)
  document.body.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition name="player-fade" appear>
      <div class="player-mask">
        <!-- 舞台 -->
        <div ref="stageRef" class="player-stage" tabindex="0" @click="onStageClick">
          <!-- HTML 模式：iframe 固定1920×1080 + scale缩放 -->
          <div v-if="isHtmlMode" class="player-frame player-frame-html">
            <iframe
              ref="frameRef"
              :srcdoc="htmlContent"
              class="player-iframe"
              :style="{ width: SLIDE_W + 'px', height: SLIDE_H + 'px', transform: `scale(${scale})`, transformOrigin: 'top left' }"
              @load="onFrameLoad"
            />
          </div>
          <!-- 旧模式：PresentationFrame 渲染 -->
          <div v-else class="player-frame">
            <PresentationFrame
              v-if="slides[current]"
              :slide="slides[current]"
              :theme="theme"
              :page="current + 1"
              :total="total"
            />
          </div>

          <!-- 翻页热区提示 -->
          <button
            class="player-nav left"
            :disabled="current === 0"
            title="上一页"
            @click.stop="prev"
          >
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
              <path d="M15 18l-6-6 6-6" />
            </svg>
          </button>
          <button
            class="player-nav right"
            :disabled="current >= total - 1"
            title="下一页"
            @click.stop="next"
          >
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
              <path d="M9 6l6 6-6 6" />
            </svg>
          </button>
        </div>

        <!-- 底部控制条 -->
        <div class="player-bar">
          <span class="player-count">{{ current + 1 }} / {{ total }}</span>
          <div class="player-progress">
            <div
              class="player-progress-fill"
              :style="{ width: `${((current + 1) / total) * 100}%` }"
            ></div>
          </div>
          <span class="player-hint">← → 翻页 · Esc 退出</span>
          <button class="player-close" title="退出播放 (Esc)" @click.stop="emit('close')">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.player-mask {
  position: fixed;
  inset: 0;
  z-index: 100;
  background: #000;
  display: flex;
  flex-direction: column;
}
.player-stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 28px 48px 12px;
  outline: none;
  cursor: pointer;
  min-height: 0;
}
.player-frame {
  width: min(100%, calc((100vh - 110px) * 16 / 9));
  box-shadow: 0 12px 60px rgba(0, 0, 0, 0.55);
  border-radius: 8px;
  overflow: hidden;
}
.player-frame-html {
  overflow: hidden;
}
.player-iframe {
  border: none;
  display: block;
}
.player-nav {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  width: 44px;
  height: 44px;
  border-radius: 50%;
  border: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255, 255, 255, 0.75);
  background: rgba(255, 255, 255, 0.08);
  transition: all 0.2s;
}
.player-nav:hover {
  background: rgba(255, 255, 255, 0.18);
  color: #fff;
}
.player-nav:disabled {
  opacity: 0.2;
  cursor: default;
}
.player-nav.left { left: 16px; }
.player-nav.right { right: 16px; }
.player-bar {
  height: 52px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 24px;
  background: rgba(0, 0, 0, 0.65);
  backdrop-filter: blur(12px);
  color: rgba(255, 255, 255, 0.7);
  font-size: 12px;
}
.player-count { min-width: 48px; text-align: center; font-variant-numeric: tabular-nums; }
.player-progress { flex: 1; height: 3px; background: rgba(255, 255, 255, 0.12); border-radius: 2px; }
.player-progress-fill { height: 100%; background: #3b82f6; border-radius: 2px; transition: width 0.3s ease; }
.player-hint { font-size: 11px; opacity: 0.45; }
.player-close {
  width: 32px; height: 32px;
  display: flex; align-items: center; justify-content: center;
  border: none; background: none; color: rgba(255, 255, 255, 0.5);
  cursor: pointer; border-radius: 6px;
}
.player-close:hover { background: rgba(255, 255, 255, 0.1); color: #fff; }
.player-fade-enter-active, .player-fade-leave-active { transition: opacity 0.25s ease; }
.player-fade-enter-from, .player-fade-leave-to { opacity: 0; }
</style>
