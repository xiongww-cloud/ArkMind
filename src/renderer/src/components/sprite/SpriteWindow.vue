<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import MascotSprite from '@/components/common/MascotSprite.vue'

const mascotState = ref<'idle' | 'thinking' | 'speaking' | 'notifying'>('idle')

// ─── 点击/拖拽 区分 ────────────────────────────────────────
let pressed = false
let moved = false
let dragging = false
let startX = 0
let startY = 0
const MOVE_THRESHOLD = 8

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0 || dragging) return
  pressed = true
  moved = false
  startX = e.clientX
  startY = e.clientY
}

async function onPointerMove(e: PointerEvent) {
  if (!pressed || dragging) return
  const dx = e.clientX - startX
  const dy = e.clientY - startY
  if (Math.abs(dx) > MOVE_THRESHOLD || Math.abs(dy) > MOVE_THRESHOLD) {
    moved = true
    dragging = true
    pressed = false
    try {
      await getCurrentWebviewWindow().startDragging()
    } catch (err) {
      console.error('[Sprite] 拖拽失败:', err)
    }
    await new Promise(r => setTimeout(r, 50))
    dragging = false
  }
}

function onPointerUp() {
  if (!dragging && !moved) {
    // 浮窗尺寸/位置由 Rust 侧固定（顶部居中），无需再传宽高
    invoke('toggle_chat_window').catch(err => {
      console.error('[Sprite] 打开聊天窗口失败:', err)
    })
  }
  pressed = false
}

// ─── 位置持久化 ────────────────────────────────────────────
async function saveCurrentPosition() {
  try {
    const win = getCurrentWebviewWindow()
    const pos = await win.outerPosition()
    // outerPosition 返回物理像素，转换为逻辑像素
    const scaleFactor = await win.scaleFactor()
    const x = pos.x / scaleFactor
    const y = pos.y / scaleFactor
    await invoke('save_sprite_position', { x, y })
  } catch (err) {
    console.error('[Sprite] 保存位置失败:', err)
  }
}

async function restorePosition() {
  try {
    const pos = await invoke<{ x: number; y: number }>('get_sprite_position')
    const win = getCurrentWebviewWindow()
    await win.setPosition({ type: 'Logical', x: pos.x, y: pos.y })
  } catch (err) {
    console.error('[Sprite] 恢复位置失败:', err)
  }
}

// ─── Event Listeners ────────────────────────────────────────
let unlistenNotify: (() => void) | null = null
let unlistenAgentStatus: (() => void) | null = null
let unlistenAgentDone: (() => void) | null = null
let unlistenAgentError: (() => void) | null = null
let unlistenMoved: (() => void) | null = null

onMounted(async () => {
  await restorePosition()

  // 监听窗口移动事件，自动保存位置
  unlistenMoved = await getCurrentWebviewWindow().onMoved(async ({ payload: pos }) => {
    try {
      const win = getCurrentWebviewWindow()
      const scaleFactor = await win.scaleFactor()
      const x = pos.x / scaleFactor
      const y = pos.y / scaleFactor
      await invoke('save_sprite_position', { x, y })
    } catch (err) {
      console.error('[Sprite] 保存位置失败:', err)
    }
  })

  unlistenNotify = await listen<string>('notification:new', () => {
    mascotState.value = 'notifying'
  })
  unlistenAgentStatus = await listen<{ stage: string; tool?: string }>('agent:status', (ev) => {
    const stage = ev.payload.stage
    if (stage === 'thinking') mascotState.value = 'thinking'
    else if (stage === 'answer') mascotState.value = 'speaking'
    else if (stage === 'tool_start') mascotState.value = 'thinking'
    // ★ 委托生命周期：delegating=已路由到子 Agent（仍在工作）；delegate-done=交回主 Agent（继续思考，不回 idle）
    else if (stage === 'delegating' || stage === 'delegate-done') mascotState.value = 'thinking'
  })
  unlistenAgentDone = await listen<any>('agent:done', () => {
    mascotState.value = 'idle'
  })
  unlistenAgentError = await listen<any>('agent:error', () => {
    mascotState.value = 'idle'
  })
})

onUnmounted(() => {
  unlistenMoved?.()
  unlistenNotify?.()
  unlistenAgentStatus?.()
  unlistenAgentDone?.()
  unlistenAgentError?.()
})
</script>

<template>
<div class="sprite-root"
  @pointerdown="onPointerDown"
  @pointermove="onPointerMove"
  @pointerup="onPointerUp">
  <div class="sprite-content">
    <MascotSprite :state="mascotState" />
  </div>
</div>
</template>

<style>
.sprite-root {
  width: 48px !important;
  height: 74px !important;
  display: flex !important;
  align-items: center !important;
  justify-content: center !important;
  background: transparent !important;
  overflow: hidden !important;
  touch-action: none !important;
}
.sprite-content {
  cursor: pointer !important;
  transition: transform 0.2s !important;
}
.sprite-content:hover {
  transform: scale(1.1) !important;
}
.sprite-content:active {
  transform: scale(0.95) !important;
}
</style>
