<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onUnmounted, nextTick, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import ChatSidebar from '@/components/chat/ChatSidebar.vue'
import ToolCallCard from '@/components/chat/ToolCallCard.vue'
import { toast } from '@/composables/useToast'
import { kbAgentChat, kbChat, notesV2Create, fetchNotebooks, kbSaveSessions, kbLoadSessions, kbDeleteSession } from '@/api/db-client'
import { sanitizeHtml, sanitizeHtmlSync } from '@/utils/sanitize'
import { listen } from '@tauri-apps/api/event'

// ─── Markdown renderer ──────────────────────────────────────────
async function renderMarkdown(text: string): Promise<string> {
  if (!text) return ''
  try {
    const mod = await import('marked')
    const Marked = mod.Marked || (mod as any).default?.Marked || (mod as any).default
    let html = ''
    if (Marked && typeof Marked === 'function') { const marked = new Marked({ gfm: true, breaks: true }); html = marked.parse(text) || '' }
    else { const { marked } = mod as any; html = (marked?.parse || marked)(text) || '' }
    return sanitizeHtml(html)
  } catch { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;') }
}
// ─── Types ──────────────────────────────────────────────────────
interface ChatMessage { role: 'user'|'assistant'; content: string; error?: string; toolCalls?: any[]; rawToolCalls?: any[]; toolResults?: any[]; isThinking?: boolean; contextDocs?: any[]; workspace?: string; typewriterText?: string; typewriterDone?: boolean }
interface ChatSession { id: string; name: string; messages: ChatMessage[]; createdAt: string }

const rendered = ref<Record<number,string>>({})
async function renderMessageContent(idx: number) {
  const msgs = currentSession.value?.messages; if (!msgs || !msgs[idx]) return ''
  if (rendered.value[idx] !== undefined) return rendered.value[idx]
  const text = msgs[idx].content || ''; if (!text) return ''
  const safeText = text.replace(/~/g, '～')
  rendered.value[idx] = await renderMarkdown(safeText)
}
function renderedContent(idx: number): string { return rendered.value[idx] || '' }

// ─── Typewriter ─────────────────────────────────────────────────
const activeTypewriters = new Set<ChatMessage>()
const typewriterTimers = new Map<ChatMessage, ReturnType<typeof setInterval>>()
function startTypewriter(msg: ChatMessage, fullText: string, speed = 8) {
  stopTypewriter(msg)
  if (!fullText) { msg.typewriterText = ''; msg.typewriterDone = true; return }
  const idx = currentSession.value?.messages.indexOf(msg) ?? -1
  if (idx >= 0) renderMarkdown(fullText.replace(/~/g, '～')).then(html => { rendered.value[idx] = html })
  const TYPEWRITER_CHUNK_SIZE = 5
  let pos = 0
  msg.typewriterText = fullText.slice(0, pos)
  msg.typewriterDone = false
  activeTypewriters.add(msg)
  const timer = setInterval(() => {
    pos += TYPEWRITER_CHUNK_SIZE
    if (pos >= fullText.length) {
      pos = fullText.length; clearInterval(timer); typewriterTimers.delete(msg); activeTypewriters.delete(msg)
      msg.typewriterText = fullText; msg.typewriterDone = true; return
    }
    msg.typewriterText = fullText.slice(0, pos)
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
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s:any) => ({ ...s, name: s.title || '新对话', updatedAt: s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch (e) { console.error('[KB] loadSessions failed:', e) }
  if (!sessions.value.length) { newSession() }
  nextTick(() => scrollToBottomIfNear())
})
// ★ 每次切换回来时确保到底
onActivated(() => { nextTick(() => scrollToBottomIfNear()) })
async function selectSession(id: string) {
  currentSessionId.value = id; rendered.value = {}
  stopAllTypewriters()
  // Trigger markdown rendering for all AI messages — await all
  const session = sessions.value.find(s => s.id === id)
  if (session) {
    await Promise.all(session.messages.map((m, i) => {
      if (m.role === 'assistant' && m.content && !m.isThinking) {
        return renderMessageContent(i)
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
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s: any) => ({ ...s, name: s.title || '新会话', updatedAt: s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt })).sort((a: any, b: any) => new Date(b.updatedAt || a.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch { sessions.value = [] }
  if (!sessions.value.length) { newSession(); return }
  currentSessionId.value = sessions.value[0].id
  // 初始化内容快照
  const s = currentSession.value
  if (s) lastSavedContent = JSON.stringify(s.messages)
  // Render markdown for loaded AI messages — await all renders
  const msgs = sessions.value[0].messages
  await Promise.all(msgs.map((m, i) => {
    if (m.role === 'assistant' && m.content) {
      return renderMessageContent(i)
    }
    return Promise.resolve()
  }))
  scrollToBottomIfNear()
}

// ─── Chat state ─────────────────────────────────────────────────
const inputMessage = ref('')
const isChatting = ref(false)
const thinkingProgress = ref('')
const isDrawerOpen = ref(false)
const showWorkspaceIdx = ref<number | null>(null)
const feedScrollRef = ref<HTMLElement|null>(null)
const isRefining = ref(false)
const sidebarOpen = ref(true)
const confirmDeleteId = ref<string|null>(null)

function confirmDeleteSession(id: string) { confirmDeleteId.value = id }
function executeDelete() {
  const id = confirmDeleteId.value; if (!id) return
  kbDeleteSession(id).catch(() => {})
  sessions.value = sessions.value.filter(s => s.id !== id)
  if (currentSessionId.value === id) currentSessionId.value = sessions.value[0]?.id ?? null
  confirmDeleteId.value = null
  sessionDirty.value = true; saveToStorage()
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

// ─── Suggestion prompts ─────────────────────────────────────────
const suggestions = ['分析一下这个项目的关键信息', '请帮我写一份投标应答', '搜索关于S12700E的参数信息', '最近项目有什么进展']

// ─── Send message ───────────────────────────────────────────────
let thoughtTimer: any = null
async function handleSendMessage() {
  const q = inputMessage.value.trim()
  if (!q || isChatting.value) return
  const session = currentSession.value; if (!session) return
  session.messages.push({ role: 'user', content: q })
  if (session.name === '新对话') { session.name = q.slice(0, 30) }
  sessionDirty.value = true; inputMessage.value = ''; saveToStorage(); scrollToBottomIfNear()
  isChatting.value = true
  thinkingProgress.value = ''
  const aiMsg = { role: 'assistant' as const, content: '', isThinking: true }
  session.messages.push(aiMsg)
  const aiIdx = session.messages.length - 1
  // ★ 冷启动障眼法：立即显示拟人化短句
  const thinkingPhrases = ['让我想想啊……', '嗯……我回忆一下。', '别急，我翻一下脑子里的记录。', '稍等，我查查看……', '嗯，我想想……']
  const randomPhrase = thinkingPhrases[Math.floor(Math.random() * thinkingPhrases.length)]
  aiMsg.content = randomPhrase; scrollToBottomIfNear()
  // ★ Real Cognitive Stream: 实时进度
  const unlistenProgress = await listen<{msg:string}>('agent:progress', (ev) => {
    thinkingProgress.value = ev.payload.msg; scrollToBottomIfNear()
  })
  let unlistenStream: (() => void) | null = null
  let unlistenThought: (() => void) | null = null
  let unlistenStatus: (() => void) | null = null
  try {
    // ★ 监听 status 事件，动态演变拟人化文案
    const toolHints: Record<string, string> = {
      ext_tool_search: '我正在检索相关资料……',
      ext_tool_object: '我在翻看记忆中的对象……',
      ext_tool_traverse: '我在继续往后翻看……',
      ext_tool_batch_search: '我在同时查几个方向……',
    }
    unlistenStatus = await listen<{ stage: string; tool?: string; target?: string }>('agent:status', (ev) => {
      const { stage, tool, target } = ev.payload
      if (stage === 'tool_start' && tool) {
        const hint = toolHints[tool] || `正在调用 ${tool}……`
        const targetHint = target ? `「${target.slice(0, 20)}」` : ''
        session.messages[aiIdx].content = `${hint}${targetHint ? ' ' + targetHint : ''}`
        scrollToBottomIfNear()
      }
    })
    // ★ 流式监听：实时显示 LLM thought + 最终答案
    const streamedContent = ref('')
    unlistenStream = await listen<{token:string}>('agent:token', (ev) => { streamedContent.value += ev.payload.token; scrollToBottomIfNear() })
    unlistenThought = await listen<{text:string}>('agent:thought', (ev) => { let idx=0; const full=ev.payload.text; thinkingProgress.value=''; const t=setInterval(()=>{thinkingProgress.value=full.slice(0,++idx);scrollToBottomIfNear();if(idx>=full.length)clearInterval(t)},30); thoughtTimer=t as any })
    const history: any[] = []
    for (const m of session.messages) {
      if (m === aiMsg) continue
      if (m.role === 'user') history.push({ role: 'user', content: m.content })
      else if (m.role === 'assistant') {
        const entry: any = { role: 'assistant', content: m.content || '' }
        if (m.rawToolCalls?.length) entry.tool_calls = m.rawToolCalls
        history.push(entry)
        if (m.toolResults) { for (const tr of m.toolResults) history.push({ role: 'tool', content: tr.content || '', tool_call_id: tr.id, name: tr.name }) }
      }
    }
    const result: any = await kbAgentChat({ query: q, history })
    const msg = session.messages[aiIdx]; msg.isThinking = false
    if (result.success) {
      msg.content = (result.answer || '').replace(/<refs>[\s\S]*<\/refs>/g, '').trim()
      thinkingProgress.value = ''
      if (streamedContent.value) { msg.content = streamedContent.value.replace(/<refs>[\s\S]*<\/refs>/g, '').trim(); streamedContent.value = '' }
      msg.toolCalls = result.tool_log || []
      startTypewriter(msg, msg.content)
      const rawCalls: any[] = []; const toolResults: any[] = []
      for (const r of (result.tool_log || [])) {
        for (const t of (r.tools || [])) {
          const callId = 'call_' + Math.random().toString(36).slice(2, 8)
          rawCalls.push({ id: callId, type: 'function', function: { name: t.tool, arguments: typeof t.args === 'string' ? t.args : JSON.stringify(t.args) } })
          const TOOL_RESULT_MAX_LENGTH = 30000
          toolResults.push({ id: callId, name: t.tool, content: typeof t.result === 'string' ? t.result.substring(0, TOOL_RESULT_MAX_LENGTH) : '' })
        }
      }
      msg.rawToolCalls = rawCalls; msg.toolResults = toolResults; msg.contextDocs = result.context_docs || result.sources || [];
      msg.workspace = result.workspace || '';
      renderMessageContent(aiIdx)
    } else { msg.error = result.answer || result.error || '失败' }
  } catch (e: any) {
    const msg = session.messages[aiIdx]; msg.isThinking = false
    msg.error = typeof e === 'string' ? e : (e.message || e.toString?.() || 'AI 服务异常')
  } finally {
    isChatting.value = false; sessionDirty.value = true; saveToStorage(); scrollToBottomIfNear()
    try { ;unlistenStream(); unlistenThought(); unlistenStatus?.(); unlistenProgress() } catch (e) { console.error('[KB] unlisten failed:', e) }
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

const notebookPickerOpen = ref(false); const notebookSearch = ref(''); const savingMsgIdx = ref<number|null>(null); const notebooks = ref<{id:string;name:string}[]>([])
const filteredNotebooks = computed(() => { const q = notebookSearch.value.trim().toLowerCase(); if (!q) return notebooks.value; return notebooks.value.filter(n => n.name.toLowerCase().includes(q)) })
async function saveToNotebook(nbId: string) {
  const msgIdx = savingMsgIdx.value; const html = renderedContent(msgIdx ?? -1)
  if (msgIdx == null || !html) return
  try { await notesV2Create({ title: 'AI 问答记录', content: html, isDaily: false, notebookId: nbId }); toast({ message: '已保存到笔记', type: 'success' }) }
  catch (e: any) { toast({ message: e.message || '保存失败', type: 'error' }) }
  finally { notebookPickerOpen.value = false; savingMsgIdx.value = null }
}
async function openNotebookPicker(idx: number) { savingMsgIdx.value = idx; notebookSearch.value = ''; try { const r: any = await fetchNotebooks(); notebooks.value = r?.notebooks || [] } catch { notebooks.value = [] }; notebookPickerOpen.value = true }

// ─── Init ───────────────────────────────────────────────────────
onMounted(loadFromStorage)
onUnmounted(() => {
  saveToStorage()
  stopAllTypewriters()
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
    <!-- Top Bar -->
    <div class="h-10 border-b border-surface-200 bg-white flex items-center justify-between px-4 flex-shrink-0">
      <div class="flex items-center gap-2">
        <button @click="sidebarOpen = !sidebarOpen" class="w-6 h-6 flex items-center justify-center rounded-lg text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-all" title="切换侧栏">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M3.75 6.75h16.5M3.75 12h16.5m-16.5 5.25h16.5"/></svg>
        </button>
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">{{ currentSession?.name || '售前 AI 工作台' }}</span>
      </div>
    </div>

    <!-- Messages Feed -->
    <div ref="feedScrollRef" class="flex-1 overflow-y-auto px-6 py-6 space-y-5 scrollbar-thin pb-28">
      <template v-if="currentSession">
        <!-- Empty State -->
        <div v-if="currentSession.messages.length === 0" class="flex flex-col items-center justify-center h-full text-center px-8 py-16">
          <div class="w-20 h-20 mb-6 rounded-3xl bg-gradient-to-br from-[#2563eb]/10 to-[#93c5fd]/10 flex items-center justify-center ring-1 ring-[#2563eb]/10">
            <svg class="w-10 h-10 text-[#2563eb]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>
          </div>
          <h2 class="text-[17px] font-semibold text-[#1d1d1f] mb-2">售前 AI 知识工作台</h2>
          <p class="text-[13px] text-[#86868b] max-w-md mb-8 leading-relaxed">输入售前疑问或技术参数需求，AI 将结合知识库生成专业的应答建议。</p>
          <div class="grid grid-cols-2 gap-3 max-w-lg w-full">
            <div v-for="(s, si) in suggestions" :key="si" @click="inputMessage = s; handleSendMessage()"
              class="bg-white border border-[#e5e5ea] rounded-2xl px-4 py-3.5 text-left cursor-pointer hover:border-[#2563eb]/30 hover:shadow-sm transition-all duration-200">
              <div class="text-[11px] text-[#2563eb] font-medium mb-1">建议问题 {{ si + 1 }}</div>
              <div class="text-[12px] text-[#3a3a3c] leading-relaxed">{{ s }}</div>
            </div>
          </div>
        </div>

        <!-- Messages -->
        <div v-for="(msg, idx) in currentSession.messages" :key="idx" class="flex gap-3 items-start animate-fade-in" :class="msg.role === 'user' ? 'flex-row-reverse' : ''">
          <div v-if="msg.role === 'user'" class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
          <div v-else class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
          <div class="min-w-0" :class="msg.role === 'user' ? 'max-w-[65%]' : 'max-w-[75%]'">
            <div class="text-[10px] text-[#c7c7cc] mb-1.5 px-1" :class="msg.role === 'user' ? 'text-right' : ''">{{ new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }) }}</div>
            <div v-if="msg.isThinking && (!msg.blocks || msg.blocks.length === 0)">
              <div class="bg-white border border-[#e5e5ea] rounded-2xl px-3 py-2 shadow-sm rounded-tl-[4px]">
                <div class="flex items-center gap-2">
                  <span class="text-[12px] text-[#3a3a3c] transition-colors duration-500">{{ thinkingProgress || '思考中' }}</span>
                  <div class="flex gap-1">
                    <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:0ms"/>
                    <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:150ms"/>
                    <span class="w-1.5 h-1.5 bg-[#2563eb] rounded-full animate-bounce" style="animation-delay:300ms"/>
                  </div>
                </div>
              </div>
            </div>
            <div v-else-if="msg.isThinking && msg.blocks && msg.blocks.length > 0" class="space-y-1.5 mb-2">
              <div v-for="(block, bi) in msg.blocks" :key="bi" class="text-[12px] text-surface-600 bg-surface-50 px-3 py-1.5 rounded-lg border-l-2 border-surface-200 italic">{{ block.content }}</div>
            </div>
            <div v-else-if="msg.error" class="bg-red-50 border border-red-100 rounded-2xl px-5 py-4 text-[13px] text-red-600 rounded-tl-[4px]">{{ msg.error }}</div>
            <div v-else-if="msg.content" :class="msg.role === 'user' ? 'bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-2' : 'bg-white border border-[#e5e5ea] rounded-2xl rounded-tl-[4px] px-3 py-2 shadow-sm'">
              <template v-if="msg.role === 'user'">
                <pre class="text-[12px] leading-relaxed whitespace-pre-wrap font-sans">{{ msg.content }}</pre>
              </template>
              <template v-else>
                <div v-if="isTypewriting(msg)" class="text-[12px] text-[#3a3a3c] leading-relaxed whitespace-pre-wrap">
                  {{ typewriterText(msg) }}<span class="inline-block w-[2px] h-[1em] bg-[#2563eb] ml-0.5 align-middle animate-pulse">▌</span>
                </div>
                <div v-else class="text-[13px] text-[#3a3a3c] leading-relaxed markdown-body" :style="{ opacity: renderedContent(idx) ? 1 : 0, transform: renderedContent(idx) ? 'translateY(0)' : 'translateY(12px)', transition: 'opacity 1s ease, transform 1s ease' }" v-html="renderedContent(idx)"></div>
                <div class="flex items-center gap-1 mt-3 pt-3 border-t border-[#e5e5ea]">
                  <button @click="copyAnswer(idx)" class="w-7 h-7 flex items-center justify-center rounded-lg text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5 transition-all" title="复制答案"><AppIcon name="copy" class="w-3.5 h-3.5" /></button>
                  <button @click="copyFullTrace(idx)" class="w-7 h-7 flex items-center justify-center rounded-lg text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5 transition-all" title="复制完整对话">
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>
                  </button>
                  <button v-if="msg.workspace" @click="showWorkspaceIdx = showWorkspaceIdx === idx ? null : idx" class="w-7 h-7 flex items-center justify-center rounded-lg transition-all" :class="showWorkspaceIdx === idx ? 'text-[#2563eb] bg-[#2563eb]/10' : 'text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5'" title="查看 Workspace">
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 17V7m0 10a2 2 0 01-2-2V9a2 2 0 012-2h6a2 2 0 012 2v6a2 2 0 01-2 2m-6 0h6"/></svg>
                  </button>
                  <button @click="openNotebookPicker(idx)" class="w-7 h-7 flex items-center justify-center rounded-lg text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5 transition-all" title="保存到笔记"><AppIcon name="file-plus" class="w-3.5 h-3.5" /></button>
                </div>
                <div v-if="showWorkspaceIdx === idx && msg.workspace" class="mt-2 p-3 bg-[#f8f9fa] border border-[#e5e5ea] rounded-xl text-[12px] font-mono whitespace-pre-wrap leading-relaxed text-[#3a3a3c] max-h-[300px] overflow-y-auto">{{ msg.workspace }}</div>
              </template>
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- Input Bar (floating over messages) -->
    <div class="absolute bottom-0 left-0 right-0 z-10 px-4 pb-4 pt-10 bg-gradient-to-b from-transparent via-white/50 to-white">
      <div class="max-w-3xl mx-auto">
        <p class="text-[10px] text-surface-300 text-left mb-1.5 pl-1">↵ 发送 · Shift+↵ 换行</p>
        <div class="flex items-center gap-2 bg-white rounded-2xl border border-surface-200 px-4 py-2.5 shadow-[0_2px_8px_rgba(0,0,0,0.06),0_0_0_1px_rgba(0,0,0,0.02)] transition-all duration-200 focus-within:border-brand-300 focus-within:shadow-[0_0_0_3px_rgba(37,99,235,0.1),0_4px_12px_rgba(0,0,0,0.08)]">
          <textarea v-model="inputMessage" rows="1" placeholder="向 AI 提问..." class="flex-1 border-none resize-none outline-none text-[13px] text-surface-800 bg-transparent leading-relaxed placeholder:text-surface-400 max-h-[120px]" @keydown.enter.exact.prevent="handleSendMessage" @input="(e: any) => { e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 120) + 'px' }" />
          <button @click="handleSendMessage" :disabled="!inputMessage.trim() || isChatting" class="w-8 h-8 rounded-xl flex items-center justify-center flex-shrink-0 transition-all duration-200" :class="inputMessage.trim() && !isChatting ? 'bg-brand-600 text-white hover:bg-brand-700 shadow-sm' : 'bg-surface-100 text-surface-400 cursor-not-allowed'" title="发送">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>
</div>

<!-- Modals -->
<Teleport to="body">
  <Transition name="modal-fade">
    <div v-if="confirmDeleteId" class="fixed inset-0 z-[90] flex items-center justify-center" @click.self="confirmDeleteId = null">
      <div class="absolute inset-0 bg-black/20" />
      <div class="relative bg-white rounded-xl shadow-lg border border-surface-200 w-full max-w-sm mx-4 p-6 z-10">
        <h3 class="text-[13px] font-semibold text-surface-800 mb-2">删除会话</h3>
        <p class="text-[13px] text-surface-500 leading-relaxed mb-5">确定要删除该会话吗？此操作不可撤销。</p>
        <div class="flex items-center justify-end gap-2">
          <button @click="confirmDeleteId = null" class="flex items-center gap-1.5 py-1 px-2 rounded text-[12px] text-surface-400 bg-transparent border-none hover:text-surface-600 hover:bg-surface-50 transition-colors">取消</button>
          <button @click="executeDelete" class="flex items-center gap-1.5 py-1 px-3 rounded text-[12px] text-white bg-[#2563eb] hover:bg-[#1d4ed8] transition-colors">删除</button>
        </div>
      </div>
    </div>
  </Transition>
</Teleport>

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

  <!-- Notebook Picker Modal -->
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
.animate-bounce { animation: bounce 1.2s infinite ease-in-out; }
.animate-fade-in { animation: fadeIn 0.3s ease-out; }
@keyframes fadeIn { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
.modal-fade-enter-active,.modal-fade-leave-active { transition: opacity 0.2s ease; }
.modal-fade-enter-from,.modal-fade-leave-to { opacity: 0; }
.sidebar-slide-enter-active,.sidebar-slide-leave-active { transition: width 0.2s ease, opacity 0.2s ease; overflow: hidden; }
.sidebar-slide-enter-from,.sidebar-slide-leave-to { width: 0 !important; opacity: 0; padding: 0; margin: 0; }
.drawer-slide-enter-active,.drawer-slide-leave-active { transition: width 0.2s ease, opacity 0.2s ease; overflow: hidden; }
.drawer-slide-enter-from,.drawer-slide-leave-to { width: 0 !important; opacity: 0; }
.line-clamp-4 { display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
</style>

<style>
.markdown-body { font-size: 13px; color: #3a3a3c; line-height: 1.75; }
.markdown-body h1 { font-size: 1.1rem; font-weight: 700; margin: 1rem 0 0.5rem; color: #1d1d1f; }
.markdown-body h2 { font-size: 1rem; font-weight: 600; margin: 0.9rem 0 0.4rem; color: #1d1d1f; }
.markdown-body h3 { font-size: 0.9rem; font-weight: 600; margin: 0.8rem 0 0.3rem; color: #3a3a3c; }
.markdown-body p { margin: 0.5rem 0; }
.markdown-body ul,.markdown-body ol { margin: 0.4rem 0; padding-left: 1.5rem; }
.markdown-body strong { font-weight: 600; color: #1d1d1f; }
.markdown-body code { background: rgba(0,0,0,0.04); color: #e11d48; padding: 2px 6px; border-radius: 4px; font-size: 0.9em; font-family: 'SF Mono', monospace; }
.markdown-body pre { background: #1d1d1f; color: #f5f5f7; padding: 14px 18px; border-radius: 10px; overflow-x: auto; margin: 0.8rem 0; font-size: 0.8rem; }
.markdown-body pre code { background: none; color: inherit; padding: 0; }
.markdown-body table { width: 100%; border-collapse: collapse; margin: 16px 0; font-size: 12px; }
.markdown-body th { background: #f5f5f7; color: #1d1d1f; font-weight: 600; padding: 10px 14px; text-align: left; border-bottom: 2px solid rgba(0,0,0,0.06); }
.markdown-body td { padding: 12px 14px; border-bottom: 1px solid rgba(0,0,0,0.04); color: #3a3a3c; }
.markdown-body blockquote { border-left: 3px solid #2563eb; padding: 6px 14px; margin: 0.6rem 0; color: #6e6e73; background: rgba(107,78,255,0.04); border-radius: 0 6px 6px 0; }
.markdown-body a { color: #2563eb; }

/* macOS-style thin scrollbar */
.scrollbar-thin::-webkit-scrollbar { width: 4px; }
.scrollbar-thin::-webkit-scrollbar-track { background: transparent; }
.scrollbar-thin::-webkit-scrollbar-thumb { background: #e5e5ea; border-radius: 4px; }
.scrollbar-thin::-webkit-scrollbar-thumb:hover { background: #c7c7cc; }
.scrollbar-thin { scrollbar-width: thin; scrollbar-color: #e5e5ea transparent; }
</style>
