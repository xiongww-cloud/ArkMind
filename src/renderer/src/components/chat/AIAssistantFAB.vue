<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { kbAgentChat, kbSaveSessions, kbLoadSessions, kbDeleteSession } from '@/api/db-client'
import { sanitizeHtml } from '@/utils/sanitize'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import AppIcon from '@/components/common/AppIcon.vue'

async function renderMd(text: string): Promise<string> {
  if (!text) return ''
  try {
    const mod = await import('marked')
    const Marked = mod.Marked || (mod as any).default?.Marked || (mod as any).default
    if (Marked && typeof Marked === 'function') {
      const marked = new Marked({ gfm: true, breaks: true })
      return sanitizeHtml(marked.parse(text) || '')
    }
    const { marked } = mod as any
    return sanitizeHtml((marked?.parse || marked)(text) || '')
  } catch { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;') }
}

// ─── Types ──────────────────────────────────────────────────
interface ChatMessage { id: string; role: string; content: string; error?: string; thinking?: boolean; typewriterText?: string; typewriterDone?: boolean; timestamp?: number; toolCalls?: any[]; rawToolCalls?: any[]; toolResults?: any[] }
interface ChatSession { id: string; name: string; messages: ChatMessage[]; createdAt: string; updatedAt: string }

// ─── State ──────────────────────────────────────────────────
const open = ref(false)
const inputText = ref('')
const isChatting = ref(false)
const thinkingProgress = ref('')
const chatListRef = ref<HTMLElement | null>(null)
const rendered = ref<Record<string, string>>({})
const activeTypewriters = new Set<string>()
const typewriterTimers = new Map<string, ReturnType<typeof setInterval>>()

// ─── Session Management ─────────────────────────────────────
const sessions = ref<ChatSession[]>([])
const currentSessionId = ref<string | null>(null)
const showSessionDropdown = ref(false)
const sessionNameEdit = ref('')
const editingSessionId = ref<string | null>(null)
const sessionDirty = ref(false)
let lastSavedContent = ''

const currentSession = computed(() => sessions.value.find(s => s.id === currentSessionId.value) || null)

// 代理 messages 到当前会话
const messages = computed<ChatMessage[]>({
  get: () => currentSession.value?.messages || [],
  set: (val) => { if (currentSession.value) currentSession.value.messages = val }
})

function newSession() {
  stopAllTypewriters()
  const id = crypto.randomUUID()
  const now = new Date().toLocaleString('sv-SE')
  sessions.value.unshift({ id, name: '新对话', messages: [], createdAt: now, updatedAt: now })
  currentSessionId.value = id
  saveToStorage()
}

async function switchSession(id: string) {
  if (currentSessionId.value === id) return
  stopAllTypewriters()
  currentSessionId.value = id
  // 渲染当前会话的 AI 消息
  const session = currentSession.value
  if (session) {
    session.messages.forEach((m) => {
      if (m.role === 'assistant' && m.content && !m.thinking && !rendered.value[m.id]) {
        renderMd(m.content.replace(/~/g, '～')).then(html => { rendered.value[m.id] = html })
      }
    })
  }
  scrollChat()
}

async function deleteSession(id: string) {
  if (sessions.value.length <= 1) return
  const ok = await dangerConfirm({ title: '删除会话', message: '确定删除此会话？此操作不可撤销。', confirmText: '删除' })
  if (!ok) return
  kbDeleteSession(id).catch(() => {})
  sessions.value = sessions.value.filter(s => s.id !== id)
  if (currentSessionId.value === id) {
    currentSessionId.value = sessions.value[0]?.id ?? null
  }
  sessionDirty.value = true
  saveToStorage()
  toast({ message: '会话已删除', type: 'success' })
}

function startRenameSession(id: string) {
  const s = sessions.value.find(x => x.id === id)
  if (s) { editingSessionId.value = id; sessionNameEdit.value = s.name }
}

function saveRenameSession(id: string) {
  const s = sessions.value.find(x => x.id === id)
  if (s && sessionNameEdit.value.trim()) {
    s.name = sessionNameEdit.value.trim()
    sessionDirty.value = true
    saveToStorage()
  }
  editingSessionId.value = null
}

function saveToStorage() {
  const s = currentSession.value
  if (s && s.messages.length > 0 && sessionDirty.value) {
    const currentContent = JSON.stringify(s.messages)
    if (currentContent === lastSavedContent) {
      sessionDirty.value = false
      return
    }
    s.updatedAt = new Date().toLocaleString('sv-SE')
    sessions.value.sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime())
    sessionDirty.value = false
    lastSavedContent = currentContent
    kbSaveSessions([s]).catch((e) => console.error('[FAB] 保存会话失败:', e))
  }
}

async function loadFromStorage() {
  try {
    const r = await kbLoadSessions()
    if (r?.sessions && Array.isArray(r.sessions)) {
      sessions.value = r.sessions.map((s: any) => ({
        ...s,
        name: s.title || '新对话',
        updatedAt: s.updated_at || s.updatedAt,
        createdAt: s.created_at || s.createdAt,
        messages: (s.messages || []).map((m: any) => {
          const { isThinking, typewriterText, typewriterDone, ...rest } = m
          return rest
        })
      })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime())
    }
  } catch { sessions.value = [] }
  if (!sessions.value.length) { newSession(); return }
  currentSessionId.value = sessions.value[0].id
  // 初始化
  const s = currentSession.value
  if (s) {
    for (const m of s.messages) {
      if (m.role === 'assistant') { m.thinking = false; m.typewriterDone = true }
    }
    lastSavedContent = JSON.stringify(s.messages)
  }
  // 渲染 AI 消息
  const msgs = sessions.value[0].messages
  await Promise.all(msgs.map((m) => {
    if (m.role === 'assistant' && m.content) {
      return renderMd(m.content.replace(/~/g, '～')).then(html => { rendered.value[m.id] = html })
    }
    return Promise.resolve()
  }))
  scrollChat()
}

// ─── Drag ────────────────────────────────────────────────────
const fabY = ref(0)
const dragging = ref(false)
const dragStartY = ref(0)
const dragStartFabY = ref(0)
const dragMoved = ref(false)
const DRAG_THRESHOLD = 4
const winH = ref(700)

function initFabY() {
  winH.value = window.innerHeight
  fabY.value = window.innerHeight - 20 - 40
}
function onFabPointerDown(e: PointerEvent) {
  dragging.value = true
  dragMoved.value = false
  dragStartY.value = e.clientY
  dragStartFabY.value = fabY.value
  ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
}
function onFabPointerMove(e: PointerEvent) {
  if (!dragging.value) return
  const dy = e.clientY - dragStartY.value
  if (Math.abs(dy) > DRAG_THRESHOLD) dragMoved.value = true
  fabY.value = Math.max(12, Math.min(winH.value - 52, dragStartFabY.value + dy))
}
function onFabPointerUp() { dragging.value = false }

const suggestions = ['分析一下当前项目整体进展', '最近有什么值得关注的动态', '帮我梳理一下待办优先级']

function scrollChat() { nextTick(() => { if (chatListRef.value) chatListRef.value.scrollTop = chatListRef.value.scrollHeight }) }
function fmtTime() { return new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }) }

function stopTypewriter(msgId: string) {
  const timer = typewriterTimers.get(msgId)
  if (timer) { clearInterval(timer); typewriterTimers.delete(msgId) }
  activeTypewriters.delete(msgId)
}
function stopAllTypewriters() {
  for (const msgId of activeTypewriters) {
    const timer = typewriterTimers.get(msgId)
    if (timer) clearInterval(timer)
    const msg = messages.value.find(m => m.id === msgId)
    if (msg) {
      // 确保渲染完成后再标记完成
      const full = msg.typewriterText || msg.content || ''
      renderMd(full.replace(/~/g, '～')).then(h => {
        rendered.value[msgId] = h
        msg.typewriterDone = true
      }).catch(() => { msg.typewriterDone = true })
    }
  }
  activeTypewriters.clear()
  typewriterTimers.clear()
}

function startTypewriter(msgId: string, text: string) {
  stopTypewriter(msgId)
  const msg = messages.value.find(m => m.id === msgId)
  if (!msg) return
  if (!text) { msg.typewriterText = ''; msg.typewriterDone = true; return }
  const full = text.replace(/~/g, '～')
  // 预渲染完整文本
  renderMd(full).then(h => { rendered.value[msgId] = h })
  let pos = 0
  msg.typewriterText = ''
  msg.typewriterDone = false
  activeTypewriters.add(msgId)
  const timer = setInterval(() => {
    pos += 4
    if (pos >= full.length) {
      clearInterval(timer)
      typewriterTimers.delete(msgId)
      activeTypewriters.delete(msgId)
      msg.typewriterText = full
      // 确保渲染完成后再标记 typewriterDone
      renderMd(full).then(h => {
        rendered.value[msgId] = h
        msg.typewriterDone = true
      }).catch(() => { msg.typewriterDone = true })
      return
    }
    msg.typewriterText = full.slice(0, pos)
  }, 8)
  typewriterTimers.set(msgId, timer)
}

// ─── Send ───────────────────────────────────────────────────
async function send() {
  const q = inputText.value.trim()
  if (!q || isChatting.value) return
  stopAllTypewriters()
  inputText.value = ''
  nextTick(() => { const el = document.querySelector('.fab-input') as HTMLTextAreaElement; if (el) el.style.height = 'auto' })
  
  // 如果是第一条消息，用用户提问作为会话名称
  if (currentSession.value && currentSession.value.messages.length === 0 && currentSession.value.name === '新对话') {
    currentSession.value.name = q.slice(0, 30)
  }
  
  messages.value.push({ id: crypto.randomUUID(), role: 'user', content: q, timestamp: Date.now() })
  const aiMsg = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true }
  messages.value.push(aiMsg)
  sessionDirty.value = true
  scrollChat()
  isChatting.value = true
  thinkingProgress.value = ''

  // ★ 冷启动障眼法：立即显示拟人化短句
  const thinkingPhrases = ['让我想想啊……', '嗯……我回忆一下。', '别急，我翻一下脑子里的记录。', '稍等，我查查看……', '嗯，我想想……']
  const randomPhrase = thinkingPhrases[Math.floor(Math.random() * thinkingPhrases.length)]
  aiMsg.content = randomPhrase
  scrollChat()

  const unlistenProgress = await listen<{ msg: string }>('agent:progress', (ev) => { thinkingProgress.value = ev.payload.msg })
  let unlistenStream: (() => void) | null = null
  let unlistenThought: (() => void) | null = null
  let unlistenStatus: (() => void) | null = null
  try {
    // ★ 监听 status 事件，动态演变拟人化文案
    const toolHints: Record<string, string> = {
      ext_tool_search: '我正在检索相关资料',
      ext_tool_object: '我在翻看记忆中的对象',
      ext_tool_traverse: '我在继续往后翻看',
      ext_tool_batch_search: '我在同时查几个方向',
    }
    unlistenStatus = await listen<{ stage: string; tool?: string; target?: string }>('agent:status', (ev) => {
      const { stage, tool } = ev.payload
      if (stage === 'tool_start' && tool) {
        const hint = toolHints[tool] || `正在调用 ${tool}`
        aiMsg.content = hint
        scrollChat()
      }
    })
    const streamed = ref('')
    unlistenStream = await listen<{ token: string }>('agent:token', (ev) => {
      streamed.value += ev.payload.token
      aiMsg.content = streamed.value
      scrollChat()
    })
    unlistenThought = await listen<{ text: string }>('agent:thought', (ev) => {
      let idx = 0; const full = ev.payload.text; thinkingProgress.value = ''
      const t = setInterval(() => { thinkingProgress.value = full.slice(0, ++idx); if (idx >= full.length) clearInterval(t) }, 30)
    })
    const history: any[] = []
    for (const m of messages.value) {
      if (m === aiMsg) continue
      if (m.role === 'user') history.push({ role: 'user', content: m.content })
      else if (m.role === 'assistant') {
        // ★ 包含 tool_calls 以保持完整上下文
        if ((m as any).toolCalls?.length) {
          history.push({ role: 'assistant', content: m.content || '', tool_calls: (m as any).toolCalls })
        } else {
          history.push({ role: 'assistant', content: m.content || '' })
        }
      }
    }
    const result: any = await kbAgentChat({ query: q, history })
    aiMsg.thinking = false; thinkingProgress.value = ''
    if (result.success) {
      // 过滤 XML 工具调用标签和 refs 标签
      let content = (result.answer || '')
        .replace(/<｜｜DSML｜｜[\s\S]*?<\/｜｜DSML｜｜tool_calls>/g, '')
        .replace(/<tool_calls>[\s\S]*?<\/tool_calls>/g, '')
        .replace(/<refs>[\s\S]*<\/refs>/g, '')
        .trim()
      aiMsg.content = content
      if (streamed.value) {
        content = streamed.value
          .replace(/<｜｜DSML｜｜[\s\S]*?<\/｜｜DSML｜｜tool_calls>/g, '')
          .replace(/<tool_calls>[\s\S]*?<\/tool_calls>/g, '')
          .replace(/<refs>[\s\S]*<\/refs>/g, '')
          .trim()
        aiMsg.content = content
      }
      // ★ 提取工具调用和结果
      if (result.tools && Array.isArray(result.tools)) {
        const rawCalls: any[] = []
        const toolResults: any[] = []
        for (const t of result.tools) {
          const callId = 'call_' + Math.random().toString(36).slice(2, 8)
          rawCalls.push({ id: callId, type: 'function', function: { name: t.tool, arguments: typeof t.args === 'string' ? t.args : JSON.stringify(t.args) } })
          const rawResult = typeof t.result === 'string' ? t.result : ''
          const truncated = rawResult.length > 30000 ? rawResult.substring(0, 30000) + '\n\n[结果已截断]' : rawResult
          toolResults.push({ id: callId, name: t.tool, content: truncated })
        }
        aiMsg.rawToolCalls = rawCalls
        aiMsg.toolResults = toolResults
      }
    } else {
      aiMsg.content = result.answer || result.error || '失败'
    }
    startTypewriter(aiMsg.id, aiMsg.content)
  } catch (e: any) {
    aiMsg.thinking = false
    aiMsg.error = typeof e === 'string' ? e : (e.message || 'AI 服务异常')
  } finally {
    isChatting.value = false; scrollChat()
    sessionDirty.value = true; saveToStorage()
    try { unlistenStream?.(); unlistenThought?.(); unlistenStatus?.(); unlistenProgress() } catch {}
  }
}

async function stopAgent() {
  try {
    const { agentCancel } = await import('@/api/db-client')
    await agentCancel()
    isChatting.value = false
    // 更新最后一条 AI 消息
    const session = currentSession.value
    if (session) {
      const lastMsg = session.messages[session.messages.length - 1]
      if (lastMsg && lastMsg.role === 'assistant' && lastMsg.thinking) {
        lastMsg.thinking = false
        if (!lastMsg.content) lastMsg.content = '已停止执行。'
      }
    }
    scrollChat()
  } catch (e) {
    console.error('[FAB] 停止失败:', e)
  }
}

function toggle() {
  if (dragMoved.value) return
  open.value = !open.value
  if (open.value) {
    // 打开时重新加载会话，保持与知识库页面同步
    loadFromStorage()
    nextTick(() => { scrollChat() })
  }
}

// ─── Lifecycle ──────────────────────────────────────────────
function handleClickOutside(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (!target.closest('.fab-panel')) {
    showSessionDropdown.value = false
  }
}

onMounted(async () => {
  initFabY()
  await loadFromStorage()
  document.addEventListener('click', handleClickOutside)
})

onUnmounted(() => {
  saveToStorage()
  stopAllTypewriters()
  document.removeEventListener('click', handleClickOutside)
})

// 监听会话切换，清空渲染缓存
watch(currentSessionId, () => {
  rendered.value = {}
  stopAllTypewriters()
})
</script>

<template>
<div class="fab-root" :style="{ top: fabY + 'px' }">
  <!-- Floating Button -->
  <button @click="toggle" @pointerdown="onFabPointerDown" @pointermove="onFabPointerMove" @pointerup="onFabPointerUp"
    class="fab-button" :class="{ 'fab-open': open }" title="AI 助手">
    <!-- Outer glow ring -->
    <span class="fab-glow" />
    <!-- Orbital rings -->
    <span class="fab-orbit fab-orbit--1" />
    <span class="fab-orbit fab-orbit--2" />
    <span class="fab-orbit fab-orbit--3" />
    <!-- Core -->
    <span class="fab-core">
      <svg class="fab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M21 11.5a8.38 8.38 0 01-.9 3.8 8.5 8.5 0 01-7.6 4.7 8.38 8.38 0 01-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 01-.9-3.8 8.5 8.5 0 014.7-7.6 8.38 8.38 0 013.8-.9h.5a8.48 8.48 0 018 8v.5z"/>
      </svg>
    </span>
    <span class="fab-label">AI</span>
  </button>

  <!-- Chat Panel -->
  <Transition name="fab-panel">
    <div v-if="open" class="fab-panel" :style="{ bottom: (winH - fabY + 8) + 'px' }">
      <!-- Scanline -->
      <div class="fab-scanline" />
      <!-- Close button -->
      <button @click="open = false" class="fab-panel__close">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 18L18 6M6 6l12 12"/></svg>
      </button>

      <!-- Title Bar -->
      <div class="flex items-center gap-1.5 px-3 py-1.5 border-b border-white/20 bg-white/20 backdrop-blur">
        <svg class="w-3.5 h-3.5 text-[#2563eb] flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>
        
        <!-- Session Selector -->
        <div class="relative flex-1 min-w-0">
          <button @click="showSessionDropdown = !showSessionDropdown"
            @blur="setTimeout(() => showSessionDropdown = false, 150)"
            class="flex items-center gap-1 px-2 py-1 rounded-lg hover:bg-white/30 transition-colors max-w-full w-full">
            <span class="text-[11px] font-medium text-[#3a3a3c] truncate">{{ currentSession?.name || '新对话' }}</span>
            <svg class="w-3 h-3 text-[#86868b] flex-shrink-0 transition-transform" :class="showSessionDropdown ? 'rotate-180' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M19 9l-7 7-7-7"/></svg>
          </button>
          
          <!-- Dropdown -->
          <div v-if="showSessionDropdown" class="absolute top-full left-0 mt-1 w-72 bg-white rounded-xl shadow-lg border border-[#e5e5ea] z-50 overflow-hidden" @click.stop>
            <div class="px-3 py-2 border-b border-[#e5e5ea]">
              <button @click="newSession(); showSessionDropdown = false" class="flex items-center gap-1.5 w-full px-2 py-1.5 rounded-lg text-[12px] text-[#2563eb] hover:bg-blue-50 transition-colors">
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>
                新建对话
              </button>
            </div>
            <div class="overflow-y-auto max-h-[300px] py-1">
              <div v-for="s in sessions" :key="s.id"
                class="group flex items-center gap-2 px-3 py-2 cursor-pointer hover:bg-[#f8f9fa] transition-colors"
                :class="s.id === currentSessionId ? 'bg-[#2563eb]/5' : ''"
                @click="switchSession(s.id); showSessionDropdown = false">
                <svg class="w-3.5 h-3.5 flex-shrink-0" :class="s.id === currentSessionId ? 'text-[#2563eb]' : 'text-[#86868b]'" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"/></svg>
                <template v-if="editingSessionId === s.id">
                  <input v-model="sessionNameEdit" @keyup.enter="saveRenameSession(s.id)" @keyup.escape="editingSessionId = null" @blur="saveRenameSession(s.id)"
                    class="flex-1 min-w-0 bg-white border border-[#2563eb] rounded px-1.5 py-0.5 text-[12px] outline-none" @click.stop autofocus />
                </template>
                <template v-else>
                  <div class="flex-1 min-w-0">
                    <div class="truncate text-[12px]" :class="s.id === currentSessionId ? 'font-medium text-[#1d1d1f]' : 'text-[#3a3a3c]'">{{ s.name }}</div>
                    <div v-if="s.messages.length" class="text-[10px] text-[#86868b] truncate mt-0.5">{{ (() => { const m = [...s.messages].reverse().find((m: any) => !m.thinking); return m?.content?.slice(0, 40) || '...' })() }}</div>
                  </div>
                  <button @click.stop="startRenameSession(s.id)" class="w-5 h-5 flex items-center justify-center rounded text-[#86868b] hover:text-[#2563eb] hover:bg-[#2563eb]/5 opacity-0 group-hover:opacity-100 transition-all" title="重命名">
                    <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg>
                  </button>
                  <button v-if="sessions.length > 1" @click.stop="deleteSession(s.id)" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" title="删除">
                    <AppIcon name="trash" size="12"/>
                  </button>
                </template>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Messages -->
      <div ref="chatListRef" class="fab-panel__body">
        <!-- Empty -->
        <div v-if="messages.length === 0" class="flex flex-col items-center justify-center h-full px-5 py-6 text-center">
          <div class="w-14 h-14 mb-3 rounded-2xl bg-gradient-to-br from-[#2563eb]/10 to-[#93c5fd]/10 flex items-center justify-center ring-1 ring-[#2563eb]/10">
            <svg class="w-7 h-7 text-[#2563eb]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
              <path d="M21 11.5a8.38 8.38 0 01-.9 3.8 8.5 8.5 0 01-7.6 4.7 8.38 8.38 0 01-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 01-.9-3.8 8.5 8.5 0 014.7-7.6 8.38 8.38 0 013.8-.9h.5a8.48 8.48 0 018 8v.5z"/>
            </svg>
          </div>
          <h3 class="text-[14px] font-semibold text-[#1d1d1f] mb-1">AI 知识助手</h3>
          <p class="text-[11px] text-[#86868b] mb-4 leading-relaxed">基于你的知识库回答问题</p>
          <div class="grid grid-cols-1 gap-1.5 w-full max-w-[300px]">
            <div v-for="(s, i) in suggestions" :key="i" @click="inputText = s; send()"
              class="bg-white border border-[#e5e5ea] rounded-xl px-3 py-2 text-left cursor-pointer hover:border-[#2563eb]/30 hover:shadow-sm transition-all duration-200">
              <div class="text-[10px] text-[#2563eb] font-medium mb-0.5">建议 {{ i + 1 }}</div>
              <div class="text-[12px] text-[#3a3a3c] leading-relaxed">{{ s }}</div>
            </div>
          </div>
        </div>

        <!-- Messages -->
        <div v-for="m in messages" :key="m.id" class="flex gap-2 items-start animate-fade-in mb-2.5"
          :class="m.role === 'user' ? 'flex-row-reverse' : ''">
          <div v-if="m.role === 'user'" class="w-6 h-6 rounded-full flex items-center justify-center text-[10px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
          <div v-else class="w-6 h-6 rounded-full flex items-center justify-center text-[10px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
          <div class="min-w-0" :class="m.role === 'user' ? 'max-w-[70%]' : 'max-w-[80%]'">
            <div class="text-[10px] text-[#c7c7cc] mb-0.5 px-0.5" :class="m.role === 'user' ? 'text-right' : ''">{{ fmtTime() }}</div>
            <!-- Thinking -->
            <div v-if="m.thinking" class="bg-white border border-[#e5e5ea] rounded-2xl px-3 py-1.5 shadow-sm rounded-tl-[4px]">
              <div class="flex items-center gap-1.5">
                <div class="flex gap-0.5">
                  <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:0ms"/>
                  <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:150ms"/>
                  <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:300ms"/>
                </div>
                <span class="text-[11px] text-[#86868b] transition-colors duration-300">{{ m.content || thinkingProgress || '思考中' }}</span>
              </div>
            </div>
            <!-- User -->
            <div v-else-if="m.role === 'user'" class="bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-1.5">
              <pre class="text-[12px] leading-relaxed whitespace-pre-wrap font-sans">{{ m.content }}</pre>
            </div>
            <!-- AI -->
            <div v-else class="fab-bubble-ai">
              <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
              <div v-else-if="m.typewriterText !== undefined && !m.typewriterDone" class="fab-md" v-html="rendered[m.id] || ''"></div>
              <div v-else-if="rendered[m.id]" class="fab-md" v-html="rendered[m.id]" />
              <div v-else class="text-[11px] text-[#3a3a3c] leading-relaxed whitespace-pre-wrap">{{ m.content }}</div>
            </div>
          </div>
        </div>
      </div>

      <!-- Input -->
      <div class="fab-panel__foot">
        <div class="fab-input-wrap">
          <textarea v-model="inputText" rows="1" placeholder="向 AI 提问..." class="fab-input"
            @keydown.enter.exact.prevent="send"
            @input="(e: any) => { e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 56) + 'px' }" />
          <button v-if="isChatting" @click="stopAgent"
            class="w-7 h-7 rounded-full bg-red-500 hover:bg-red-600 text-white flex items-center justify-center transition-colors flex-shrink-0" title="停止">
            <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
          </button>
          <button v-else @click="send" :disabled="!inputText.trim()"
            class="fab-send" :class="{ 'fab-send--on': inputText.trim() }">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
          </button>
        </div>
      </div>
    </div>
  </Transition>
</div>
</template>

<style scoped>
/* ═══ Orb ═══ */
.fab-root { position: fixed; right: 16px; z-index: 9999; cursor: grab; touch-action: none; user-select: none; }
.fab-root:active { cursor: grabbing; }
.fab-root:has(.fab-panel) { cursor: default; }

.fab-button {
  position: relative; width: 40px; height: 40px; border-radius: 50%;
  background: transparent;
  display: flex; align-items: center; justify-content: center;
  border: none; cursor: pointer;
  transition: transform 0.2s;
}
.fab-button:hover { transform: scale(1.1); }
.fab-button.fab-open { animation: none; transform: scale(0.85); opacity: 0; pointer-events: none; }

/* Core sphere */
.fab-core {
  position: relative; z-index: 2;
  width: 32px; height: 32px; border-radius: 50%;
  background: linear-gradient(135deg, #2563eb 0%, #6366f1 50%, #a855f7 100%);
  display: flex; align-items: center; justify-content: center;
  box-shadow:
    0 0 12px rgba(99,102,241,0.5),
    0 0 24px rgba(37,99,235,0.3),
    inset 0 1px 1px rgba(255,255,255,0.3),
    inset 0 -1px 2px rgba(0,0,0,0.2);
}

.fab-icon { width: 16px; height: 16px; color: white; filter: drop-shadow(0 1px 2px rgba(0,0,0,0.3)); }

/* Glow aura */
.fab-glow {
  position: absolute; inset: -6px; border-radius: 50%;
  background: radial-gradient(circle, rgba(99,102,241,0.25) 0%, transparent 70%);
  animation: glow-pulse 3s ease-in-out infinite;
  pointer-events: none;
}
@keyframes glow-pulse {
  0%, 100% { opacity: 0.6; transform: scale(1); }
  50% { opacity: 1; transform: scale(1.15); }
}

/* Orbital rings */
.fab-orbit {
  position: absolute; border-radius: 50%;
  pointer-events: none;
}
.fab-orbit--1 {
  inset: -5px;
  border: 1px solid rgba(99,102,241,0.25);
  border-style: dashed;
  animation: spin 8s linear infinite;
}
.fab-orbit--2 {
  inset: -9px;
  border: 1px solid rgba(37,99,235,0.12);
  animation: spin 14s linear infinite reverse;
}
.fab-orbit--2::before {
  content: '';
  position: absolute; top: -2px; left: 50%; width: 4px; height: 4px;
  background: #818cf8; border-radius: 50%;
  box-shadow: 0 0 6px #818cf8;
  transform: translateX(-50%);
  animation: dot-orbit 14s linear infinite reverse;
}
.fab-orbit--3 {
  inset: -13px;
  border: 1px solid rgba(168,85,247,0.08);
  border-style: dotted;
  animation: spin 22s linear infinite;
}
@keyframes spin { from{transform:rotate(0deg)} to{transform:rotate(360deg)} }

.fab-label {
  position: absolute; bottom: -1px; right: -1px; font-size: 6px; font-weight: 800; color: white;
  background: linear-gradient(135deg, #6366f1, #a855f7); padding: 1px 3px; border-radius: 4px;
  letter-spacing: 0.5px; box-shadow: 0 1px 4px rgba(0,0,0,0.4); z-index: 3;
  border: 1px solid rgba(255,255,255,0.15);
}
@keyframes spin { from{transform:rotate(0deg)} to{transform:rotate(360deg)} }

/* ═══ Panel ═══ */
.fab-panel {
  position: fixed; right: 16px;
  width: 576px; height: 72vh;
  transform-origin: bottom right;
  background: rgba(255,255,255,0.88); backdrop-filter: blur(24px); -webkit-backdrop-filter: blur(24px);
  border-radius: 14px;
  border: none;
  box-shadow: 0 16px 48px rgba(0,0,0,0.12), 0 0 0 1px rgba(0,0,0,0.04);
  display: flex; flex-direction: column; overflow: hidden;
}
/* Gradient border */
.fab-panel::before {
  content: ''; position: absolute; inset: 0; border-radius: 14px; padding: 1px;
  background: linear-gradient(135deg, rgba(6,182,212,0.3), rgba(37,99,235,0.25), rgba(99,102,241,0.2), rgba(168,85,247,0.15));
  -webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
  -webkit-mask-composite: xor; mask-composite: exclude;
  pointer-events: none; z-index: 1;
}
/* Corner decorations */
.fab-panel::after {
  content: ''; position: absolute; inset: 0; border-radius: 14px; pointer-events: none; z-index: 2;
  background:
    /* top-left */
    linear-gradient(135deg, rgba(6,182,212,0.5) 0%, transparent 3%) no-repeat 0 0,
    /* top-right */
    linear-gradient(225deg, rgba(37,99,235,0.5) 0%, transparent 3%) no-repeat 100% 0,
    /* bottom-left */
    linear-gradient(45deg, rgba(99,102,241,0.5) 0%, transparent 3%) no-repeat 0 100%,
    /* bottom-right */
    linear-gradient(315deg, rgba(168,85,247,0.5) 0%, transparent 3%) no-repeat 100% 100%;
}

/* Scanline */
.fab-scanline {
  position: absolute; top: 0; left: 0; right: 0; height: 1px; z-index: 3;
  background: linear-gradient(90deg, transparent 0%, rgba(6,182,212,0.4) 30%, rgba(37,99,235,0.6) 50%, rgba(6,182,212,0.4) 70%, transparent 100%);
  animation: scan-sweep 4s ease-in-out infinite;
}
@keyframes scan-sweep {
  0% { transform: translateX(-100%); opacity: 0; }
  20% { opacity: 1; }
  80% { opacity: 1; }
  100% { transform: translateX(100%); opacity: 0; }
}

.fab-panel__close {
  position: absolute; top: 8px; right: 8px; z-index: 10;
  width: 24px; height: 24px; border-radius: 6px;
  display: flex; align-items: center; justify-content: center;
  color: #94a3b8; background: transparent; border: none;
  cursor: pointer; transition: all 0.15s;
}
.fab-panel__close:hover { color: #3a3a3c; background: rgba(0,0,0,0.04); box-shadow: 0 0 8px rgba(37,99,235,0.1); }

.fab-panel__body {
  flex: 1; min-height: 0; overflow-y: auto; padding: 12px; background: transparent;
  scrollbar-width: thin; scrollbar-color: rgba(37,99,235,0.2) transparent;
  cursor: text; user-select: text;
}
.fab-panel__body::-webkit-scrollbar { width: 3px; }
.fab-panel__body::-webkit-scrollbar-track { background: transparent; }
.fab-panel__body::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

.fab-panel__foot { padding: 8px 12px; flex-shrink: 0; border-top: 1px solid rgba(37,99,235,0.08); background: rgba(255,255,255,0.6); }

.fab-input-wrap {
  display: flex; align-items: center; gap: 6px;
  background: rgba(248,249,251,0.8); border: 1px solid #e5e5ea; border-radius: 10px; padding: 6px 10px;
  transition: border-color 0.2s, box-shadow 0.2s;
}
.fab-input-wrap:focus-within {
  border-color: rgba(6,182,212,0.4);
  box-shadow: 0 0 0 3px rgba(37,99,235,0.06), 0 0 12px rgba(6,182,212,0.08);
  background: white;
}

.fab-input { flex: 1; border: none; resize: none; outline: none; font-size: 12px; color: #3a3a3c; background: transparent; line-height: 1.5; max-height: 56px; font-family: inherit; }
.fab-input::placeholder { color: #94a3b8; }

.fab-send { width: 28px; height: 28px; border-radius: 8px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; border: none; cursor: pointer; background: linear-gradient(135deg, #e2e8f0, #f1f5f9); color: #94a3b8; transition: all 0.2s; }
.fab-send--on { background: linear-gradient(135deg, #06b6d4, #2563eb); color: white; box-shadow: 0 1px 8px rgba(37,99,235,0.3); }
.fab-send--on:hover { box-shadow: 0 2px 12px rgba(37,99,235,0.45); transform: scale(1.05); }

/* ═══ AI Bubble ═══ */
.fab-bubble-ai {
  background: rgba(255,255,255,0.9);
  border: 1px solid #e5e5ea;
  border-radius: 0 12px 12px 12px;
  padding: 8px 12px;
  box-shadow: 0 1px 4px rgba(0,0,0,0.04);
}

/* ═══ Markdown ═══ */
.fab-md :deep(p) { margin: 0.25em 0; font-size: 11px; color: #3a3a3c; line-height: 1.65; }
.fab-md :deep(h1), .fab-md :deep(h2), .fab-md :deep(h3), .fab-md :deep(h4) { color: #1d1d1f; margin: 0.4em 0 0.15em; font-weight: 600; }
.fab-md :deep(h1) { font-size: 0.85rem; } .fab-md :deep(h2) { font-size: 0.8rem; } .fab-md :deep(h3) { font-size: 0.75rem; } .fab-md :deep(h4) { font-size: 0.7rem; }
.fab-md :deep(ul), .fab-md :deep(ol) { padding-left: 1.2em; margin: 0.15em 0; }
.fab-md :deep(li) { font-size: 11px; color: #3a3a3c; margin: 0.05em 0; }
.fab-md :deep(strong) { color: #1d1d1f; font-weight: 600; }
.fab-md :deep(code) { background: rgba(0,0,0,0.04); color: #e11d48; padding: 0px 3px; border-radius: 3px; font-size: 0.9em; }
.fab-md :deep(pre) { background: #1d1d1f; color: #f5f5f7; padding: 6px 10px; border-radius: 6px; overflow-x: auto; margin: 0.3em 0; font-size: 10px; }
.fab-md :deep(pre code) { background: none; color: inherit; padding: 0; }
.fab-md :deep(blockquote) { border-left: 2px solid #2563eb; padding: 2px 8px; margin: 0.2em 0; color: #6e6e73; background: rgba(37,99,235,0.03); border-radius: 0 4px 4px 0; }
.fab-md :deep(a) { color: #2563eb; }
.fab-md :deep(table) { width: 100%; border-collapse: collapse; margin: 0.3em 0; font-size: 10px; }
.fab-md :deep(th) { background: #f5f5f7; color: #1d1d1f; font-weight: 600; padding: 4px 6px; text-align: left; border-bottom: 1px solid rgba(0,0,0,0.06); }
.fab-md :deep(td) { padding: 4px 6px; color: #3a3a3c; border-bottom: 1px solid rgba(0,0,0,0.04); }
.cursor-blink { animation: blink 1s step-end infinite; }
@keyframes blink { 50% { opacity: 0; } }
@keyframes bounce-dot { 0%,80%,100%{transform:scale(0.6);opacity:0.4} 40%{transform:scale(1);opacity:1} }
.animate-bounce { animation: bounce-dot 1.2s infinite ease-in-out; }

/* ═══ Animation ═══ */
@keyframes fade-in { from{opacity:0;transform:translateY(4px)} to{opacity:1;transform:translateY(0)} }
.animate-fade-in { animation: fade-in 0.25s ease-out; }
.fab-panel-enter-active { transition: all 0.25s cubic-bezier(0.16,1,0.3,1); }
.fab-panel-leave-active { transition: all 0.15s ease; }
.fab-panel-enter-from { opacity: 0; transform: scale(0.88) translateY(12px); }
.fab-panel-leave-to { opacity: 0; transform: scale(0.94) translateY(6px); }
</style>
