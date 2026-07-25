<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { notificationStats } from '@/api/notification-client'
import { kbFabInitSession, kbFabLoadHistory, kbFabSaveMessage } from '@/api/db-client'
import MascotSprite from '@/components/common/MascotSprite.vue'
import { useAgentChat, type ChatMessage } from '@/composables/useAgentChat'

const agent = useAgentChat()

// ─── State ──────────────────────────────────────────────────
const open = ref(false)
const expanded = ref(false)
const inputText = ref('')
const chatListRef = ref<HTMLElement | null>(null)
const historyLoaded = ref(false)

// ─── Messages ───────────────────────────────────────────────
const messages = ref<ChatMessage[]>([])

// ─── Notifications ──────────────────────────────────────────
const unreadCount = ref(0)

// ─── Mascot State ────────────────────────────────────────────
const mascotState = ref<'idle' | 'thinking' | 'speaking' | 'notifying'>('idle')

async function loadNotificationStats() {
  try {
    const s = await notificationStats()
    unreadCount.value = s.unread_count || 0
    if (unreadCount.value > 0 && !open.value) {
      mascotState.value = 'notifying'
    }
  } catch {}
}

// ─── FAB Session History ──────────────────────────────────────
async function loadFabHistory() {
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
      // 加载历史后渲染 markdown
      for (const msg of messages.value) {
        if (msg.role === 'assistant' && msg.content) {
          agent.renderMd(msg.content).then(h => { if (h && msg.id) agent.rendered.value[msg.id] = h })
        }
      }
    }
  } catch (e) {
    console.error('[FAB] 加载历史失败:', e)
  }
}

async function saveFabMessage(msg: ChatMessage) {
  try {
    await kbFabSaveMessage({
      role: msg.role,
      content: msg.content || '',
    })
  } catch (e) {
    console.error('[FAB] 保存消息失败:', e)
  }
}

// ─── Position ─────────────────────────────────────────────────
const popoverSide = computed(() => 'right')

const speechTexts = ['准备聊点什么？', '有什么可以帮你的？', '想了解什么？', '问我点什么吧', '知识库就绪，尽管问！']
const currentSpeechText = ref(speechTexts[0])

function scrollChat() { nextTick(() => { if (chatListRef.value) chatListRef.value.scrollTop = chatListRef.value.scrollHeight }) }
function fmtTime() {
  const d = new Date()
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today.getTime() - 86400000)
  const time = d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (d >= today) return `今天 ${time}`
  if (d >= yesterday) return `昨天 ${time}`
  return d.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

// ─── Send ───────────────────────────────────────────────────
async function send() {
  const q = inputText.value.trim()
  if (!q || agent.isChatting.value) return
  expanded.value = true
  agent.stopAllTypewriters(messages.value)
  inputText.value = ''
  nextTick(() => { const el = document.querySelector('.fab-input') as HTMLTextAreaElement; if (el) el.style.height = '' })

  const userMsg: ChatMessage = { id: crypto.randomUUID(), role: 'user', content: q, timestamp: Date.now() }
  messages.value.push(userMsg)
  // 保存用户消息
  saveFabMessage(userMsg)

  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true }
  messages.value.push(aiMsg)
  while (messages.value.length > 20) { messages.value.shift() }
  scrollChat()
  mascotState.value = 'thinking'

  await agent.startAgentChat(q, {
    getMessage: () => aiMsg,
    scrollFn: scrollChat,
    getHistory: () => buildFabHistory(),
    sessionId: 'fab-main-session',
    onStartAnswer: () => { mascotState.value = 'speaking' },
    onFinish: () => {
      mascotState.value = 'idle'
      // 保存 AI 回复
      saveFabMessage(aiMsg)
    },
  })
}

function buildFabHistory() {
  // 构建历史：排除最后一条（当前问题），传递给后端用于上下文
  return messages.value.slice(0, -1).map(m => ({
    role: m.role,
    content: m.content || '',
  }))
}

async function stopAgent() {
  await agent.stopAgent(() => messages.value)
  scrollChat()
  mascotState.value = 'idle'
}

function toggle() {
  if (open.value) {
    open.value = false
    expanded.value = false
    agent.stopAllTypewriters(messages.value)
  } else {
    open.value = true
    expanded.value = false
    currentSpeechText.value = speechTexts[Math.floor(Math.random() * speechTexts.length)]
  }
  mascotState.value = 'idle'
}

function closeAll() {
  open.value = false
  expanded.value = false
  agent.stopAllTypewriters(messages.value)
  mascotState.value = 'idle'
}

async function expandChat() {
  if (expanded.value) {
    expanded.value = false
    agent.stopAllTypewriters(messages.value)
    return
  }
  expanded.value = true
  if (!historyLoaded.value) {
    await loadFabHistory()
  }
  scrollChat()
}

// ─── Lifecycle ──────────────────────────────────────────────
const unlistenNotify = ref<(() => void) | null>(null)
const unlistenAgentStatus = ref<(() => void) | null>(null)

function onDocMouseDown(e: MouseEvent) {
  if (!open.value || expanded.value) return
  if (!(e.target as HTMLElement).closest('.fab-root')) {
    closeAll()
  }
}

onMounted(async () => {
  document.addEventListener('mousedown', onDocMouseDown)
  loadFabHistory()
  loadNotificationStats()
  unlistenNotify.value = await listen<string>('notification:new', async () => {
    unreadCount.value++
    const s = await notificationStats().catch(() => null)
    if (s) unreadCount.value = s.unread_count
    if (!open.value && !expanded.value) {
      mascotState.value = 'notifying'
    }
  })
  unlistenAgentStatus.value = await listen<{ stage: string; tool?: string }>('agent:status', (ev) => {
    if (agent.isChatting.value) return
    const stage = ev.payload.stage
    if (stage === 'thinking') mascotState.value = 'thinking'
    else if (stage === 'answer') mascotState.value = 'speaking'
    else if (stage === 'tool_start') {
      mascotState.value = 'thinking'
      agent.setThinkingProgress(agent.pickToolStatus(ev.payload.tool || ''))
    }
  })
  await listen<any>('agent:done', () => {
    if (!agent.isChatting.value) mascotState.value = 'idle'
  })
  await listen<any>('agent:error', () => {
    if (!agent.isChatting.value) mascotState.value = 'idle'
  })
})

onUnmounted(() => {
  document.removeEventListener('mousedown', onDocMouseDown)
  agent.stopAllTypewriters(messages.value)
  unlistenNotify.value?.()
  unlistenAgentStatus.value?.()
})

</script>

<template>
<div class="fab-root">
  <!-- Floating Button -->
  <button @click="toggle" class="fab-button" :class="{ 'fab-open': open }" title="AI 助手">
    <MascotSprite :state="mascotState" />
  </button>

  <!-- ═══ Chat Container (compact + expanded) ═══ -->
  <Transition name="fab-chat">
    <div v-if="open" class="fab-chat" :class="['fab-chat--' + popoverSide, { 'fab-chat--expanded': expanded }]">
      <!-- Floating close button (expanded only) -->
      <button v-if="expanded" @click="closeAll"
        class="absolute top-2 right-2 z-10 w-6 h-6 rounded-md flex items-center justify-center text-[#94a3b8] hover:text-[#ef4444] hover:bg-red-50 transition-colors" title="关闭">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M6 18L18 6M6 6l12 12"/></svg>
      </button>

      <!-- Messages (expanded only) -->
      <div v-if="expanded" ref="chatListRef" class="fab-chat__body">
          <div v-for="m in messages" :key="m.id" class="flex gap-2 items-start animate-fade-in mb-2.5"
            :class="m.role === 'user' ? 'flex-row-reverse' : ''">
            <div v-if="m.role === 'user'" class="w-6 h-6 rounded-full flex items-center justify-center text-[9px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
            <div v-else class="w-6 h-6 rounded-full flex items-center justify-center text-[9px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
            <div class="min-w-0" :class="m.role === 'user' ? 'max-w-[70%]' : 'max-w-[80%]'">
              <div class="text-[9px] text-[#c7c7cc] mb-0.5 px-0.5" :class="m.role === 'user' ? 'text-right' : ''">{{ fmtTime() }}</div>
              <div v-if="m.role === 'user'" class="bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-1.5">
                <pre class="text-[11px] leading-relaxed whitespace-pre-wrap font-sans">{{ m.content }}</pre>
              </div>
              <div v-else class="fab-bubble-ai" :class="{ 'thinking-bubble': m.thinking }">
                <div v-if="m.error" class="text-[10px] text-red-500">{{ m.error }}</div>
                <div v-else class="fab-md" v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || agent.thinkingProgress.value || m.content || '思考中'" />
              </div>
            </div>
          </div>
        </div>

      <!-- Input (always visible) -->
      <div class="fab-chat__input">
        <button @click="expandChat"
          class="w-6 h-6 rounded-md flex items-center justify-center flex-shrink-0 text-[#94a3b8] hover:text-[#2563eb] hover:bg-blue-50 transition-colors" :title="expanded ? '收起聊天记录' : '查看聊天记录'">
          <svg class="w-3.5 h-3.5 transition-transform" :class="{ 'rotate-180': expanded }" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6"/></svg>
        </button>
        <textarea v-model="inputText" rows="1" :placeholder="expanded ? '向 AI 提问...' : currentSpeechText" class="fab-input"
          @keydown.enter.exact.prevent="send"
          @input="(e: any) => { if (!inputText.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 40) + 'px' }" />
        <button v-if="agent.isChatting.value" @click="stopAgent"
          class="w-7 h-7 rounded-full bg-red-500 hover:bg-red-600 text-white flex items-center justify-center transition-colors flex-shrink-0" title="停止">
          <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
        </button>
        <button v-else @click="send" :disabled="!inputText.trim()"
          class="fab-send" :class="{ 'fab-send--on': inputText.trim() }">
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
        </button>
      </div>
    </div>
  </Transition>
</div>
</template>

<style scoped>
/* ═══ Orb ═══ */
.fab-root { position: fixed; bottom: 20px; left: -2px; z-index: 9999; user-select: none; }
.fab-root:has(.fab-chat) { cursor: default; }

.fab-button {
  position: relative; width: 70px; height: 70px; border-radius: 50%;
  background: transparent;
  display: flex; align-items: center; justify-content: center;
  border: none; cursor: pointer;
  transition: transform 0.2s;
}
.fab-button:hover { transform: scale(1.1); }
.fab-button.fab-open { animation: none; transform: scale(0.85); opacity: 0.4; pointer-events: auto; }

/* ═══ Chat Container ═══ */
.fab-chat {
  position: absolute; bottom: 15px;
  width: 380px;
  background: rgba(255,255,255,0.92);
  backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px);
  border: 1px solid rgba(0,0,0,0.06);
  border-radius: 16px;
  box-shadow: 0 4px 20px rgba(0,0,0,0.12), 0 1px 4px rgba(0,0,0,0.06);
  z-index: 9998;
  overflow: hidden;
  transition: height 0.3s cubic-bezier(0.16,1,0.3,1), width 0.3s cubic-bezier(0.16,1,0.3,1);
}
/* 小箭头指向精灵 */
.fab-chat--left::after {
  content: '';
  position: absolute;
  right: -5px; top: 50%; transform: translateY(-50%);
  border-top: 5px solid transparent;
  border-bottom: 5px solid transparent;
  border-left: 5px solid rgba(255,255,255,0.92);
}
.fab-chat--right::after {
  content: '';
  position: absolute;
  left: -5px; top: 50%; transform: translateY(-50%);
  border-top: 5px solid transparent;
  border-bottom: 5px solid transparent;
  border-right: 5px solid rgba(255,255,255,0.92);
}
.fab-chat--left { right: calc(100% + 10px); }
.fab-chat--right { left: calc(100% + 10px); }

.fab-chat--expanded {
  width: 440px;
  height: calc(60vh + 50px);
  display: flex; flex-direction: column;
}

.fab-chat__body {
  flex: 1; min-height: 0; overflow-y: auto; padding: 10px;
  scrollbar-width: thin; scrollbar-color: rgba(37,99,235,0.2) transparent;
}
.fab-chat__body::-webkit-scrollbar { width: 3px; }
.fab-chat__body::-webkit-scrollbar-track { background: transparent; }
.fab-chat__body::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

.fab-chat__input {
  display: flex; align-items: center; gap: 4px;
  padding: 6px 10px;
  border-top: 1px solid rgba(0,0,0,0.04);
}

/* Chat transition */
.fab-chat-enter-active { transition: all 0.5s cubic-bezier(0.16,1,0.3,1); }
.fab-chat-leave-active { transition: all 0.25s ease; }
.fab-chat-enter-from { opacity: 0; transform: scale(0.92); }
.fab-chat-leave-to { opacity: 0; transform: scale(0.95); }

.fab-input { flex: 1; border: none; resize: none; outline: none; font-size: 11px; color: #3a3a3c; background: transparent; line-height: 1.5; max-height: 40px; font-family: inherit; }
.fab-input::placeholder { color: #94a3b8; }

.fab-send { width: 28px; height: 28px; border-radius: 8px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; border: none; cursor: pointer; background: linear-gradient(135deg, #e2e8f0, #f1f5f9); color: #94a3b8; transition: all 0.2s; }
.fab-send--on { background: linear-gradient(135deg, #06b6d4, #2563eb); color: white; box-shadow: 0 1px 8px rgba(37,99,235,0.3); }
.fab-send--on:hover { box-shadow: 0 2px 12px rgba(37,99,235,0.45); transform: scale(1.05); }

/* ═══ AI Bubble ═══ */
.fab-bubble-ai {
  background: rgba(255,255,255,0.9);
  border: 1px solid #e5e5ea;
  border-radius: 4px 16px 16px 16px;
  padding: 2px 12px;
  box-shadow: 0 1px 4px rgba(0,0,0,0.04);
}

/* ═══ Markdown ═══ */
.fab-md :deep(p) { margin: 0.3em 0; font-size: 11px; color: #3a3a3c; line-height: 1.65; }
.fab-md { font-size: 11px; }
.fab-md :deep(h1), .fab-md :deep(h2), .fab-md :deep(h3), .fab-md :deep(h4) { color: #1d1d1f; font-weight: 600; }
.fab-md :deep(h1) { font-size: 0.85rem; margin: 0.5em 0 0.25em; border-bottom: 1px solid #e5e5ea; padding-bottom: 0.2em; }
.fab-md :deep(h2) { font-size: 0.8rem; margin: 0.4em 0 0.2em; }
.fab-md :deep(h3) { font-size: 0.75rem; margin: 0.35em 0 0.15em; }
.fab-md :deep(h4) { font-size: 0.7rem; margin: 0.3em 0 0.1em; }
.fab-md :deep(ul), .fab-md :deep(ol) { padding-left: 1.2em; margin: 0.2em 0; }
.fab-md :deep(li) { font-size: 11px; color: #3a3a3c; margin: 0.1em 0; }
.fab-md :deep(strong) { color: #1d1d1f; font-weight: 600; }
.fab-md :deep(code) { background: rgba(0,0,0,0.06); color: #e11d48; padding: 1px 4px; border-radius: 3px; font-size: 0.85em; font-family: 'SF Mono', monospace; }
.fab-md :deep(pre) { background: #f8f9fa; color: #3a3a3c; padding: 8px 12px; border-radius: 6px; overflow-x: auto; margin: 0.5em 0; font-size: 11px; border: 1px solid #e5e5ea; }
.fab-md :deep(pre code) { background: none; color: inherit; padding: 0; }
.fab-md :deep(blockquote) { border-left: 2px solid #2563eb; padding: 6px 10px; margin: 0.5em 0; color: #4a4a4f; background: linear-gradient(135deg, rgba(37,99,235,0.04), rgba(99,102,241,0.02)); border-radius: 0 6px 6px 0; font-style: italic; }
.fab-md :deep(a) { color: #2563eb; text-decoration: none; }
.fab-md :deep(a:hover) { text-decoration: underline; }
.fab-md :deep(table) { width: 100%; border-collapse: separate; border-spacing: 0; border: 1px solid #e5e5ea; border-radius: 6px; overflow: hidden; margin: 0.5em 0; font-size: 11px; }
.fab-md :deep(th) { background: #f8f9fa; color: #1d1d1f; font-weight: 600; padding: 6px 8px; border-bottom: 1px solid #e5e5ea; text-align: left; }
.fab-md :deep(td) { padding: 6px 8px; border-bottom: 1px solid #f0f0f5; color: #3a3a3c; text-align: left; }
.fab-md :deep(tr:last-child td) { border-bottom: none; }
.fab-md :deep(tr:hover td) { background: #f8f9fa; }
.cursor-blink { animation: blink 1s step-end infinite; }
@keyframes blink { 50% { opacity: 0; } }
@keyframes bounce-dot { 0%,80%,100%{transform:scale(0.6);opacity:0.4} 40%{transform:scale(1);opacity:1} }
.animate-bounce { animation: bounce-dot 1.2s infinite ease-in-out; }

/* ═══ Animation ═══ */
@keyframes fade-in { from{opacity:0;transform:translateY(4px)} to{opacity:1;transform:translateY(0)} }
.animate-fade-in { animation: fade-in 0.25s ease-out; }
@keyframes thinking-shimmer {
  0%   { background-position: 200% center; }
  100% { background-position: -200% center; }
}
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
  animation: thinking-shimmer 1.8s ease-in-out infinite;
  pointer-events: none;
  border-radius: inherit;
}
</style>
