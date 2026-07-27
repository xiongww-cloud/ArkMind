<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { kbFabInitSession, kbFabLoadHistory } from '@/api/db-client'
import { useAgentChat, type ChatMessage } from '@/composables/useAgentChat'

const agent = useAgentChat()

// ─── State ──────────────────────────────────────────────────
const expanded = ref(false)
const inputText = ref('')
const chatListRef = ref<HTMLElement | null>(null)
const historyLoaded = ref(false)
const messages = ref<ChatMessage[]>([])

const speechTexts = ['准备聊点什么？', '有什么可以帮你的？', '想了解什么？', '问我点什么吧', '知识库就绪，尽管问！']
const currentSpeechText = ref(speechTexts[0])

// ─── Window Resize ──────────────────────────────────────────
async function resizeWindow(shouldExpand: boolean) {
  try {
    await invoke('resize_chat_window_smooth', { expanded: shouldExpand })
  } catch (err) {
    console.error('[Chat] 调整窗口大小失败:', err)
  }
}

// ─── History ────────────────────────────────────────────────
async function loadHistory() {
  try {
    await kbFabInitSession()
    const result = await kbFabLoadHistory(50)
    if (result.success && result.messages) {
      messages.value = result.messages.map((m: any, i: number) => ({
        id: crypto.randomUUID(),
        role: m.role,
        content: m.content,
        timestamp: Date.now() - (result.messages!.length - i) * 1000,
      }))
      historyLoaded.value = true
      for (const msg of messages.value) {
        if (msg.role === 'assistant' && msg.content) {
          agent.renderMd(msg.content).then(h => { if (h && msg.id) agent.rendered.value[msg.id] = h })
        }
      }
    }
  } catch (e) {
    console.error('[Chat] 加载历史失败:', e)
  }
}

// ─── Helpers ────────────────────────────────────────────────
function scrollChat() {
  nextTick(() => {
    if (chatListRef.value) chatListRef.value.scrollTop = chatListRef.value.scrollHeight
  })
}

function fmtTime(ts?: number) {
  const d = ts ? new Date(ts) : new Date()
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today.getTime() - 86400000)
  const time = d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (d >= today) return `今天 ${time}`
  if (d >= yesterday) return `昨天 ${time}`
  return d.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

// ─── Expand/Collapse ────────────────────────────────────────
async function toggleExpand() {
  expanded.value = !expanded.value
  await resizeWindow(expanded.value)
  if (expanded.value) {
    scrollChat()
  }
}

// ─── Send ───────────────────────────────────────────────────
async function send() {
  const q = inputText.value.trim()
  if (!q || agent.isChatting.value) return

  // 发送消息时自动展开（不 await，让发送和展开并行）
  if (!expanded.value) {
    expanded.value = true
    resizeWindow(true).catch(() => {})
  }

  agent.stopAllTypewriters(messages.value)
  inputText.value = ''

  const userMsg: ChatMessage = { id: crypto.randomUUID(), role: 'user', content: q, timestamp: Date.now() }
  messages.value.push(userMsg)

  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true, toolCalls: [], rawToolCalls: [], toolResults: [], timestamp: Date.now() }
  messages.value.push(aiMsg)
  while (messages.value.length > 20) { messages.value.shift() }
  scrollChat()

  await agent.startAgentChat(q, {
    getMessage: () => aiMsg,
    scrollFn: scrollChat,
    sessionId: 'fab-main-session',
    onDone: (result, ctx) => {
      if (ctx.finalAnswer && ctx.finalAnswer.trim()) {
        aiMsg.toolCalls = result.tool_log || []
      }
    },
    onError: (error) => {
      aiMsg.content = error
    },
  })
}

async function stopAgent() {
  await agent.stopAgent(() => messages.value)
  scrollChat()
}

// ─── Window Controls ────────────────────────────────────────
async function closeWindow() {
  try {
    await invoke('close_chat_window')
  } catch (err) {
    console.error('[Chat] 关闭失败:', err)
  }
}

// ─── Drag ───────────────────────────────────────────────────
let pressed = false
let hasMoved = false
let startX = 0
let startY = 0
const MOVE_THRESHOLD = 8

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const target = e.target as HTMLElement
  if (target.closest('.chat-input') || target.closest('button')) return
  pressed = true
  hasMoved = false
  startX = e.clientX
  startY = e.clientY
  ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
}

async function onPointerMove(e: PointerEvent) {
  if (!pressed || hasMoved) return
  const dx = e.clientX - startX
  const dy = e.clientY - startY
  if (Math.abs(dx) > MOVE_THRESHOLD || Math.abs(dy) > MOVE_THRESHOLD) {
    hasMoved = true
    try {
      await getCurrentWebviewWindow().startDragging()
    } catch (err) {
      console.error('[Chat] 拖拽失败:', err)
    }
  }
}

function onPointerUp() {
  pressed = false
}

// ─── Lifecycle ──────────────────────────────────────────────
onMounted(async () => {
  await loadHistory()
  currentSpeechText.value = speechTexts[Math.floor(Math.random() * speechTexts.length)]

  await listen('chat:reset', async () => {
    if (expanded.value) {
      expanded.value = false
    }
    await resizeWindow(false)
  })
})
</script>

<template>
<div class="chat-window" :class="{ 'is-expanded': expanded }">
  <!-- ═══ Compact Mode: Input Only ═══ -->
  <div v-if="!expanded" class="compact-input-wrapper"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp">
    <button @click.stop="toggleExpand"
      class="compact-expand-btn" title="展开聊天">
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6"/></svg>
    </button>
    <textarea v-model="inputText" rows="1" :placeholder="currentSpeechText" class="compact-input"
      @keydown.enter.exact.prevent="send"
      @input="(e: any) => { if (!inputText.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 40) + 'px' }" />
    <button v-if="agent.isChatting.value" @click.stop="stopAgent"
      class="compact-send-btn compact-send-btn--stop" title="停止">
      <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
    </button>
    <button v-else @click.stop="send" :disabled="!inputText.trim()"
      class="compact-send-btn" :class="{ 'compact-send-btn--on': inputText.trim() }">
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
    </button>
  </div>

  <!-- ═══ Expanded Mode: Full Chat ═══ -->
  <template v-else>
    <!-- Messages -->
    <div ref="chatListRef" class="chat-messages">
      <div v-for="m in messages" :key="m.id" class="flex gap-2 items-start animate-fade-in mb-3"
        :class="m.role === 'user' ? 'flex-row-reverse' : ''">
        <div v-if="m.role === 'user'" class="w-7 h-7 rounded-full flex items-center justify-center text-[10px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
        <div v-else class="w-7 h-7 rounded-full flex items-center justify-center text-[10px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
        <div class="min-w-0" :class="m.role === 'user' ? 'max-w-[70%]' : 'max-w-[85%]'">
          <div class="text-[9px] text-[#c7c7cc] mb-1 px-0.5" :class="m.role === 'user' ? 'text-right' : ''">{{ fmtTime(m.timestamp) }}</div>
            <div v-if="m.role === 'user'" class="bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-1.5">
            <pre class="text-[12px] leading-relaxed whitespace-pre-wrap font-sans">{{ m.content }}</pre>
          </div>
          <div v-else class="chat-bubble-ai" :class="{ 'thinking-bubble': m.thinking }">
            <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
            <div v-else class="chat-md" v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || agent.thinkingProgress.value || m.content || '思考中'" />
          </div>
        </div>
      </div>
    </div>

    <!-- Input Bar (same layout as compact) -->
    <div class="chat-input-bar"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp">
      <button @click="toggleExpand"
        class="compact-expand-btn" title="收起聊天">
        <svg class="w-3.5 h-3.5 rotate-180" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6"/></svg>
      </button>
      <textarea v-model="inputText" rows="1" placeholder="向 AI 提问..." class="compact-input"
        @keydown.enter.exact.prevent="send"
        @input="(e: any) => { if (!inputText.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 40) + 'px' }" />
      <button v-if="agent.isChatting.value" @click.stop="stopAgent"
        class="compact-send-btn compact-send-btn--stop" title="停止">
        <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
      </button>
      <button v-else @click.stop="send" :disabled="!inputText.trim()"
        class="compact-send-btn" :class="{ 'compact-send-btn--on': inputText.trim() }">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
      </button>
    </div>
  </template>
</div>
</template>

<style scoped>
.chat-window {
  margin: auto;
  display: flex;
  flex-direction: column;
  width: calc(100% - 20px);
  background: rgba(255, 255, 255, 0.92);
  backdrop-filter: blur(20px);
  border-radius: 12px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25),
              0 2px 4px rgba(0, 0, 0, 0.15);
}
.chat-window.is-expanded {
  width: calc(100% - 12px);
  height: calc(100% - 12px);
  border-radius: 12px;
  overflow: hidden;
}

/* ═══ Compact Mode ═══ */
.compact-input-wrapper {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 6px;
  width: 100%;
  box-sizing: border-box;
  flex: 0 0 auto;
  height: 36px;
  margin: auto 0;
}
.compact-expand-btn {
  width: 24px;
  height: 24px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: #94a3b8;
  background: none;
  border: none;
  cursor: pointer;
  transition: all 0.15s;
}
.compact-expand-btn:hover {
  color: #2563eb;
  background: rgba(37, 99, 235, 0.08);
}
.compact-expand-btn svg {
  transition: transform 0.2s;
}
.compact-input {
  flex: 1;
  height: 24px;
  border: none;
  resize: none;
  outline: none;
  font-size: 11px;
  color: #3a3a3c;
  background: transparent;
  line-height: 24px;
  max-height: 24px;
  font-family: inherit;
}
.compact-input::placeholder {
  color: #94a3b8;
}
.compact-send-btn {
  width: 28px;
  height: 28px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: none;
  cursor: pointer;
  background: linear-gradient(135deg, #e2e8f0, #f1f5f9);
  color: #94a3b8;
  transition: all 0.2s;
}
.compact-send-btn--on {
  background: linear-gradient(135deg, #06b6d4, #2563eb);
  color: white;
  box-shadow: 0 1px 8px rgba(37, 99, 235, 0.3);
}
.compact-send-btn--on:hover {
  box-shadow: 0 2px 12px rgba(37, 99, 235, 0.45);
  transform: scale(1.05);
}
.compact-send-btn--stop {
  background: rgba(239, 68, 68, 0.9);
  color: white;
}
.compact-send-btn--stop:hover {
  background: #ef4444;
}

/* ─── Messages ─── */
.chat-messages {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(37, 99, 235, 0.2) transparent;
}
.chat-messages::-webkit-scrollbar { width: 4px; }
.chat-messages::-webkit-scrollbar-track { background: transparent; }
.chat-messages::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

/* ─── Input Bar ─── */
.chat-input-bar {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 6px;
  width: 100%;
  box-sizing: border-box;
  flex: 0 0 auto;
  height: 36px;
  border-top: 1px solid rgba(0, 0, 0, 0.04);
}

/* ─── AI Bubble ─── */
.chat-bubble-ai {
  background: rgba(255,255,255,0.9);
  border: 1px solid #e5e5ea;
  border-radius: 4px 16px 16px 16px;
  padding: 2px 12px;
  box-shadow: 0 1px 4px rgba(0,0,0,0.04);
}

/* ─── Markdown ─── */
.chat-md :deep(p) { margin: 0.3em 0; font-size: 11px; color: #3a3a3c; line-height: 1.65; }
.chat-md { font-size: 11px; }
.chat-md :deep(h1), .chat-md :deep(h2), .chat-md :deep(h3), .chat-md :deep(h4) { color: #1d1d1f; font-weight: 600; }
.chat-md :deep(h1) { font-size: 0.85rem; margin: 0.5em 0 0.25em; border-bottom: 1px solid #e5e5ea; padding-bottom: 0.2em; }
.chat-md :deep(h2) { font-size: 0.8rem; margin: 0.4em 0 0.2em; }
.chat-md :deep(h3) { font-size: 0.75rem; margin: 0.35em 0 0.15em; }
.chat-md :deep(h4) { font-size: 0.7rem; margin: 0.3em 0 0.1em; }
.chat-md :deep(ul), .chat-md :deep(ol) { padding-left: 1.2em; margin: 0.2em 0; }
.chat-md :deep(li) { font-size: 11px; color: #3a3a3c; margin: 0.1em 0; }
.chat-md :deep(strong) { color: #1d1d1f; font-weight: 600; }
.chat-md :deep(code) { background: rgba(0,0,0,0.06); color: #e11d48; padding: 1px 4px; border-radius: 3px; font-size: 0.85em; font-family: 'SF Mono', monospace; }
.chat-md :deep(pre) { background: #f8f9fa; color: #3a3a3c; padding: 8px 12px; border-radius: 6px; overflow-x: auto; margin: 0.5em 0; font-size: 11px; border: 1px solid #e5e5ea; }
.chat-md :deep(pre code) { background: none; color: inherit; padding: 0; }
.chat-md :deep(blockquote) { border-left: 2px solid #2563eb; padding: 6px 10px; margin: 0.5em 0; color: #4a4a4f; background: linear-gradient(135deg, rgba(37,99,235,0.04), rgba(99,102,241,0.02)); border-radius: 0 6px 6px 0; font-style: italic; }
.chat-md :deep(a) { color: #2563eb; text-decoration: none; }
.chat-md :deep(a:hover) { text-decoration: underline; }
.chat-md :deep(table) { width: 100%; border-collapse: separate; border-spacing: 0; border: 1px solid #e5e5ea; border-radius: 6px; overflow: hidden; margin: 0.5em 0; font-size: 11px; }
.chat-md :deep(th) { background: #f8f9fa; color: #1d1d1f; font-weight: 600; padding: 6px 8px; border-bottom: 1px solid #e5e5ea; text-align: left; }
.chat-md :deep(td) { padding: 6px 8px; border-bottom: 1px solid #f0f0f5; color: #3a3a3c; text-align: left; }
.chat-md :deep(tr:last-child td) { border-bottom: none; }
.chat-md :deep(tr:hover td) { background: #f8f9fa; }
.chat-md :deep(a:hover) { text-decoration: underline; }

/* ─── Animation ─── */
.animate-fade-in { animation: fadeIn 0.25s ease-out; }
@keyframes fadeIn { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
.thinking-bubble {
  position: relative;
  overflow: hidden;
}
.thinking-bubble::after {
  content: '';
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent 30%, rgba(37,99,235,0.08) 50%, transparent 70%);
  background-size: 200% 100%;
  animation: shimmer 1.8s ease-in-out infinite;
  pointer-events: none;
  border-radius: inherit;
}
@keyframes shimmer {
  0% { background-position: 200% center; }
  100% { background-position: -200% center; }
}
</style>
