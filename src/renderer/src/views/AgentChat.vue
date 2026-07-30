<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onUnmounted, nextTick, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import ChatSidebar from '@/components/chat/ChatSidebar.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import { kbAgentChat, kbChat, notesV2Create, fetchNotebooks, kbSaveSessions, kbLoadSessions, kbLoadSessionMessages, kbDeleteSession } from '@/api/db-client'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { useAgentChat, type ChatMessage as AgentChatMessage } from '@/composables/useAgentChat'
import { pickRetrievalPhrase } from '@/utils/chat'
import { ProjectCard, TodoCard, ObjectList } from '@/components/renderer'

const agent = useAgentChat()

// ─── Types ──────────────────────────────────────────────────────
interface ChatMessage extends AgentChatMessage { id?: string; role: 'user'|'assistant'; toolCalls?: any[]; rawToolCalls?: any[]; toolResults?: any[]; contextDocs?: any[]; workspace?: string }
interface ChatSession { id: string; name: string; messages: ChatMessage[]; createdAt: string }

function formatChatTime(ts: number) {
  const d = new Date(ts)
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today.getTime() - 86400000)
  const time = d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (d >= today) return `今天 ${time}`
  if (d >= yesterday) return `昨天 ${time}`
  return d.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

async function renderMessageContent(msgId: string, text: string) {
  if (!text) return
  const safeText = text.replace(/~/g, '～')
  agent.rendered.value[msgId] = await agent.renderMd(safeText)
}
function renderedContent(msgId: string): string { return agent.rendered.value[msgId] || '' }

// ─── Typewriter ─────────────────────────────────────────────────
const activeTypewriters = new Set<ChatMessage>()
const typewriterTimers = new Map<ChatMessage, ReturnType<typeof setInterval>>()
function startTypewriter(msg: ChatMessage, fullText: string, speed = 8) {
  stopTypewriter(msg)
  if (!fullText) { msg.content = ''; msg.typewriterText = ''; msg.typewriterDone = true; renderMessageContent(msg.id || '', ''); return }
  const msgId = msg.id || ''
  const TYPEWRITER_CHUNK_SIZE = 5
  let pos = 0
  msg.content = ''
  msg.typewriterText = ''
  msg.typewriterDone = false
  activeTypewriters.add(msg)
  let lastRenderTime = 0
  const RENDER_THROTTLE_MS = 80
  const timer = setInterval(() => {
    pos += TYPEWRITER_CHUNK_SIZE
    if (pos >= fullText.length) {
      pos = fullText.length; clearInterval(timer); typewriterTimers.delete(msg); activeTypewriters.delete(msg)
      msg.content = fullText; msg.typewriterText = fullText; msg.typewriterDone = true
      renderMessageContent(msgId, fullText)
      return
    }
    const partial = fullText.slice(0, pos)
    msg.content = partial
    msg.typewriterText = partial
    const now = Date.now()
    if (now - lastRenderTime > RENDER_THROTTLE_MS) {
      lastRenderTime = now
      renderMessageContent(msgId, partial)
    }
  }, speed)
  typewriterTimers.set(msg, timer)
}
function stopTypewriter(msg: ChatMessage) {
  const timer = typewriterTimers.get(msg)
  if (timer) { clearInterval(timer); typewriterTimers.delete(msg) }
  if (msg.typewriterText !== undefined && !msg.typewriterDone) { msg.typewriterDone = true }
  activeTypewriters.delete(msg)
}
function stopAllTypewriters() { for (const msg of activeTypewriters) { stopTypewriter(msg) } }
function typewriterText(msg: ChatMessage): string { return msg.typewriterText || '' }
function isTypewriting(msg: ChatMessage): boolean { return msg.typewriterText !== undefined && !msg.typewriterDone }

// ─── Session management ─────────────────────────────────────────
const sessions = ref<ChatSession[]>([])
const currentSessionId = ref<string|null>(null)
const sessionNameEdit = ref('')
const editingSessionId = ref<string|null>(null)
const sessionDirty = ref(false)  // 标记当前会话是否有变更
let lastSavedContent = ''  // 上次保存时的内容快照

function newSession() {
  stopAllTypewriters()
  const id = crypto.randomUUID()
  sessions.value.unshift({ id, name: '新对话', messages: [], createdAt: new Date().toLocaleString('sv-SE'), updatedAt: new Date().toLocaleString('sv-SE') })
  currentSessionId.value = id; saveToStorage()
}

// ★ 启动时加载持久化会话
onMounted(async () => {
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s:any) => ({ ...s, name: s.title || '新对话', updatedAt: s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt, messages: (s.messages || []).map((m: any) => { const { isThinking, typewriterText, typewriterDone, ...rest } = m; return { ...rest, dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.timestamp || (rest.created_at ? new Date(rest.created_at).getTime() : Date.now()) } }) })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch (e) { console.error('[KB] loadSessions failed:', e) }
  if (!sessions.value.length) { newSession() }
  else if (!currentSessionId.value) { currentSessionId.value = sessions.value[0].id }
  // 初始化分页状态：初始不足 10 条的会话没有更早消息
  for (const s of sessions.value) { sessionHasMore.value[s.id] = (s.messages?.length || 0) >= 10 }
  // 渲染当前会话的消息
  if (currentSessionId.value) { await selectSession(currentSessionId.value) }
  initialScrollDone = false
  nextTick(() => scrollToBottomIfNear())
})
// ★ 每次切换回来时重新加载会话（保持与全局对话框同步）
onActivated(async () => {
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
          return { ...rest, dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.timestamp || (rest.created_at ? new Date(rest.created_at).getTime() : Date.now()) }
        })
      })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime())
    }
    if (sessions.value.length && !currentSessionId.value) { currentSessionId.value = sessions.value[0].id }
    // 初始化分页状态
    for (const s of sessions.value) { sessionHasMore.value[s.id] = (s.messages?.length || 0) >= 10 }
  } catch {}
  initialScrollDone = false
  nextTick(() => scrollToBottomIfNear())
})
async function selectSession(id: string) {
  currentSessionId.value = id; agent.rendered.value = {}
  stopAllTypewriters()
  // Trigger markdown rendering for all AI messages — await all
  const session = sessions.value.find(s => s.id === id)
  if (session) {
    await Promise.all(session.messages.map((m) => {
      if (m.role === 'assistant' && m.content && !m.isThinking) {
        return renderMessageContent(m.id || '', m.content)
      }
      return Promise.resolve()
    }))
  }
  scrollToBottomIfNear()
}
const currentSession = computed(() => sessions.value.find(s => s.id === currentSessionId.value) || null)

function saveToStorage() { 
  const s = currentSession.value; 
  if (s && s.messages.length > 0 && sessionDirty.value) {
    // 只有内容实际变化时才更新时间戳
    const currentContent = JSON.stringify(s.messages)
    if (currentContent === lastSavedContent) {
      sessionDirty.value = false
      return
    }
    s.updatedAt = new Date().toLocaleString('sv-SE'); 
    sessions.value.sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()); 
    sessionDirty.value = false;
    lastSavedContent = currentContent
    kbSaveSessions([s]).catch((e) => console.error('[Agent] 保存会话失败:', e)); 
  }
}
async function loadFromStorage() {
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s: any) => ({ ...s, name: s.title || '新会话', updatedAt: s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt, messages: (s.messages || []).map((m: any) => { const { isThinking, typewriterText, typewriterDone, ...rest } = m; return rest }) })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch { sessions.value = [] }
  if (!sessions.value.length) { newSession(); return }
  currentSessionId.value = sessions.value[0].id
  // 初始化内容快照
  const s = currentSession.value
  if (s) {
    for (const m of s.messages) {
      if (m.role === 'assistant') {
        m.isThinking = false
        m.typewriterDone = true
      }
    }
    lastSavedContent = JSON.stringify(s.messages)
  }
  // Render markdown for loaded AI messages — await all renders
  const msgs = sessions.value[0].messages
  await Promise.all(msgs.map((m) => {
    if (m.role === 'assistant' && m.content) {
      return renderMessageContent(m.id || '', m.content)
    }
    return Promise.resolve()
  }))
  scrollToBottomIfNear()
}

// ─── Chat state ─────────────────────────────────────────────────
const inputMessage = ref('')
const textareaRef = ref<HTMLTextAreaElement | null>(null)
const isDrawerOpen = ref(false)
const showWorkspaceIdx = ref<number | null>(null)
const feedScrollRef = ref<HTMLElement|null>(null)
const isRefining = ref(false)
const sidebarOpen = ref(true)

// ─── 消息分页加载 ────────────────────────────────────────────
const loadingMore = ref(false)
const sessionHasMore = ref<Record<string, boolean>>({})

async function loadOlderMessages() {
  const el = feedScrollRef.value
  const session = currentSession.value
  if (!el || !session || loadingMore.value) return
  const oldestDbId = session.messages.find((m: any) => m.dbId)?.dbId
  if (sessionHasMore.value[session.id] === false) return
  loadingMore.value = true
  const prevHeight = el.scrollHeight
  try {
    const r = await kbLoadSessionMessages(session.id, oldestDbId, 10)
    if (r?.success && Array.isArray(r.messages) && r.messages.length > 0) {
      const older = r.messages.map((m: any) => {
        const { isThinking, typewriterText, typewriterDone, ...rest } = m
        return { ...rest, dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.created_at ? new Date(rest.created_at).getTime() : Date.now() }
      })
      session.messages.unshift(...older)
      // 渲染旧消息的 markdown
      for (const m of older) {
        if (m.role === 'assistant' && m.content && !m.isThinking) {
          renderMessageContent(m.id || '', m.content)
        }
      }
      nextTick(() => { el.scrollTop = el.scrollHeight - prevHeight })
      sessionHasMore.value[session.id] = r.messages.length >= 10
    } else {
      sessionHasMore.value[session.id] = false
    }
  } catch {}
  loadingMore.value = false
}

function onFeedScroll() {
  const el = feedScrollRef.value
  if (!el || el.scrollTop > 100) return
  loadOlderMessages()
}

let unlistenProgress: (() => void) | null = null
let unlistenRoundThought: (() => void) | null = null
let unlistenDone: (() => void) | null = null
let unlistenError: (() => void) | null = null
let hasRoundThought = false

async function confirmDeleteSession(id: string) {
  const ok = await dangerConfirm({ title: '删除会话', message: '确定删除此会话？此操作不可撤销。', confirmText: '删除' })
  if (!ok) return
  kbDeleteSession(id).catch(() => {})
  sessions.value = sessions.value.filter(s => s.id !== id)
  if (currentSessionId.value === id) {
    const nextId = sessions.value[0]?.id ?? null
    if (nextId) await selectSession(nextId)
    else currentSessionId.value = null
  }
  sessionDirty.value = true; saveToStorage()
  toast({ message: '会话已删除', type: 'success' })
}

let initialScrollDone = false
function scrollToBottomIfNear() {
  const el = feedScrollRef.value as HTMLElement | null; if (!el) return
  if (!initialScrollDone) { initialScrollDone = true; nextTick(() => { el.scrollTop = el.scrollHeight }); return }
  const SCROLL_NEAR_BOTTOM_THRESHOLD = 150
  const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_NEAR_BOTTOM_THRESHOLD
  if (nearBottom) { nextTick(() => { el.scrollTop = el.scrollHeight }) }
}

watch(currentSessionId, () => { initialScrollDone = false })

// ─── Context recall ─────────────────────────────────────────────
const recalledContext = computed(() => {
  const session = currentSession.value; if (!session) return []
  for (let i = session.messages.length - 1; i >= 0; i--) {
    const msg = session.messages[i]
    if (msg.role === 'assistant' && msg.contextDocs?.length) return msg.contextDocs
  }
  return []
})

// ─── Send message ───────────────────────────────────────────────

const MAX_INPUT_LENGTH = 5000
async function handleSendMessage() {
  const q = inputMessage.value.trim()
  if (!q || agent.isChatting.value) return
  console.log('[FE] handleSendMessage BUILD=streamfix-0719')
  if (q.length > MAX_INPUT_LENGTH) { toast({ message: `输入内容过长（${q.length}/${MAX_INPUT_LENGTH}），请精简后重试`, type: 'error' }); return }
  const session = currentSession.value; if (!session) return
  session.messages.push({ id: crypto.randomUUID(), role: 'user', content: q, timestamp: Date.now() })
  if (session.name === '新对话') { session.name = q.slice(0, 30) }
  sessionDirty.value = true; inputMessage.value = ''; saveToStorage()
  nextTick(() => { const el = feedScrollRef.value as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight })
  nextTick(() => { const el = textareaRef.value; if (el) el.style.height = '' })
  agent.isChatting.value = true
  agent.setThinkingProgress('')
  // ★ Real Cognitive Stream: 实时进度
  unlistenProgress = await listen<{msg:string}>('agent:progress', (ev) => {
    if (!hasRoundThought) { agent.setThinkingProgress(ev.payload.msg) }
    scrollToBottomIfNear()
  })
  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', isThinking: true, toolCalls: [], timestamp: Date.now() }
  session.messages.push(aiMsg); const aiIdx = session.messages.length - 1
  // ★ 检索阶段固定文案：随机选一条，LLM 调用工具期间不再切换
  const presetText = pickRetrievalPhrase()
  aiMsg.content = presetText
  agent.setThinkingProgress(presetText)
  nextTick(() => { const el = feedScrollRef.value as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight })
  try {
    // ★ 工具轮结束：恢复预设文案，保持呼吸气泡
    unlistenRoundThought = await listen<{text:string}>('agent:round-thought', (ev) => {
      console.log('[FE] round-thought received')
      hasRoundThought = true
      const msg = session.messages[aiIdx]
      msg.isThinking = true
      if (ev.payload.text && ev.payload.text.trim()) { agent.setThinkingProgress(ev.payload.text.trim()) }
      else { agent.setThinkingProgress(pickRetrievalPhrase()) }
      scrollToBottomIfNear()
    })

    // ★ 监听 agent:done 事件（后台任务完成时触发）
    unlistenDone = await listen<any>('agent:done', (ev) => {
      console.log('[FE] done received')
      const result = ev.payload
      const msg = session.messages[aiIdx]; msg.isThinking = false
      if (result.success) {
        const finalAnswer = (result.answer || '').replace(/<refs>[\s\S]*<\/refs>/g, '').trim()
        agent.setThinkingProgress('')
        startTypewriter(msg, finalAnswer)
      } else { msg.error = result.answer || result.error || '失败' }
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenDone(); unlistenProgress(); unlistenRoundThought() } catch (e) { console.error('[KB] unlisten failed:', e) }
    })

    // ★ 监听 agent:error 事件
    unlistenError = await listen<any>('agent:error', (ev) => {
      const msg = session.messages[aiIdx]; msg.isThinking = false
      msg.error = ev.payload.error || 'AI 服务异常'
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenError(); unlistenDone(); unlistenProgress(); unlistenRoundThought() } catch {}
    })

    // ★ 调用命令（立即返回，不等待结果）
    invoke('kb_agent_chat', { data: { query: q, sessionId: currentSessionId.value } }).catch((err) => {
      console.error('[KB] invoke failed:', err)
      const msg = session.messages[aiIdx]; msg.isThinking = false
      msg.error = String(err)
      agent.isChatting.value = false; scrollToBottomIfNear()
    })
  } catch (e: any) {
    const msg = session.messages[aiIdx]; msg.isThinking = false
    msg.error = typeof e === 'string' ? e : (e.message || e.toString?.() || 'AI 服务异常')
    agent.isChatting.value = false; scrollToBottomIfNear()
  }
}

async function stopAgent() {
  try {
    const { agentCancel } = await import('@/api/db-client')
    await agentCancel()
    agent.isChatting.value = false
    // 更新最后一条 AI 消息
    const session = currentSession.value
    if (session) {
      const lastMsg = session.messages[session.messages.length - 1]
      if (lastMsg && lastMsg.role === 'assistant' && lastMsg.isThinking) {
        lastMsg.isThinking = false
        if (!lastMsg.content) lastMsg.content = '已停止执行。'
      }
    }
    scrollToBottomIfNear()
  } catch (e) {
    console.error('[KB] 停止失败:', e)
  }
}

// ─── Actions ────────────────────────────────────────────────────
async function copyAnswer(idx: number) {
  const m = currentSession.value?.messages[idx]
  if (!m?.content) return
  try { await navigator.clipboard.writeText(m.content); toast({ message: '已复制', type: 'success' }) } catch { toast({ message: '复制失败', type: 'error' }) }
}
function copyFullTrace(idx: number) {
  const session = currentSession.value; if (!session) return
  const aiMsg = session.messages[idx]; if (!aiMsg || aiMsg.role !== 'assistant') return
  // Get the previous user message
  let userQuery = ''
  for (let i = idx - 1; i >= 0; i--) { if (session.messages[i].role === 'user') { userQuery = session.messages[i].content; break } }
  // Build trace parts
  const parts: string[] = []
  parts.push('=== User Query ===')
  parts.push(userQuery || '(unknown)')
  parts.push('')
  // Tool calls
  if (aiMsg.toolCalls?.length) {
    parts.push('=== Tool Calls & Results ===')
    for (const round of aiMsg.toolCalls) {
      parts.push(`--- Round ${round.round} ---`)
      if (round.thought) parts.push(`Thought: ${round.thought}`)
      for (const tc of (round.tools || [])) {
        parts.push(`>>> ${tc.tool}`)
        parts.push(`Args: ${JSON.stringify(tc.args, null, 2)}`)
        if (tc.result) parts.push(`Result (${tc.result_len || tc.result.length} chars):\n${tc.result}`)
        parts.push('')
      }
    }
  }
  // Final answer
  if (aiMsg.content) {
    parts.push('=== Final Answer ===')
    parts.push(aiMsg.content)
  }
  const text = parts.join('\n')
  try { navigator.clipboard.writeText(text); toast({ message: '完整对话已复制', type: 'success' }) } catch { toast({ message: '复制失败', type: 'error' }) }
}
async function refreshAnswer(idx: number) {
  const m = currentSession.value?.messages[idx]
  if (!m?.content || isRefining.value) return; isRefining.value = true
  try { const r: any = await kbChat({ query: `请将以下标书应答内容润色优化：\n\n${m.content}` }); if (r.success) { m.content = r.answer || m.content; sessionDirty.value = true; saveToStorage() } }
  catch (e: any) { toast({ message: e.message || '润色失败', type: 'error' }) }
  finally { isRefining.value = false }
}

const notebookPickerOpen = ref(false); const notebookSearch = ref(''); const savingMsgId = ref<string|null>(null); const notebooks = ref<{id:string;name:string}[]>([])
const filteredNotebooks = computed(() => { const q = notebookSearch.value.trim().toLowerCase(); if (!q) return notebooks.value; return notebooks.value.filter(n => n.name.toLowerCase().includes(q)) })
async function saveToNotebook(nbId: string) {
  const msgId = savingMsgId.value; const html = msgId ? renderedContent(msgId) : ''
  if (!msgId || !html) return
  try { await notesV2Create({ title: 'AI 问答记录', content: html, isDaily: false, notebookId: nbId }); toast({ message: '已保存到笔记', type: 'success' }) }
  catch (e: any) { toast({ message: e.message || '保存失败', type: 'error' }) }
  finally { notebookPickerOpen.value = false; savingMsgId.value = null }
}
async function openNotebookPicker(msgId: string) { savingMsgId.value = msgId; notebookSearch.value = ''; try { const r: any = await fetchNotebooks(); notebooks.value = r?.notebooks || [] } catch { notebooks.value = [] }; notebookPickerOpen.value = true }

// ─── Init ───────────────────────────────────────────────────────
onUnmounted(() => {
  stopAllTypewriters()
  unlistenProgress?.()
  unlistenRoundThought?.()
  unlistenDone?.()
  unlistenError?.()
})
</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm" :style="{ fontFamily: 'Inter, system-ui, -apple-system, sans-serif' }">
  <!-- Sidebar -->
  <transition name="sidebar-slide">
    <ChatSidebar v-if="sidebarOpen"
      :sessions="sessions" :currentSessionId="currentSessionId"
      :editingSessionId="editingSessionId" :sessionNameEdit="sessionNameEdit"
      @select="selectSession" @new="newSession" @confirmDelete="confirmDeleteSession"
      @startRename="(id:string)=>{editingSessionId=id;const s=sessions.find(x=>x.id===id);if(s)sessionNameEdit=s.name}"
      @saveRename="(id:string)=>{const s=sessions.find(x=>x.id===id);if(s){s.name=sessionNameEdit.trim()||s.name;editingSessionId=null;sessionDirty.value=true;saveToStorage()}}"
      @updateSessionNameEdit="(val:string)=>sessionNameEdit=val"
      @cancelRename="editingSessionId=null" />
  </transition>

  <!-- Main Chat -->
  <div class="flex-1 flex flex-col min-w-0 relative bg-surface-50/30">
    <!-- Messages Feed -->
    <div ref="feedScrollRef" class="flex-1 overflow-y-auto px-6 py-6 space-y-5 scrollbar-thin" @scroll="onFeedScroll">
      <template v-if="currentSession">
        <!-- 加载更早消息提示 -->
        <div v-if="loadingMore" class="flex items-center justify-center py-2">
          <span class="text-[11px] text-surface-400">加载中...</span>
        </div>
        <div v-else-if="currentSession.messages.length > 0 && sessionHasMore[currentSession.id] === false" class="flex items-center justify-center py-2">
          <span class="text-[11px] text-surface-300">没有更早的消息了</span>
        </div>
        <!-- Empty State -->
        <div v-if="currentSession.messages.length === 0" class="flex flex-col items-center justify-center h-full text-center px-8 py-16">
          <div class="w-20 h-20 mb-6 rounded-3xl bg-gradient-to-br from-[#2563eb]/10 to-[#93c5fd]/10 flex items-center justify-center ring-1 ring-[#2563eb]/10">
            <svg class="w-10 h-10 text-[#2563eb]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>
          </div>
          <h2 class="text-[17px] font-semibold text-[#1d1d1f] mb-2">AI 知识助手</h2>
          <p class="text-[13px] text-[#86868b] max-w-md leading-relaxed">基于知识与记忆系统的AI智能助手，帮你检索、理解和推断项目、产品、文档等信息。</p>
        </div>

        <!-- Messages -->
        <div v-for="(msg, idx) in currentSession.messages" :key="msg.id || idx" class="flex gap-3 items-start animate-fade-in" :class="msg.role === 'user' ? 'flex-row-reverse' : ''">
          <div v-if="msg.role === 'user'" class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
          <div v-else class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
            <div class="min-w-0" :class="msg.role === 'user' ? 'max-w-[65%]' : (msg.isThinking ? '' : 'max-w-[85%]')">
            <div class="text-[10px] text-[#c7c7cc] mb-1.5 px-1" :class="msg.role === 'user' ? 'text-right' : ''">{{ msg.timestamp ? formatChatTime(msg.timestamp) : '' }}</div>
            <div v-if="msg.error" class="bg-red-50 border border-red-100 rounded-2xl px-5 py-4 text-[13px] text-red-600 rounded-tl-[4px]">{{ msg.error }}</div>
            <div v-else-if="msg.role === 'user'" class="bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-2">
              <pre class="text-[12px] leading-relaxed whitespace-pre-wrap font-sans">{{ msg.content }}</pre>
            </div>
            <div v-if="msg.role !== 'user' && (msg.isThinking || msg.content)">
              <div class="bg-white border border-[#e5e5ea] rounded-2xl rounded-tl-[4px] px-3 shadow-sm w-full" :class="{ 'thinking-bubble': msg.isThinking }">
                <div v-if="agent.isTypewriting(msg)" class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || msg.content || '思考中'"></div>
                <div v-else class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" :style="{ opacity: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || msg.content || '思考中') ? 1 : 0, transform: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || msg.content || '思考中') ? 'translateY(0)' : 'translateY(12px)', transition: 'opacity 1s ease, transform 1s ease' }" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || msg.content || '思考中'"></div>
              </div>
              <div v-if="!msg.isThinking && msg.content" class="flex items-center gap-1 px-1.5 py-1.5">
                <button @click="copyAnswer(idx)" class="w-6 h-6 flex items-center justify-center rounded-md text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5 transition-all" title="复制答案"><AppIcon name="copy" class="w-3 h-3" /></button>
                <button v-if="msg.workspace" @click="showWorkspaceIdx = showWorkspaceIdx === idx ? null : idx" class="w-6 h-6 flex items-center justify-center rounded-md transition-all" :class="showWorkspaceIdx === idx ? 'text-[#2563eb] bg-[#2563eb]/10' : 'text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5'" title="查看 Workspace">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 17V7m0 10a2 2 0 01-2-2V9a2 2 0 012-2h6a2 2 0 012 2v6a2 2 0 01-2 2m-6 0h6"/></svg>
                </button>
              </div>
              <div v-if="showWorkspaceIdx === idx && msg.workspace" class="mt-1 p-3 bg-[#f8f9fa] border border-[#e5e5ea] rounded-xl text-[12px] font-mono whitespace-pre-wrap leading-relaxed text-[#3a3a3c] max-h-[300px] overflow-y-auto">{{ msg.workspace }}</div>
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- Input Bar -->
    <div class="flex-shrink-0 px-4 pb-4 pt-2 bg-surface-50/30">
      <div class="max-w-3xl mx-auto">
        <p class="text-[10px] text-surface-300 text-right mb-1.5 pr-1">↵ 发送 · Shift+↵ 换行</p>
        <div class="flex items-end gap-2 bg-white rounded-2xl border border-surface-200 px-3 py-2 shadow-[0_2px_8px_rgba(0,0,0,0.06),0_0_0_1px_rgba(0,0,0,0.02)] transition-all duration-200 focus-within:border-brand-300 focus-within:shadow-[0_0_0_3px_rgba(37,99,235,0.1),0_4px_12px_rgba(0,0,0,0.08)]">
          <textarea ref="textareaRef" v-model="inputMessage" rows="1" maxlength="5000" placeholder="向 AI 提问..." class="flex-1 border-none resize-none outline-none text-[13px] text-surface-800 bg-transparent leading-relaxed placeholder:text-surface-400 max-h-[80px] py-1" @keydown.enter.exact.prevent="handleSendMessage" @input="(e: any) => { if (!inputMessage.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 80) + 'px' }" />
          <button v-if="agent.isChatting.value" @click="stopAgent" class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 bg-red-500 hover:bg-red-600 text-white transition-all duration-200" title="停止">
            <svg class="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
          </button>
          <button v-else @click="handleSendMessage" :disabled="!inputMessage.trim()" class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 transition-all duration-200" :class="inputMessage.trim() ? 'bg-brand-600 text-white hover:bg-brand-700 shadow-sm' : 'bg-surface-100 text-surface-400 cursor-not-allowed'" title="发送">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>
</div>

<!-- Modals -->
<Teleport to="body">
  <Transition name="modal-fade">
    <div v-if="notebookPickerOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm" @click.self="notebookPickerOpen = false">
      <div class="bg-white rounded-2xl w-[360px] max-h-[70vh] overflow-hidden shadow-xl flex flex-col border border-[#e5e5ea]">
        <div class="flex items-center justify-between px-5 py-4 border-b border-[#e5e5ea]">
          <h3 class="text-[13px] font-semibold text-[#1d1d1f]">保存到笔记本</h3>
          <button @click="notebookPickerOpen = false" class="w-7 h-7 flex items-center justify-center rounded-lg text-[#86868b] hover:text-[#2563eb]">✕</button>
        </div>
        <div class="px-4 py-2"><input v-model="notebookSearch" class="w-full border border-[#e5e5ea] rounded-lg px-3 py-2 text-[12px] outline-none focus:border-[#2563eb]/40" placeholder="搜索笔记本..." /></div>
        <div class="flex-1 overflow-y-auto px-4 py-2 max-h-[300px] space-y-1">
          <button v-for="nb in filteredNotebooks" :key="nb.id" @click="saveToNotebook(nb.id)" class="w-full flex items-center gap-2 px-3 py-2.5 text-[12px] text-left rounded-xl transition-colors text-[#3a3a3c] hover:bg-[#f0f0f5]">
            <svg class="w-4 h-4 text-[#86868b] flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
            <span class="truncate">{{ nb.name }}</span>
          </button>
          <div v-if="filteredNotebooks.length === 0" class="text-[12px] text-[#86868b] text-center py-6">无匹配笔记本</div>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

</template>

<style scoped>
@keyframes bounce { 0%,80%,100%{transform:scale(0.6);opacity:0.4} 40%{transform:scale(1);opacity:1} }
@keyframes recall { 0%,100%{opacity:0.2;transform:scale(0.8)} 50%{opacity:1;transform:scale(1.2)} }
@keyframes thinking-shimmer {
  0%   { background-position: 200% center; }
  100% { background-position: -200% center; }
}
.animate-bounce { animation: bounce 1.2s infinite ease-in-out; }
.animate-fade-in { animation: fadeIn 0.3s ease-out; }
@keyframes fadeIn { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
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
.modal-fade-enter-active,.modal-fade-leave-active { transition: opacity 0.2s ease; }
.modal-fade-enter-from,.modal-fade-leave-to { opacity: 0; }
.sidebar-slide-enter-active,.sidebar-slide-leave-active { transition: width 0.2s ease, opacity 0.2s ease; overflow: hidden; }
.sidebar-slide-enter-from,.sidebar-slide-leave-to { width: 0 !important; opacity: 0; padding: 0; margin: 0; }
.drawer-slide-enter-active,.drawer-slide-leave-active { transition: width 0.2s ease, opacity 0.2s ease; overflow: hidden; }
.drawer-slide-enter-from,.drawer-slide-leave-to { width: 0 !important; opacity: 0; }
.line-clamp-4 { display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
</style>

<style>
/* macOS-style thin scrollbar */
.scrollbar-thin::-webkit-scrollbar { width: 4px; }
.scrollbar-thin::-webkit-scrollbar-track { background: transparent; }
.scrollbar-thin::-webkit-scrollbar-thumb { background: #e5e5ea; border-radius: 4px; }
.scrollbar-thin::-webkit-scrollbar-thumb:hover { background: #c7c7cc; }
.scrollbar-thin { scrollbar-width: thin; scrollbar-color: #e5e5ea transparent; }
</style>
