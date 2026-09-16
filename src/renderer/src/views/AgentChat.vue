<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onDeactivated, onUnmounted, nextTick, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import ChatSidebar from '@/components/chat/ChatSidebar.vue'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import { kbAgentChat, kbChat, notesV2Create, fetchNotebooks, kbSaveSessions, kbLoadSessions, kbLoadSessionMessages, kbDeleteSession } from '@/api/db-client'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { useAgentChat, type ChatMessage as AgentChatMessage } from '@/composables/useAgentChat'
import { pickRetrievalPhrase } from '@/utils/chat'
import { ProjectCard, TodoCard, ObjectList } from '@/components/renderer'

const agent = useAgentChat()

// ─── Attachments ─────────────────────────────────────────────────
const IMG_EXTS = new Set(['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'])
const DOC_EXTS = new Set(['pdf', 'doc', 'docx', 'xlsx', 'xls', 'pptx', 'ppt', 'txt', 'md', 'csv', 'json'])
const ALL_EXTS = new Set([...IMG_EXTS, ...DOC_EXTS])
const MAX_ATTACH = 5
const MAX_SIZE = 20 * 1024 * 1024

interface PendingAttachment { path: string; name: string; kind: 'image' | 'doc'; previewUrl?: string }
const pendingAttachments = ref<PendingAttachment[]>([])

function extOf(name: string): string { const i = name.lastIndexOf('.'); return i > 0 ? name.slice(i + 1).toLowerCase() : '' }

async function openAttachmentDialog() {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const result = await open({ multiple: true, canCreateDirectories: false, filters: [{ name: '文档与图片', extensions: [...ALL_EXTS] }] })
  if (!result) return
  const paths: string[] = Array.isArray(result) ? result : [result as string]
  for (const p of paths) {
    if (pendingAttachments.value.length >= MAX_ATTACH) { toast({ message: `最多附加 ${MAX_ATTACH} 个文件`, type: 'error' }); break }
    const name = p.replace(/^.*[\\\/]/, '')
    const ext = extOf(name)
    if (!ALL_EXTS.has(ext)) { toast({ message: `不支持的文件类型: ${ext}`, type: 'error' }); continue }
    if (pendingAttachments.value.some(a => a.path === p)) continue
    const kind: 'image' | 'doc' = IMG_EXTS.has(ext) ? 'image' : 'doc'
    pendingAttachments.value.push({ path: p, name, kind })
  }
}

function removeAttachment(idx: number) { pendingAttachments.value.splice(idx, 1) }

async function onPaste(e: ClipboardEvent) {
  // 优先从 clipboardData.files 获取（WebView2 中比 items 更可靠）
  const files = e.clipboardData?.files
  if (files && files.length > 0) {
    e.preventDefault()
    let added = 0
    for (const file of Array.from(files)) {
      if (pendingAttachments.value.length >= MAX_ATTACH) { toast({ message: `最多附加 ${MAX_ATTACH} 个文件`, type: 'error' }); break }
      const ext = extOf(file.name) || (file.type.split('/')[1] === 'jpeg' ? 'jpg' : file.type.split('/')[1] || 'bin')
      if (!ALL_EXTS.has(ext) && !IMG_EXTS.has(ext)) { continue }
      try {
        const buf = await file.arrayBuffer()
        const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: file.name || null })
        if (r?.success && r.path) {
          const kind: 'image' | 'doc' = IMG_EXTS.has(ext) ? 'image' : 'doc'
          const previewUrl = kind === 'image' ? URL.createObjectURL(new Blob([buf], { type: file.type })) : undefined
          pendingAttachments.value.push({ path: r.path, name: r.name || file.name, kind, previewUrl })
          added++
        }
      } catch (err) { console.warn('[Chat] paste file save failed:', err) }
    }
    if (added > 0) toast({ message: `已粘贴 ${added} 个文件`, type: 'success' })
    else toast({ message: '粘贴失败：不支持的文件类型', type: 'error' })
    return
  }
  // 回退：从 clipboardData.items 获取图片
  const items = e.clipboardData?.items
  if (!items) return
  for (const item of items) {
    if (!item.type.startsWith('image/')) continue
    if (pendingAttachments.value.length >= MAX_ATTACH) { toast({ message: `最多附加 ${MAX_ATTACH} 个文件`, type: 'error' }); break }
    e.preventDefault()
    const file = item.getAsFile()
    if (!file) continue
    try {
      const buf = await file.arrayBuffer()
      const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : item.type.split('/')[1]
      const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: null })
      if (r?.success && r.path) {
        const blobUrl = URL.createObjectURL(new Blob([buf], { type: item.type }))
        pendingAttachments.value.push({ path: r.path, name: r.name || `pasted.${ext}`, kind: 'image', previewUrl: blobUrl })
        toast({ message: '已粘贴图片', type: 'success' })
      }
    } catch (err) { console.warn('[Chat] paste image save failed:', err); toast({ message: '粘贴图片失败', type: 'error' }) }
  }
  // 系统剪贴板中的文件不暴露给 clipboardData（浏览器安全限制），
  // 若未添加任何附件，回退从后端读取剪贴板文件列表
  if (pendingAttachments.value.length < MAX_ATTACH) {
    try {
      const r: any = await invoke('chat_read_clipboard_files')
      if (r?.success && Array.isArray(r.files) && r.files.length) {
        let addedMac = 0
        for (const f of r.files) {
          if (pendingAttachments.value.length >= MAX_ATTACH) break
          pendingAttachments.value.push({ path: f.path, name: f.name, kind: f.kind || 'doc' })
          addedMac++
        }
        if (addedMac > 0) toast({ message: `已粘贴 ${addedMac} 个文件`, type: 'success' })
      }
    } catch (err) { console.warn('[Chat] clipboard read failed:', err) }
  }
}

function extractAttachmentNames(content: string): string[] {
  const names: string[] = []
  const re = /【附件：(.+?)】/g
  let m
  while ((m = re.exec(content)) !== null) names.push(m[1])
  return names
}

function displayContent(content: string): string {
  const i = content.indexOf('<<<ATTACHMENTS>>>')
  return i >= 0 ? content.slice(0, i).trimEnd() : content
}

// ─── Types ──────────────────────────────────────────────────────
interface ChatMessage extends AgentChatMessage { id?: string; role: 'user'|'assistant'; toolCalls?: any[]; rawToolCalls?: any[]; toolResults?: any[]; contextDocs?: any[]; workspace?: string; attachments?: Array<{path:string;name:string;kind:'image'|'doc'}> }
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
  const version = (renderVersion.get(msgId) || 0) + 1
  renderVersion.set(msgId, version)
  const safeText = text.replace(/~/g, '～')
  const html = await agent.renderMd(safeText)
  if (renderVersion.get(msgId) === version) {
    agent.rendered.value[msgId] = html
  }
}
function renderedContent(msgId: string): string { return agent.rendered.value[msgId] || '' }

// 版本号防竞态：防止旧渲染覆盖新渲染
const renderVersion = new Map<string, number>()

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
  // 恢复输入框文本
  try {
    const savedInput = localStorage.getItem('chat_input_backup')
    if (savedInput) { inputMessage.value = savedInput; localStorage.removeItem('chat_input_backup') }
    const savedAttachments = localStorage.getItem('chat_attachments_backup')
    if (savedAttachments) { pendingAttachments.value = JSON.parse(savedAttachments); localStorage.removeItem('chat_attachments_backup') }
  } catch {}
  
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s:any) => ({ ...s, name: s.title || '新对话', updatedAt: s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt, messages: (s.messages || []).map((m: any) => { const { isThinking, typewriterText, typewriterDone, ...rest } = m; return { ...rest, dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.timestamp || (rest.created_at ? new Date(rest.created_at).getTime() : Date.now()) } }) })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch (e) { console.error('[KB] loadSessions failed:', e) }
  // ★ 允许无会话状态：首次发送消息时自动创建
  if (!currentSessionId.value && sessions.value.length) { currentSessionId.value = sessions.value[0].id }
  // 初始化分页状态：初始不足 10 条的会话没有更早消息
  for (const s of sessions.value) { sessionHasMore.value[s.id] = (s.messages?.length || 0) >= 10 }
  // 渲染当前会话的消息
  if (currentSessionId.value) { await selectSession(currentSessionId.value) }
  initialScrollDone = false
  nextTick(() => scrollToBottomIfNear())
})
// ★ 每次切换回来时重新加载会话（保持与全局对话框同步）
// 但如果 AI 正在回复中，不要覆盖当前会话状态
onActivated(async () => {
  // 如果 AI 正在回复中，跳过重新加载以保留进行中的状态
  if (agent.isChatting.value) {
    initialScrollDone = false
    nextTick(() => scrollToBottomIfNear())
    return
  }
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
  // ★ 重新渲染当前会话的所有 assistant 消息（KeepAlive 激活时 rendered 缓存可能为空）
  if (currentSessionId.value) {
    const session = sessions.value.find(s => s.id === currentSessionId.value)
    if (session) {
      agent.rendered.value = {}
      renderVersion.clear()
      await Promise.all(session.messages.map(m => {
        if (m.role === 'assistant' && m.content && !m.isThinking) {
          return renderMessageContent(m.id || '', m.content)
        }
        return Promise.resolve()
      }))
    }
  }
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
  if (!sessions.value.length) { currentSessionId.value = null; return }
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
let unlistenToken: (() => void) | null = null
let unlistenDone: (() => void) | null = null
let unlistenError: (() => void) | null = null

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
  // ★ 防御：清理上一轮可能泄漏的 listener，防止旧 listener 把新 token 写进旧气泡
  try { unlistenProgress?.(); unlistenToken?.(); unlistenDone?.(); unlistenError?.() } catch {}
  unlistenProgress = null; unlistenToken = null; unlistenDone = null; unlistenError = null
  if (q.length > MAX_INPUT_LENGTH) { toast({ message: `输入内容过长（${q.length}/${MAX_INPUT_LENGTH}），请精简后重试`, type: 'error' }); return }
  // ★ 无会话时自动创建一个（首次发消息即建会话）
  if (!currentSession.value) { newSession() }
  const session = currentSession.value; if (!session) return
  const atts = [...pendingAttachments.value]
  session.messages.push({ id: crypto.randomUUID(), role: 'user', content: q, attachments: atts.length ? atts : undefined, timestamp: Date.now() })
  if (session.name === '新对话') { session.name = q.slice(0, 30) }
  sessionDirty.value = true; inputMessage.value = ''; pendingAttachments.value = []; saveToStorage()
  nextTick(() => { const el = feedScrollRef.value as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight })
  nextTick(() => { const el = textareaRef.value; if (el) el.style.height = '' })
  agent.isChatting.value = true
  agent.setThinkingProgress('')
  // ★ Real Cognitive Stream: 实时进度
  unlistenProgress = await listen<{msg:string}>('agent:progress', (ev) => {
    agent.setThinkingProgress(pickRetrievalPhrase())
    scrollToBottomIfNear()
  })
  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', isThinking: true, toolCalls: [], timestamp: Date.now() }
  session.messages.push(aiMsg)
  // ★ 初始默认"思考中"，工具调用时才切换预设文本
  agent.setThinkingProgress('思考中')
  // ★ 实时 token 流（打字机队列）
  const tokenQueue: string[] = []
  let tokenIdx = 0
  let tokenFlushed = false
  const flushTokenQueue = () => {
    if (tokenFlushed) return; tokenFlushed = true
    clearInterval(tokenTimer)
    while (tokenIdx < tokenQueue.length) aiMsg.content += tokenQueue[tokenIdx++]
  }
  let lastRenderTime = 0
  const tokenTimer = setInterval(() => {
    if (tokenIdx < tokenQueue.length) { if (tokenIdx === 0) agent.setThinkingProgress(''); aiMsg.content += tokenQueue[tokenIdx++] }
    const now = Date.now()
    if (aiMsg.id && aiMsg.content && now - lastRenderTime > 200) {
      lastRenderTime = now
      renderMessageContent(aiMsg.id, aiMsg.content)
    }
    scrollToBottomIfNear()
  }, 15)
  unlistenToken = await listen<{token:string}>('agent:token', (ev) => {
    tokenQueue.push(ev.payload.token)
  })
  nextTick(() => { const el = feedScrollRef.value as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight })
  try {
    // ★ 监听 agent:done 事件（后台任务完成时触发）
    unlistenDone = await listen<any>('agent:done', async (ev) => {
      console.log('[FE] done received')
      flushTokenQueue()
      const result = ev.payload
      aiMsg.isThinking = false
      if (result.success) {
        const finalAnswer = (result.answer || '').replace(/<refs>[\s\S]*<\/refs>/g, '').trim()
        if (finalAnswer && !aiMsg.content) aiMsg.content = finalAnswer
        if (!aiMsg.content) aiMsg.content = result.answer || '（无回复内容）'
        // ★ 先完成渲染，再清空 progress（避免短暂窗口显示原始 Markdown）
        await renderMessageContent(aiMsg.id || '', aiMsg.content)
        agent.setThinkingProgress('')
      } else { aiMsg.error = result.answer || result.error || '失败' }
      agent.isChatting.value = false; scrollToBottomIfNear()
      
      // ★ 保存会话到数据库（确保 AI 回复被持久化）
      sessionDirty.value = true
      saveToStorage()
      
      try { unlistenDone(); unlistenToken(); unlistenProgress() } catch (e) { console.error('[KB] unlisten failed:', e) }
    })

    // ★ 监听 agent:error 事件
    unlistenError = await listen<any>('agent:error', (ev) => {
      flushTokenQueue()
      aiMsg.isThinking = false
      aiMsg.error = ev.payload.error || 'AI 服务异常'
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenError(); unlistenDone(); unlistenToken(); unlistenProgress() } catch {}
    })

    // ★ 调用命令（立即返回，不等待结果）
    invoke('kb_agent_chat', { data: { query: q, sessionId: currentSessionId.value, attachments: atts.map(a => ({ path: a.path, name: a.name })) } }).catch((err) => {
      console.error('[KB] invoke failed:', err)
      flushTokenQueue()
      aiMsg.isThinking = false
      aiMsg.error = String(err)
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenDone?.(); unlistenToken?.(); unlistenProgress?.(); unlistenError?.() } catch {}
    })
  } catch (e: any) {
    flushTokenQueue()
    aiMsg.isThinking = false
    aiMsg.error = typeof e === 'string' ? e : (e.message || e.toString?.() || 'AI 服务异常')
    agent.isChatting.value = false; scrollToBottomIfNear()
    try { unlistenDone?.(); unlistenToken?.(); unlistenProgress?.(); unlistenError?.() } catch {}
  }
}

async function stopAgent() {
  try {
    const { agentCancel } = await import('@/api/db-client')
    await agentCancel()
    agent.isChatting.value = false
    // C4 修复：停止时 unlisten 所有旧 listener，防止重发时重复处理
    unlistenProgress?.(); unlistenProgress = null
    unlistenToken?.(); unlistenToken = null
    unlistenDone?.(); unlistenDone = null
    unlistenError?.(); unlistenError = null
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
// ★ 页面停用时清理资源（KeepAlive 模式下不会触发 onUnmounted）
onDeactivated(() => {
  stopAllTypewriters()
  // 保存当前输入框文本和待发送附件到 localStorage
  try {
    localStorage.setItem('chat_input_backup', inputMessage.value)
    if (pendingAttachments.value.length > 0) {
      localStorage.setItem('chat_attachments_backup', JSON.stringify(pendingAttachments.value))
    }
  } catch {}
})

onUnmounted(() => {
  stopAllTypewriters()
  unlistenProgress?.()
  unlistenToken?.()
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
    <div ref="feedScrollRef" class="flex-1 overflow-y-auto overflow-x-hidden px-6 py-6 space-y-5 scrollbar-thin" @scroll="onFeedScroll">
      <!-- Empty State（无会话 或 会话无消息时显示） -->
      <div v-if="!currentSession || currentSession.messages.length === 0" class="flex flex-col items-center justify-center h-full text-center px-8 py-16">
        <div class="w-20 h-20 mb-6 rounded-3xl bg-gradient-to-br from-[#2563eb]/10 to-[#93c5fd]/10 flex items-center justify-center ring-1 ring-[#2563eb]/10">
          <svg class="w-10 h-10 text-[#2563eb]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M9.813 15.904L9 18.75l-.813-2.846a4.5 4.5 0 00-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 003.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 003.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 00-3.09 3.09z"/></svg>
        </div>
        <h2 class="text-[17px] font-semibold text-[#1d1d1f] mb-2">AI 知识助手</h2>
        <p class="text-[13px] text-[#86868b] max-w-md leading-relaxed">基于知识与记忆系统的AI智能助手，帮你检索、理解和推断项目、产品、文档等信息。</p>
      </div>
      <template v-if="currentSession">
        <!-- 加载更早消息提示 -->
        <div v-if="loadingMore" class="flex items-center justify-center py-2">
          <span class="text-[11px] text-surface-400">加载中...</span>
        </div>
        <div v-else-if="currentSession.messages.length > 0 && sessionHasMore[currentSession.id] === false" class="flex items-center justify-center py-2">
          <span class="text-[11px] text-surface-300">没有更早的消息了</span>
        </div>

        <!-- Messages -->
        <div v-for="(msg, idx) in currentSession.messages" :key="msg.id || idx" class="flex gap-3 items-start animate-fade-in" :class="msg.role === 'user' ? 'flex-row-reverse' : ''">
          <div v-if="msg.role === 'user'" class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-[#2563eb] text-white shadow-sm">我</div>
          <div v-else class="w-8 h-8 rounded-full flex items-center justify-center text-[11px] font-semibold flex-shrink-0 bg-gradient-to-br from-[#2563eb] to-[#93c5fd] text-white shadow-sm">AI</div>
            <div class="min-w-0" :class="msg.role === 'user' ? 'max-w-[65%]' : (msg.isThinking ? '' : 'max-w-[85%]')">
            <div class="text-[10px] text-[#c7c7cc] mb-1.5 px-1" :class="msg.role === 'user' ? 'text-right' : ''">{{ msg.timestamp ? formatChatTime(msg.timestamp) : '' }}</div>
            <div v-if="msg.error" class="bg-red-50 border border-red-100 rounded-2xl px-5 py-4 text-[13px] text-red-600 rounded-tl-[4px]">{{ msg.error }}</div>
            <div v-else-if="msg.role === 'user'" class="bg-[#2563eb] text-white rounded-2xl rounded-tr-[4px] px-3 py-2">
              <pre class="text-[12px] leading-relaxed whitespace-pre-wrap font-sans">{{ displayContent(msg.content) }}</pre>
              <AttachmentChips v-if="msg.attachments?.length" :files="msg.attachments" class="mt-1.5" />
              <div v-else-if="msg.content.includes('<<<ATTACHMENTS>>>')" class="flex flex-wrap gap-1 mt-1.5">
                <span v-for="name in extractAttachmentNames(msg.content)" :key="name" class="flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] bg-white/20 text-white/90">
                  <AppIcon name="file" size="10" class="opacity-80" />{{ name }}
                </span>
              </div>
            </div>
            <div v-if="msg.role !== 'user' && (msg.isThinking || msg.content)">
              <div class="bg-white border border-[#e5e5ea] rounded-2xl rounded-tl-[4px] px-3 py-2 shadow-sm w-full" :class="{ 'thinking-bubble': msg.isThinking }">
                <div v-if="agent.isTypewriting(msg)" class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
                <div v-else class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" :style="{ opacity: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中') ? 1 : 0, transform: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中') ? 'translateY(0)' : 'translateY(12px)', transition: 'opacity 1s ease, transform 1s ease' }" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
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
        <div class="flex items-start justify-between gap-2 mb-1.5">
          <AttachmentChips v-if="pendingAttachments.length" :files="pendingAttachments" removable @remove="removeAttachment" />
          <p class="text-[10px] text-surface-300 pr-1 flex-shrink-0 mt-0.5 ml-auto">↵ 发送 · Shift+↵ 换行</p>
        </div>
        <div class="flex items-end gap-2 bg-white rounded-2xl border border-surface-200 px-3 py-2 shadow-[0_2px_8px_rgba(0,0,0,0.06),0_0_0_1px_rgba(0,0,0,0.02)] transition-all duration-200 focus-within:border-brand-300 focus-within:shadow-[0_0_0_3px_rgba(37,99,235,0.1),0_4px_12px_rgba(0,0,0,0.08)]">
          <button @click="openAttachmentDialog" class="w-6 h-6 flex items-center justify-center flex-shrink-0 rounded-md text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors" title="附加文件（图片/文档）">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg>
          </button>
          <textarea ref="textareaRef" v-model="inputMessage" rows="1" maxlength="5000" placeholder="向 AI 提问..." class="flex-1 border-none resize-none outline-none text-[13px] text-surface-800 bg-transparent leading-relaxed placeholder:text-surface-400 max-h-[80px] py-1" @keydown.enter.exact.prevent="handleSendMessage" @paste="onPaste" @input="(e: any) => { if (!inputMessage.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 80) + 'px' }" />
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
    <div v-if="notebookPickerOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm">
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
/* LLM 聊天气泡 markdown 段落去外边距，与用户气泡等高 */
.markdown-body :deep(p) {
  margin: 0;
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
.thinking-spinner {
  display: inline-block; width: 14px; height: 14px; vertical-align: middle; margin-right: 4px;
  border: 2px solid #e5e5ea; border-top-color: #2563eb; border-radius: 50%;
  animation: ts-spin 0.8s linear infinite;
}
@keyframes ts-spin { to { transform: rotate(360deg); } }
</style>
