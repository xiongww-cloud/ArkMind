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
const inputAtBottom = ref(false)
const animating = ref(false)
const noAnimate = ref(false)
const inputText = ref('')
const chatListRef = ref<HTMLElement | null>(null)
const historyLoaded = ref(false)
const messages = ref<ChatMessage[]>([])

const speechTexts = ['准备聊点什么？', '有什么可以帮你的？', '想了解什么？', '问我点什么吧', '知识库就绪，尽管问！']
const currentSpeechText = ref(speechTexts[0])

// ─── 工具函数 ──────────────────────────────────────────────
const sleep = (ms: number) => new Promise(r => setTimeout(r, ms))

// ─── History ────────────────────────────────────────────────
const loadingMore = ref(false)
const hasMore = ref(true)

function mapHistoryMessage(m: any) {
  return {
    id: 'hist-' + m.id,
    dbId: m.id,
    role: m.role,
    content: m.content,
    timestamp: m.created_at ? new Date(m.created_at).getTime() : Date.now(),
  }
}

async function loadHistory() {
  try {
    await kbFabInitSession()
    const result = await kbFabLoadHistory(10)
    if (result.success && result.messages) {
      messages.value = result.messages.map(mapHistoryMessage)
      historyLoaded.value = true
      hasMore.value = result.messages.length >= 10
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

// 滚动到顶部时加载更早的消息
async function loadOlderMessages() {
  const el = chatListRef.value
  if (!el || loadingMore.value || !hasMore.value) return
  const oldestDbId = messages.value.find((m: any) => m.dbId)?.dbId
  if (!oldestDbId) { hasMore.value = false; return }
  loadingMore.value = true
  const prevHeight = el.scrollHeight
  try {
    const result = await kbFabLoadHistory(10, oldestDbId)
    if (result.success && result.messages && result.messages.length > 0) {
      const older = result.messages.map(mapHistoryMessage)
      messages.value.unshift(...older)
      for (const msg of older) {
        if (msg.role === 'assistant' && msg.content) {
          agent.renderMd(msg.content).then(h => { if (h && msg.id) agent.rendered.value[msg.id] = h })
        }
      }
      nextTick(() => { el.scrollTop = el.scrollHeight - prevHeight })
      hasMore.value = result.messages.length >= 10
    } else {
      hasMore.value = false
    }
  } catch (e) {
    console.error('[Chat] 加载更早消息失败:', e)
  }
  loadingMore.value = false
}

function onChatScroll() {
  const el = chatListRef.value
  if (!el || el.scrollTop > 100) return
  loadOlderMessages()
}

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

// ─── 展开/收起 ────────────────────────────────────────────
async function expandChat() {
  if (expanded.value || animating.value) return
  animating.value = true
  // 1. 输入框移到窗口底部 + 窗口瞬时上移（同步）
  inputAtBottom.value = true
  expanded.value = true
  invoke('expand_chat_window', { expanded: true }).catch(() => {})
  // 2. 等消息区展开完成
  if (!historyLoaded.value) await loadHistory()
  nextTick(() => scrollChat())
  await sleep(500)
  animating.value = false
}

async function collapseChat() {
  if (!expanded.value || animating.value) return
  animating.value = true
  expanded.value = false
  await sleep(500)   // 等消息区收起
  await sleep(300)  // 延迟 0.3 秒再下移窗口
  inputAtBottom.value = false
  invoke('expand_chat_window', { expanded: false }).catch(() => {})
  animating.value = false
}

// ─── 窗口控制 ──────────────────────────────────────────────
async function hideWindow() {
  // 先收起（窗口可见，DOM 立即更新），再隐藏，避免重开时以展开态渲染出现动画
  noAnimate.value = true
  expanded.value = false
  inputAtBottom.value = false
  animating.value = false
  await nextTick()
  noAnimate.value = false
  try {
    await invoke('toggle_chat_window')  // 隐藏而非销毁，保留输入文本
  } catch (err) {
    console.error('[Chat] 隐藏失败:', err)
  }
}

// ─── 发送 ───────────────────────────────────────────────────
async function send() {
  const q = inputText.value.trim()
  if (!q || agent.isChatting.value) return

  // 未展开则并行展开（不 await）
  if (!expanded.value) expandChat()

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

// ─── 拖拽（仅收起态可用）───────────────────────────────────
let pressed = false
let hasMoved = false
let startX = 0
let startY = 0
const MOVE_THRESHOLD = 8

function onPointerDown(e: PointerEvent) {
  if (expanded.value || animating.value) return
  if (e.button !== 0) return
  const target = e.target as HTMLElement
  if (target.closest('button') || target.closest('input') || target.closest('textarea')) return
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
    try { await getCurrentWebviewWindow().startDragging() } catch {}
  }
}

function onPointerUp() { pressed = false }

// ─── Lifecycle ──────────────────────────────────────────────
let unlistenReset: (() => void) | null = null

onMounted(async () => {
  await loadHistory()
  currentSpeechText.value = speechTexts[Math.floor(Math.random() * speechTexts.length)]

  unlistenReset = await listen('chat:reset', async () => {
    // 关闭/重新显示时禁用过渡，避免展开→收起动画（macOS 隐藏窗口时 transition 不推进，重新显示会播放）
    noAnimate.value = true
    expanded.value = false
    inputAtBottom.value = false
    animating.value = false
    await nextTick()
    noAnimate.value = false
  })
})

onUnmounted(() => {
  unlistenReset?.()
})
</script>

<template>
<!-- 全窗口透明覆盖层，点击空白处关闭（保留输入文本） -->
<div class="chat-overlay" @click.self="hideWindow">
  <!-- 聊天面板（消息区+输入栏合一，flex 布局） -->
  <div class="chat-body" :class="{ expanded: expanded, 'input-at-bottom': inputAtBottom, 'no-animate': noAnimate }">
    <!-- 消息区（展开时高度动画） -->
    <div class="chat-messages">
      <div ref="chatListRef" class="messages-scroll" @scroll="onChatScroll">
        <div v-if="loadingMore" class="text-center py-1">
          <span class="text-[10px] text-[#94a3b8]">加载中...</span>
        </div>
        <div v-else-if="!hasMore && messages.length > 0" class="text-center py-1">
          <span class="text-[10px] text-[#c7c7cc]">没有更早的消息了</span>
        </div>
        <div v-for="m in messages" :key="m.id" class="msg-row" :class="m.role === 'user' ? 'msg-row--user' : ''">
          <div v-if="m.role === 'user'" class="avatar avatar--user">我</div>
          <div v-else class="avatar avatar--ai">AI</div>
          <div class="msg-content" :class="m.role === 'user' ? 'max-w-[70%]' : 'max-w-[85%]'">
            <div class="msg-time" :class="m.role === 'user' ? 'text-right' : ''">{{ fmtTime(m.timestamp) }}</div>
            <div v-if="m.role === 'user'" class="bubble-user">
              <pre class="bubble-user__text">{{ m.content }}</pre>
            </div>
            <div v-else class="bubble-ai" :class="{ 'thinking-bubble': m.thinking }">
              <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
              <div v-else class="markdown-body chat-md" v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || m.content || agent.thinkingProgress.value || '思考中'"></div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 输入栏（始终可见，面板底部） -->
    <div class="chat-input-bar"
      @pointerdown="onPointerDown" @pointermove="onPointerMove" @pointerup="onPointerUp">
      <button @click.stop="expanded ? collapseChat() : expandChat()" class="icon-btn" :title="expanded ? '收起' : '展开'">
        <svg class="w-3.5 h-3.5 transition-transform" :class="{ 'rotate-180': expanded }" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6"/></svg>
      </button>
      <input v-model="inputText" :placeholder="expanded ? '向 AI 提问...' : currentSpeechText" class="chat-input"
        @keydown.enter.exact.prevent="send" />
      <button v-if="agent.isChatting.value" @click.stop="stopAgent" class="icon-btn icon-btn--stop" title="停止">
        <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
      </button>
      <button v-else @click.stop="send" :disabled="!inputText.trim()" class="send-btn" :class="{ 'send-btn--on': inputText.trim() }">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
      </button>
    </div>
  </div>
</div>
</template>

<style scoped>
/* ═══ 全窗口透明覆盖层 ═══ */
.chat-overlay {
  width: 100vw;
  height: 100vh;
  background: transparent;
}

/* ═══ 聊天面板（消息区+输入栏合一） ═══ */
.chat-body {
  position: absolute;
  left: 50%; transform: translateX(-50%);
  top: 20px;  /* 收起态：顶部 */
  width: 580px;
  display: flex; flex-direction: column;
  background: rgba(255,255,255,0.92);
  backdrop-filter: blur(20px);
  border-radius: 12px;
  box-shadow: 0 4px 24px rgba(0,0,0,0.15), 0 1px 4px rgba(0,0,0,0.08);
  overflow: hidden;
  user-select: none;
}
.chat-body.input-at-bottom {
  top: auto;
  bottom: 68px;
}
.chat-body.fading {
  opacity: 0;
}

/* ═══ 消息区（展开时高度动画） ═══ */
.chat-messages {
  height: 0;
  overflow: hidden;
  opacity: 0;
  transition: height 500ms cubic-bezier(0.33, 1, 0.68, 1),
              opacity 300ms ease 50ms;
}
.chat-body.expanded .chat-messages {
  height: 500px;
  opacity: 1;
}

/* 重置/重开时禁用过渡（避免收起动画） */
.chat-body.no-animate .chat-messages {
  transition: none !important;
}

.messages-scroll {
  height: 100%;
  overflow-y: auto;
  padding: 12px;
  user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(37, 99, 235, 0.2) transparent;
}
.messages-scroll::-webkit-scrollbar { width: 4px; }
.messages-scroll::-webkit-scrollbar-track { background: transparent; }
.messages-scroll::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

/* ═══ 输入栏（面板底部，flex 子元素） ═══ */
.chat-input-bar {
  flex: 0 0 auto;
  width: 580px; height: 48px;
  display: flex; align-items: center; gap: 4px;
  padding: 6px 8px;
  border-top: 1px solid rgba(0,0,0,0.04);
}

/* ═══ 按钮 ═══ */
.icon-btn {
  width: 28px; height: 28px; border-radius: 8px;
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0; border: none; cursor: pointer;
  background: none; color: #94a3b8; transition: all 0.15s;
}
.icon-btn:hover { color: #2563eb; background: rgba(37, 99, 235, 0.08); }
.icon-btn--stop { background: rgba(239, 68, 68, 0.9); color: white; }
.icon-btn--stop:hover { background: #ef4444; }

.chat-input {
  flex: 1; height: 28px; border: none; resize: none; outline: none;
  font-size: 12px; color: #3a3a3c; background: transparent; line-height: 28px; font-family: inherit;
}
.chat-input::placeholder { color: #94a3b8; }

.send-btn {
  width: 28px; height: 28px; border-radius: 8px;
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0; border: none; cursor: pointer;
  background: linear-gradient(135deg, #e2e8f0, #f1f5f9); color: #94a3b8; transition: all 0.2s;
}
.send-btn--on { background: linear-gradient(135deg, #06b6d4, #2563eb); color: white; box-shadow: 0 1px 8px rgba(37, 99, 235, 0.3); }
.send-btn--on:hover { box-shadow: 0 2px 12px rgba(37, 99, 235, 0.45); transform: scale(1.05); }

/* ═══ 消息 ═══ */
.msg-row { display: flex; gap: 8px; align-items: flex-start; margin-bottom: 12px; animation: fadeIn 0.25s ease-out; }
.msg-row--user { flex-direction: row-reverse; }

.avatar { width: 28px; height: 28px; border-radius: 50%; display: flex; align-items: center; justify-content: center; font-size: 10px; font-weight: 600; flex-shrink: 0; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
.avatar--user { background: #2563eb; color: white; }
.avatar--ai { background: linear-gradient(135deg, #2563eb, #93c5fd); color: white; }

.msg-content { min-width: 0; }
.msg-time { font-size: 9px; color: #c7c7cc; margin-bottom: 4px; padding: 0 2px; }

.bubble-user { background: #2563eb; color: white; border-radius: 16px 4px 16px 16px; padding: 6px 12px; }
.bubble-user__text { font-size: 12px; line-height: 1.6; white-space: pre-wrap; font-family: inherit; margin: 0; }
.bubble-ai { background: rgba(255,255,255,0.9); border: 1px solid #e5e5ea; border-radius: 4px 16px 16px 16px; padding: 6px 12px; box-shadow: 0 1px 4px rgba(0,0,0,0.04); }

/* ═══ Markdown ═══ */
.chat-md { font-size: 12px; line-height: 1.5; }
.chat-md :deep(p) { margin: 0.15em 0; }

/* ═══ 动画 ═══ */
@keyframes fadeIn { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
.thinking-bubble { position: relative; overflow: hidden; }
.thinking-bubble::after {
  content: ''; position: absolute; inset: 0;
  background: linear-gradient(90deg, transparent 30%, rgba(37,99,235,0.08) 50%, transparent 70%);
  background-size: 200% 100%; animation: shimmer 1.8s ease-in-out infinite; pointer-events: none; border-radius: inherit;
}
@keyframes shimmer { 0% { background-position: 200% center; } 100% { background-position: -200% center; } }
:deep(.thinking-spinner) {
  display: inline-block; width: 14px; height: 14px; vertical-align: middle; margin-right: 4px;
  border: 2px solid #e5e5ea; border-top-color: #2563eb; border-radius: 50%;
  animation: ts-spin 0.8s linear infinite;
}
@keyframes ts-spin { to { transform: rotate(360deg); } }
</style>
