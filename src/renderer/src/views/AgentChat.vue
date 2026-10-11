<script setup lang="ts">
import { ref, computed, reactive, onMounted, onActivated, onDeactivated, onUnmounted, nextTick, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import ChatSidebar from '@/components/chat/ChatSidebar.vue'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import { kbAgentChat, kbChat, notesV2Create, fetchNotebooks, kbSaveSessions, kbLoadSessions, kbLoadSessionMessages, kbDeleteSession, kbWithdrawMessages, kbSessionFiles, kbReleaseGeneratedFile, previewFile, exportFile, copyFileToClipboard, type WithdrawnFile, type SessionFile } from '@/api/db-client'
import WithdrawDialog from '@/components/chat/WithdrawDialog.vue'
import SessionFilesPanel from '@/components/chat/SessionFilesPanel.vue'
import PlanDock from '@/components/chat/PlanDock.vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { useAgentChat, type ChatMessage as AgentChatMessage, type GeneratedDocument, type ClarificationData, extractDocumentsFromToolLog, parseDocumentsMetadata, parseClarificationMetadata } from '@/composables/useAgentChat'
import { pickRetrievalPhrase } from '@/utils/chat'
import { ProjectCard, TodoCard, ObjectList } from '@/components/renderer'
import DocumentObject from '@/components/chat/DocumentObject.vue'
import ClarifyCard from '@/components/chat/ClarifyCard.vue'
import MessageBody from '@/components/chat/MessageBody.vue'
import AnnotationLayer from '@/components/chat/AnnotationLayer.vue'
import AnnotatePanel from '@/components/chat/AnnotatePanel.vue'
import { hasInteractive, buildRevisionMessage, type Annotation, type TableRevision } from '@/utils/uiBlocks'
import { uploadPasteFile } from '@/utils/pasteUpload'
import { useAgentRunStore } from '@/stores/agentRun'
import { useLiveProgress } from '@/composables/useLiveProgress'

const agent = useAgentChat()
const agentRun = useAgentRunStore()

// ★ 生成进度计时：运行期每秒 tick；与 writingChars 心跳共同消除长文档生成期的"卡死"错觉
const elapsedSec = ref(0)
let elapsedTimer: ReturnType<typeof setInterval> | null = null
watch(() => agent.isChatting.value, (busy) => {
  if (busy) {
    elapsedSec.value = 0
    if (elapsedTimer) clearInterval(elapsedTimer)
    elapsedTimer = setInterval(() => { elapsedSec.value++ }, 1000)
  } else if (elapsedTimer) {
    clearInterval(elapsedTimer); elapsedTimer = null
  }
})
const fmtK = (n: number) => n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n)
const fmtElapsed = () => `${Math.floor(elapsedSec.value / 60)}:${String(elapsedSec.value % 60).padStart(2, '0')}`
const liveChars = useLiveProgress(
  () => { const a = agentRun.currentAgent; return a ? (agentRun.writingByAgent[a.id]?.confirmed ?? 0) : 0 },
  () => agent.isChatting.value,
  () => {
    const a = agentRun.currentAgent
    const w = a && agentRun.writingByAgent[a.id]
    return !!w && !w.cur && !!(w.stool || (w.pending ?? 0) > 400 || w.deliverable)
  }
)
// 当前子 Agent 进度口径（按 agent 分桶）：字数取轮末落定的 confirmed 按批叠加（provider 流式节奏无关）；
// 批间由 useLiveProgress 保守外推保持"在生长"观感；无新心跳时按"动作"显示（工具执行=正在：X；参数流=正在生成内容）
const wProgress = computed(() => {
  const a = agentRun.currentAgent
  if (!a) return ''
  const w = agentRun.writingByAgent[a.id]
  if (!w) return ''
  const shown = Math.max(Math.round(liveChars.value), w.confirmed ?? 0)
  if (w.stool || (w.pending ?? 0) > 400) return shown > 0 ? `正在生成内容 · ${fmtK(shown)} 字` : '正在生成内容'
  if (w.cur) return `正在：${w.cur}`
  if (w.deliverable) return shown > 0 ? `正在生成 · ${fmtK(shown)} 字` : ''
  return ''
})

// ─── Attachments ─────────────────────────────────────────────────
const IMG_EXTS = new Set(['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'])
const DOC_EXTS = new Set(['pdf', 'doc', 'docx', 'xlsx', 'xls', 'pptx', 'ppt', 'txt', 'md', 'csv', 'json'])
const ALL_EXTS = new Set([...IMG_EXTS, ...DOC_EXTS])
const MAX_ATTACH = 5

interface PendingAttachment {
  id: string
  path: string            // 上传中为 ''（占位），完成才写入
  name: string
  kind: 'image' | 'doc'
  previewUrl?: string
  uploading?: boolean
  progress?: number       // 0-100
  _cancelled?: boolean
}
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
    pendingAttachments.value.push({ id: crypto.randomUUID(), path: p, name, kind })
  }
}

function removeAttachment(idx: number) {
  const removed = pendingAttachments.value.splice(idx, 1)[0]
  if (!removed) return
  // 上传中：标记中断，分片循环下一轮中止并由后端清理临时文件
  if (removed.uploading) { removed._cancelled = true; return }
  if (removed.previewUrl) URL.revokeObjectURL(removed.previewUrl)
}

/** 分片上传一个粘贴文件：先占位（即时可见），再推进度 */
async function uploadPastedFile(file: File, ext: string, nameFallback: string): Promise<boolean> {
  const kind: 'image' | 'doc' = IMG_EXTS.has(ext) ? 'image' : 'doc'
  const chip: PendingAttachment = {
    id: crypto.randomUUID(),
    path: '',
    name: file.name || nameFallback,
    kind,
    previewUrl: kind === 'image' ? URL.createObjectURL(file) : undefined,
    uploading: true,
    progress: 0,
  }
  pendingAttachments.value.push(chip)
  try {
    const r = await uploadPasteFile(file, ext, (pct) => { chip.progress = pct }, () => !!chip._cancelled)
    if (!pendingAttachments.value.includes(chip)) {
      if (r?.path) invoke('file_remove_temp', { path: r.path }).catch(() => {})
      if (chip.previewUrl) URL.revokeObjectURL(chip.previewUrl)
      return false
    }
    chip.path = r.path
    chip.name = r.name || chip.name
    chip.uploading = false
    return true
  } catch (err: any) {
    const idx = pendingAttachments.value.indexOf(chip)
    if (idx >= 0) pendingAttachments.value.splice(idx, 1)
    if (chip.previewUrl) URL.revokeObjectURL(chip.previewUrl)
    const msg = String(err?.message || err)
    if (!chip._cancelled && msg !== 'Error: 已取消') {
      console.warn('[Chat] paste save failed:', err)
      if (msg.includes('上限') || msg.includes('超过')) toast({ message: msg, type: 'error' })
    }
    return false
  }
}

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
      if (await uploadPastedFile(file, ext, `pasted.${ext}`)) added++
    }
    if (added > 0) toast({ message: `已粘贴 ${added} 个文件`, type: 'success' })
    else toast({ message: '粘贴失败：不支持的文件类型', type: 'error' })
    return
  }
  // 回退：从 clipboardData.items 获取图片
  const items = e.clipboardData?.items
  if (!items) return
  let itemAdded = 0
  for (const item of items) {
    if (!item.type.startsWith('image/')) continue
    if (pendingAttachments.value.length >= MAX_ATTACH) { toast({ message: `最多附加 ${MAX_ATTACH} 个文件`, type: 'error' }); break }
    e.preventDefault()
    const file = item.getAsFile()
    if (!file) continue
    const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : item.type.split('/')[1]
    if (await uploadPastedFile(file, ext, `pasted.${ext}`)) itemAdded++
  }
  if (itemAdded > 0) toast({ message: '已粘贴图片', type: 'success' })
  // 系统剪贴板中的文件不暴露给 clipboardData（浏览器安全限制），
  // 本轮未取到任何文件且剪贴板确为文件列表时，才回退从后端读取（纯文本/图片粘贴不打后端）
  const clipboardHasFiles = Array.from(e.clipboardData?.types || []).includes('Files')
  if (itemAdded === 0 && clipboardHasFiles && pendingAttachments.value.length < MAX_ATTACH) {
    try {
      const r: any = await invoke('chat_read_clipboard_files')
      if (r?.success && Array.isArray(r.files) && r.files.length) {
        let addedMac = 0
        for (const f of r.files) {
          if (pendingAttachments.value.length >= MAX_ATTACH) break
          pendingAttachments.value.push({ id: crypto.randomUUID(), path: f.path, name: f.name, kind: f.kind || 'doc' })
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
interface ChatMessage extends AgentChatMessage { id?: string; role: 'user'|'assistant'; toolCalls?: any[]; rawToolCalls?: any[]; toolResults?: any[]; contextDocs?: any[]; workspace?: string; attachments?: Array<{path:string;name:string;kind:'image'|'doc'}>; documents?: import('@/composables/useAgentChat').GeneratedDocument[] }
interface ChatSession { id: string; name: string; messages: ChatMessage[]; createdAt: string; updatedAt?: string }

/// 历史消息恢复：从 chat_messages.metadata 还原文档卡片、追问卡片与 via 署名（重启后 tool_log 不在，靠 metadata）
function docFieldsFromMeta(m: any): { documents?: GeneratedDocument[]; clarification?: ClarificationData; assistants?: string[] } {
  const out: { documents?: GeneratedDocument[]; clarification?: ClarificationData; assistants?: string[] } = {}
  const docs = parseDocumentsMetadata(m.metadata)
  if (docs) out.documents = docs
  const clar = parseClarificationMetadata(m.metadata)
  if (clar) out.clarification = clar
  try {
    const arr = JSON.parse(m.metadata)?.assistants
    if (Array.isArray(arr) && arr.length) out.assistants = arr.filter((x: any) => typeof x === 'string' && x)
  } catch { /* 非 JSON 忽略 */ }
  return out
}

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
const savedSnapshots = new Map<string, string>()  // ★ 每会话内容快照：内容真变才写库/刷新时间，重命名与切换浏览不再误刷
let activeTokenTimer: ReturnType<typeof setInterval> | null = null  // 当前轮打字机定时器（done 丢失时由 deactivate/unmount 兜底清理）
let activeTokenConsume: (() => void) | null = null  // 当前轮 token 消费 tick（页面切回时据此重启被 onDeactivated 清掉的定时器）

function newSession() {
  stopAllTypewriters()
  const id = crypto.randomUUID()
  sessions.value.unshift({ id, name: '新对话', messages: [], createdAt: new Date().toLocaleString('sv-SE'), updatedAt: new Date().toLocaleString('sv-SE') })
  currentSessionId.value = id; saveToStorage()
}

// ★ 会话加载序号守卫：KeepAlive 首挂时 onMounted/onActivated 并发加载，晚发请求的结果必须覆盖早到者
let sessionsLoadSeq = 0

// ★ 启动时加载持久化会话
onMounted(async () => {
  window.addEventListener('ark:files-changed', onFilesChanged)
  const mySeq = ++sessionsLoadSeq
  // 恢复输入框文本
  try {
    const savedInput = localStorage.getItem('chat_input_backup')
    if (savedInput) { inputMessage.value = savedInput; localStorage.removeItem('chat_input_backup') }
    const savedAttachments = localStorage.getItem('chat_attachments_backup')
    if (savedAttachments) { pendingAttachments.value = JSON.parse(savedAttachments); localStorage.removeItem('chat_attachments_backup') }
  } catch {}
  
  try { const r = await kbLoadSessions(); if (mySeq !== sessionsLoadSeq) return; if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s:any) => ({ ...s, name: s.title || '新对话', updatedAt: s.last_msg_at || s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt, messages: (s.messages || []).map((m: any) => { const { isThinking, typewriterText, typewriterDone, ...rest } = m; return { ...rest, ...docFieldsFromMeta(rest), dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.timestamp || (rest.created_at ? new Date(rest.created_at).getTime() : Date.now()) } }) })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch (e) { console.error('[KB] loadSessions failed:', e) }
  // ★ 快照种子：加载即视为已保存基线，之后的纯浏览/切换不会触发写库
  for (const sess of sessions.value) savedSnapshots.set(sess.id, JSON.stringify(sess.messages))
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
    // ★ 切走时 onDeactivated 清掉了 token 消费定时器：切回必须重启，
    //   否则文本永久冻结到 agent:done 才一次性蹦出全文（视觉上就是"流式中断"）
    if (activeTokenConsume && !activeTokenTimer) {
      activeTokenTimer = setInterval(activeTokenConsume, 15)
    }
    initialScrollDone = false
    nextTick(() => scrollToBottomIfNear())
    return
  }
  try {
    const mySeq = ++sessionsLoadSeq
    const r = await kbLoadSessions()
    if (mySeq !== sessionsLoadSeq) return
    if (r?.sessions && Array.isArray(r.sessions)) {
      sessions.value = r.sessions.map((s: any) => ({
        ...s,
        name: s.title || '新对话',
        updatedAt: s.updated_at || s.updatedAt,
        createdAt: s.created_at || s.createdAt,
        messages: (s.messages || []).map((m: any) => {
          const { isThinking, typewriterText, typewriterDone, ...rest } = m
          return { ...rest, ...docFieldsFromMeta(rest), dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.timestamp || (rest.created_at ? new Date(rest.created_at).getTime() : Date.now()) }
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

function saveToStorage(target?: ChatSession) { 
  // 可显式传入提问时的会话：AI 回答期间用户切会话/新建后，回复必须存回原会话而非当前会话
  const s = target || currentSession.value; 
  if (s && s.messages.length > 0) {
    const currentContent = JSON.stringify(s.messages)
    // ★ 内容无变化 → 不写库也不刷新 updatedAt（done/切换等路径的无害重放直接短路）
    if (savedSnapshots.get(s.id) === currentContent) { sessionDirty.value = false; return }
    s.updatedAt = new Date().toLocaleString('sv-SE'); 
    savedSnapshots.set(s.id, currentContent)
    if (s === currentSession.value) {
      sessions.value.sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()); 
      sessionDirty.value = false;
    }
    kbSaveSessions([s]).catch((e) => console.error('[Agent] 保存会话失败:', e)); 
  }
}
async function loadFromStorage() {
  try { const r = await kbLoadSessions(); if (r?.sessions && Array.isArray(r.sessions)) sessions.value = r.sessions.map((s: any) => ({ ...s, name: s.title || '新会话', updatedAt: s.last_msg_at || s.updated_at || s.updatedAt, createdAt: s.created_at || s.createdAt, messages: (s.messages || []).map((m: any) => { const { isThinking, typewriterText, typewriterDone, ...rest } = m; return { ...rest, ...docFieldsFromMeta(rest) } }) })).sort((a: any, b: any) => new Date(b.updatedAt || b.createdAt).getTime() - new Date(a.updatedAt || a.createdAt).getTime()) } catch { sessions.value = [] }
  for (const sess of sessions.value) savedSnapshots.set(sess.id, JSON.stringify(sess.messages))
  if (!sessions.value.length) { currentSessionId.value = null; return }
  currentSessionId.value = sessions.value[0].id
  // 渲染前标记历史 AI 消息状态
  const s = currentSession.value
  if (s) {
    for (const m of s.messages) {
      if (m.role === 'assistant') {
        m.isThinking = false
        m.typewriterDone = true
      }
    }
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
        return { ...rest, ...docFieldsFromMeta(rest), dbId: rest.id, id: 'hist-' + rest.id, timestamp: rest.created_at ? new Date(rest.created_at).getTime() : Date.now() }
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
  if (!el) return
  // ★ 跟随态判定：距底 <48px 视为贴底；滚开即脱离磁吸，流式输出不再把用户拉回
  stickToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD
  if (el.scrollTop > 100) return
  loadOlderMessages()
}

let unlistenProgress: (() => void) | null = null
let unlistenThought: (() => void) | null = null
let unlistenAnswer: (() => void) | null = null
let unlistenToken: (() => void) | null = null
let unlistenDone: (() => void) | null = null
let unlistenError: (() => void) | null = null

async function confirmDeleteSession(id: string) {
  const ok = await dangerConfirm({ title: '删除会话', message: '确定删除此会话？会话中生成的文档将一并删除，此操作不可撤销。', confirmText: '删除' })
  if (!ok) return
  try {
    await kbDeleteSession(id)
  } catch {
    toast({ message: '删除失败，会话未从后端移除', type: 'error' })
    return
  }
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
// ★ 滚动跟随状态机：原 150px 磁吸区在流式输出时会吞掉用户上滑；
//   改为 stickToBottom 意图标志——只有用户滚到距底 30px 内才恢复吸附
const STICK_THRESHOLD = 30
const stickToBottom = ref(true)
function scrollToBottomIfNear() {
  const el = feedScrollRef.value as HTMLElement | null; if (!el) return
  if (!initialScrollDone) { initialScrollDone = true; stickToBottom.value = true; nextTick(() => { el.scrollTop = el.scrollHeight }); return }
  if (!stickToBottom.value) return
  nextTick(() => { const n = feedScrollRef.value as HTMLElement | null; if (n) n.scrollTop = n.scrollHeight })
}
function backToBottom() {
  stickToBottom.value = true
  nextTick(() => { const el = feedScrollRef.value as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight })
}

watch(currentSessionId, () => { initialScrollDone = false; stickToBottom.value = true })

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
  // 附件分片上传中：占位 chip 的 path 尚未就绪，直接发送会带上空路径
  if (pendingAttachments.value.some(a => a.uploading)) { toast({ message: '附件仍在上传，请稍候', type: 'error' }); return }
  console.log('[FE] handleSendMessage BUILD=streamfix-0719')
  // ★ 防御：清理上一轮可能泄漏的 listener，防止旧 listener 把新 token 写进旧气泡
  try { unlistenProgress?.(); unlistenThought?.(); unlistenAnswer?.(); unlistenToken?.(); unlistenDone?.(); unlistenError?.() } catch {}
  unlistenProgress = null; unlistenThought = null; unlistenAnswer = null; unlistenToken = null; unlistenDone = null; unlistenError = null
  if (q.length > MAX_INPUT_LENGTH) { toast({ message: `输入内容过长（${q.length}/${MAX_INPUT_LENGTH}），请精简后重试`, type: 'error' }); return }
  // ★ 无会话时自动创建一个（首次发消息即建会话）
  if (!currentSession.value) { newSession() }
  const session = currentSession.value; if (!session) return
  const atts = [...pendingAttachments.value]
  session.messages.push({ id: crypto.randomUUID(), role: 'user', content: q, attachments: atts.length ? atts : undefined, timestamp: Date.now() })
  if (session.name === '新对话') { session.name = q.slice(0, 30) }
  sessionDirty.value = true; inputMessage.value = ''; pendingAttachments.value = []; saveToStorage()
  stickToBottom.value = true; scrollToBottomIfNear()
  nextTick(() => { const el = textareaRef.value; if (el) el.style.height = '' })
  agent.isChatting.value = true
  agent.setThinkingProgress('')
  agentRun.resetParticipants()
  agentRun.setRunSid(session.id)
  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', isThinking: true, toolCalls: [], timestamp: Date.now() }
  session.messages.push(aiMsg)
  // ★ 初始默认"思考中"，工具调用时才切换预设文本
  agent.setThinkingProgress('思考中')
  // 思考流：本轮瞬态，正文开始流出即折叠为"思考了 N 秒"一行（可展开回看）
  agent.thought.value = ''
  agent.thoughtCollapsed.value = false
  agent.thoughtSecs.value = 0
  agent.thoughtMsgId.value = aiMsg.id || ''
  const thoughtT0 = Date.now()
  const collapseThought = () => {
    if (!agent.thoughtCollapsed.value && agent.thought.value) {
      agent.thoughtCollapsed.value = true
      agent.thoughtSecs.value = Math.max(1, Math.round((Date.now() - thoughtT0) / 1000))
    }
  }
  // ★ 实时 token 流（打字机队列）
  const tokenQueue: string[] = []
  let tokenIdx = 0
  let tokenFlushed = false
  if (activeTokenTimer) { clearInterval(activeTokenTimer); activeTokenTimer = null }
  const flushTokenQueue = () => {
    if (tokenFlushed) return; tokenFlushed = true
    // 清本轮定时器 + 可能被 onActivated 重启过的新 id（旧 id 已死，须按 activeTokenTimer 清）
    clearInterval(tokenTimer)
    if (activeTokenTimer) { clearInterval(activeTokenTimer); activeTokenTimer = null }
    activeTokenConsume = null
    while (tokenIdx < tokenQueue.length) aiMsg.content += tokenQueue[tokenIdx++]
  }
  let lastRenderTime = 0
  const tokenTick = () => {
    if (tokenIdx < tokenQueue.length) { if (tokenIdx === 0) agent.setThinkingProgress(''); aiMsg.content += tokenQueue[tokenIdx++] }
    const now = Date.now()
    if (aiMsg.id && aiMsg.content && now - lastRenderTime > 200) {
      lastRenderTime = now
      renderMessageContent(aiMsg.id, aiMsg.content)
    }
    scrollToBottomIfNear()
  }
  const tokenTimer = setInterval(tokenTick, 15)
  activeTokenTimer = tokenTimer
  activeTokenConsume = tokenTick
  // ★ listen 全部纳入 try：任一注册失败都会走 catch 复位 isChatting（此前 progress/token listen 在 try 外，
  //   reject 后永久卡"思考中"且 invoke 永不发起）
  try {
    // ★ Real Cognitive Stream: 实时进度（优先展示后端 msg，如"已路由到「XX」"，否则随机话术）
    unlistenProgress = await listen<{msg?:string;sid?:string}>('agent:progress', (ev) => {
      if (ev.payload?.sid && ev.payload.sid !== session.id) return
      agent.setThinkingProgress(ev.payload?.msg || pickRetrievalPhrase())
      scrollToBottomIfNear()
    })
    unlistenThought = await listen<{text:string;sid?:string}>('agent:thought', (ev) => {
      if (agent.thoughtCollapsed.value) return
      if (ev.payload?.sid && ev.payload.sid !== session.id) return
      agent.thought.value += ev.payload?.text || ''
      scrollToBottomIfNear()
    })
    unlistenAnswer = await listen<{stage?:string;sid?:string}>('agent:status', (ev) => {
      if (ev.payload?.stage !== 'answer') return
      if (ev.payload?.sid && ev.payload.sid !== session.id) return
      collapseThought()
    })
    unlistenToken = await listen<{token:string;sid?:string}>('agent:token', (ev) => {
      if (ev.payload?.sid && ev.payload.sid !== session.id) return
      tokenQueue.push(ev.payload.token)
    })
    stickToBottom.value = true; scrollToBottomIfNear()
    // ★ 监听 agent:done 事件（后台任务完成时触发）
    unlistenDone = await listen<any>('agent:done', async (ev) => {
      if (ev.payload?.sid && ev.payload.sid !== session.id) { console.warn('[sid-filter] drop done', ev.payload.sid, session.id); return }
      console.log('[FE] done received')
      flushTokenQueue()
      collapseThought()
      const result = ev.payload
      aiMsg.isThinking = false
      if (result.success) {
        const finalAnswer = (result.answer || '').replace(/<refs>[\s\S]*<\/refs>/g, '').trim()
        // ★ 提取 AI 生成的文档对象（预览/下载/复制卡片）
        const docs = extractDocumentsFromToolLog(result.tool_log)
        if (docs.length) aiMsg.documents = docs
        if (filesOpen.value) loadSessionFiles()
        // ★ 追问卡片结构化数据（text+card 双轨：正文已是文本版）
        if (result.clarification && Array.isArray(result.clarification.questions) && result.clarification.questions.length) {
          aiMsg.clarification = result.clarification
        }
        // ★ via 署名：本轮参与过的子 Agent（done 处理器先于下轮 reset 读取，全局监听只清 currentAgent）
        if (agentRun.participants.length) aiMsg.assistants = [...agentRun.participants]
        // ★ 权威答案覆盖已流出的片段（委托/多轮场景下 content 可能是委托前的开场白）；取消时保留已流出的部分内容
        if (finalAnswer && !result.cancelled) {
          aiMsg.content = finalAnswer
        } else if (!aiMsg.content) aiMsg.content = result.answer || '（无回复内容）'
        // ★ 先完成渲染，再清空 progress（避免短暂窗口显示原始 Markdown）
        await renderMessageContent(aiMsg.id || '', aiMsg.content)
        agent.setThinkingProgress('')
      } else { aiMsg.error = result.answer || result.error || '失败' }
      agent.isChatting.value = false; scrollToBottomIfNear()
      
      // ★ 保存会话到数据库（确保 AI 回复被持久化；显式存回提问时的会话）
      saveToStorage(session)
      
      try { unlistenDone(); unlistenThought(); unlistenAnswer?.(); unlistenToken(); unlistenProgress(); unlistenError() } catch (e) { console.error('[KB] unlisten failed:', e) }
    })

    // ★ 监听 agent:error 事件
    unlistenError = await listen<any>('agent:error', (ev) => {
      if (ev.payload?.sid && ev.payload.sid !== session.id) return
      flushTokenQueue()
      collapseThought()
      aiMsg.isThinking = false
      aiMsg.error = ev.payload.error || 'AI 服务异常'
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenError(); unlistenThought(); unlistenAnswer?.(); unlistenDone(); unlistenToken(); unlistenProgress() } catch {}
    })

    // ★ 调用命令（立即返回，不等待结果）
    invoke('kb_agent_chat', { data: { query: q, sessionId: currentSessionId.value, attachments: atts.map(a => ({ path: a.path, name: a.name })) } }).catch((err) => {
      console.error('[KB] invoke failed:', err)
      flushTokenQueue()
      aiMsg.isThinking = false
      aiMsg.error = String(err)
      agent.isChatting.value = false; scrollToBottomIfNear()
      try { unlistenDone?.(); unlistenThought?.(); unlistenAnswer?.(); unlistenToken?.(); unlistenProgress?.(); unlistenError?.() } catch {}
    })
  } catch (e: any) {
    flushTokenQueue()
    aiMsg.isThinking = false
    aiMsg.error = typeof e === 'string' ? e : (e.message || e.toString?.() || 'AI 服务异常')
    agent.isChatting.value = false; scrollToBottomIfNear()
    try { unlistenDone?.(); unlistenThought?.(); unlistenAnswer?.(); unlistenToken?.(); unlistenProgress?.(); unlistenError?.() } catch {}
  }
}

/// 追问卡片提交：合成答复文本走正常发送管线（handleSendMessage 自带忙碌守卫）
function submitClarification(text: string) {
  inputMessage.value = text
  handleSendMessage()
}

/// 已答复卡片的答案视图数据源：该消息之后最近一条用户消息正文（刷新/重开会话仍可解析）
function clarificationReply(msgId?: string): string {
  if (!msgId) return ''
  const arr = currentSession.value?.messages || []
  const i = arr.findIndex(x => x.id === msgId)
  if (i < 0) return ''
  return arr.slice(i + 1).find(x => x.role === 'user')?.content || ''
}

// ─── 批注修订（仅最后一条 AI 答复可批注；表格改动+正文批注统一提交）───
const annotateId = ref<string | null>(null)
const thoughtExpanded = ref(false)
const thoughtBoxRef = ref<HTMLElement | null>(null)
watch(() => agent.thought.value.length, () => {
  if (agent.thoughtCollapsed.value) return
  nextTick(() => { const raw: any = thoughtBoxRef.value; const el = Array.isArray(raw) ? raw[raw.length - 1] : raw; if (el) el.scrollTop = el.scrollHeight })
})
watch(() => agent.thoughtCollapsed.value, (v) => { if (v) thoughtExpanded.value = false })
const annotations = reactive<Annotation[]>([])
const mbRefs: Record<string, { getRevisions: () => { diff: string; headerLabel: string }[]; resetTableRevision: (i: number) => void } | null> = {}
const tableRevisions = ref<TableRevision[]>([])
function enterAnnotate(id: string) {
  if (agent.isChatting.value) return
  annotations.length = 0
  tableRevisions.value = []
  annotateId.value = id
  // 批注面板在消息尾部展开，先归队到底确保可见
  nextTick(() => backToBottom())
}
function exitAnnotate() {
  annotateId.value = null
  annotations.length = 0
  tableRevisions.value = []
}
function submitAnnotation() {
  const tables = mbRefs[annotateId.value || '']?.getRevisions() ?? []
  const text = buildRevisionMessage(tables, [...annotations])
  if (!text) { toast({ message: '尚无修订 —— 改表格或选中文字提出要求', type: 'info' }); return }
  exitAnnotate()
  submitClarification(text)
}
watch(() => agent.isChatting.value, (v) => { if (v) exitAnnotate() })
watch(currentSessionId, () => { exitAnnotate(); if (filesOpen.value) loadSessionFiles() })

// ─── 撤回：任意用户消息起，其后全部移出会话与上下文；可选连带删除生成文件 ───
const withdrawTarget = ref<{ text: string; files: WithdrawnFile[]; idx: number; nth: number; attachments: any[] } | null>(null)
function askWithdraw(idx: number) {
  const arr = currentSession.value?.messages || []
  const anchor = arr[idx]
  if (!anchor || anchor.role !== 'user') return
  const files = arr.slice(idx + 1).flatMap(m => (m.documents || []).map(d => ({ id: d.file_id, name: d.file_name })))
  // 同文本序号：后端锚点按"第 n 条同文本 active 消息"定位（历史行含 ATTACHMENTS 后缀，按基底文本比对）
  const base = (c: string) => (c || '').split('\n\n<<<ATTACHMENTS>>>')[0]
  const text = base(anchor.content || '')
  const nth = arr.slice(0, idx + 1).filter(m => m.role === 'user' && base(m.content || '') === text).length - 1
  withdrawTarget.value = { text, files, idx, nth, attachments: (anchor as any).attachments || [] }
}
async function doWithdraw(deleteFiles: boolean) {
  const t = withdrawTarget.value
  if (!t) return
  withdrawTarget.value = null
  const sid = currentSessionId.value
  if (!sid) return
  try {
    const r = await kbWithdrawMessages(sid, t.text, deleteFiles, t.nth)
    if (r.success) {
      const s = currentSession.value
      if (s) { s.messages.splice(t.idx); saveToStorage(s) }
      exitAnnotate()
      toast({ message: `已撤回 ${r.withdrawn_count} 条消息${r.files_deleted ? `，删除 ${r.files_deleted} 个文件` : ''}`, type: 'success' })
      if (filesOpen.value) loadSessionFiles()
      // 撤回回喂：修改后一键重发（本会话内消息带完整附件；历史消息无附件路径仅回文本）
      if (t.text) {
        inputMessage.value = t.text
        if (t.attachments?.length) pendingAttachments.value = t.attachments
        nextTick(() => { const el = textareaRef.value; if (el) { el.focus(); el.style.height = 'auto'; el.style.height = Math.min(el.scrollHeight, 120) + 'px' } })
      }
    } else {
      toast({ message: r.error || '撤回失败：范围为空', type: 'error' })
    }
  } catch (e: any) { toast({ message: `撤回失败：${e?.message || e}`, type: 'error' }) }
}

// ─── 交付文件面板（会话生成文件集中管理；与消息存活无关）───
const filesOpen = ref(false)
const sessionFiles = ref<SessionFile[]>([])
async function loadSessionFiles() {
  const sid = currentSessionId.value
  if (!sid) { sessionFiles.value = []; return }
  try { sessionFiles.value = (await kbSessionFiles(sid)).files || [] } catch { sessionFiles.value = [] }
}
function toggleFilesPanel() { filesOpen.value = !filesOpen.value; if (filesOpen.value) loadSessionFiles() }
async function deleteSessionFile(f: SessionFile) {
  const ok = await dangerConfirm({ title: '删除文件', message: `「${f.name}」将从资料库、知识库与消息卡片中移除`, confirmText: '删除' })
  if (!ok) return
  try { await kbReleaseGeneratedFile(f.id); toast({ message: '已删除', type: 'success' }); loadSessionFiles() } catch (e: any) { toast({ message: `删除失败：${e?.message || e}`, type: 'error' }) }
}
const onFilesChanged = () => { if (filesOpen.value) loadSessionFiles() }
async function copyUserMsg(msg: ChatMessage) {
  try { await navigator.clipboard.writeText((msg.content || '').trim()); toast({ message: '已复制', type: 'success' }) } catch { toast({ message: '复制失败', type: 'error' }) }
}
function pfFile(f: SessionFile) { previewFile(f.id).catch(() => {}) }
async function dlFile(f: SessionFile) {  try { const r = await exportFile(f.id); if (r?.success) toast({ message: '已保存', type: 'success' }); else if (r?.error && r.error !== '已取消') toast({ message: r.error, type: 'error' }) }
  catch (e: any) { toast({ message: e?.message || '下载失败', type: 'error' }) }
}
async function cpFile(f: SessionFile) {
  try { const r = await copyFileToClipboard(f.id); if (r?.success) toast({ message: `已复制「${r.fileName || f.name}」`, type: 'success' }); else toast({ message: r?.error || '复制失败', type: 'error' }) }
  catch (e: any) { toast({ message: e?.message || '复制失败', type: 'error' }) }
}

async function stopAgent() {
  try {
    const { agentCancel } = await import('@/api/db-client')
    await agentCancel(agentRun.runSid || undefined)
    agent.isChatting.value = false
    // C4 修复：停止时 unlisten 所有旧 listener，防止重发时重复处理
    unlistenProgress?.(); unlistenProgress = null
    unlistenThought?.(); unlistenAnswer?.(); unlistenThought = null
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
    agent.isChatting.value = false  // ★ agent_cancel 调用失败也复位状态，避免发送按钮永久红着
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
  if (activeTokenTimer) { clearInterval(activeTokenTimer); activeTokenTimer = null }
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
  if (activeTokenTimer) { clearInterval(activeTokenTimer); activeTokenTimer = null }
  activeTokenConsume = null
  unlistenProgress?.()
  unlistenThought?.()
  unlistenToken?.()
  unlistenDone?.()
  unlistenError?.()
  if (elapsedTimer) clearInterval(elapsedTimer)
  window.removeEventListener('ark:files-changed', onFilesChanged)
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
      @saveRename="(id:string)=>{const s=sessions.find(x=>x.id===id);if(s){const old=s.name;s.name=sessionNameEdit.trim()||s.name;editingSessionId=null;if(s.name!==old)kbSaveSessions([s]).catch((e)=>console.error('[Agent] 重命名保存失败:', e))}}"
      @updateSessionNameEdit="(val:string)=>sessionNameEdit=val"
      @cancelRename="editingSessionId=null" />
  </transition>

  <!-- Main Chat -->
  <div class="flex-1 flex flex-col min-w-0 relative bg-surface-50/30">
    <SessionFilesPanel :files="sessionFiles" :open="filesOpen" @toggle="toggleFilesPanel"
      @preview="pfFile" @download="dlFile" @copy="cpFile" @remove="deleteSessionFile" />
    <!-- ★ 流式输出中用户上滚脱离跟随时，提供一键归队入口 -->
    <button v-if="!stickToBottom && agent.isChatting.value" @click="backToBottom"
      class="absolute bottom-28 left-1/2 -translate-x-1/2 z-30 w-9 h-9 rounded-full bg-white border border-surface-200 shadow-lg flex items-center justify-center text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors"
      v-tooltip="'回到底部'">
      <AppIcon name="chevron-down" size="16" />
    </button>
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
        <div v-for="(msg, idx) in currentSession.messages" :key="msg.id || idx" class="flex items-start animate-fade-in" :class="msg.role === 'user' ? 'justify-end' : ''">
            <div class="min-w-0" :class="msg.role === 'user' ? 'max-w-[72%]' : (msg.isThinking ? '' : 'max-w-[92%]')">
            <div class="text-[10px] text-[#c7c7cc] mb-1.5 px-1 flex items-center gap-1.5" :class="msg.role === 'user' ? 'justify-end' : ''">
              <span>{{ msg.timestamp ? formatChatTime(msg.timestamp) : '' }}</span>
              <span v-if="!msg.isThinking && msg.assistants?.length" class="px-1.5 py-[1px] rounded-full bg-[#f2f2f7] text-[#8e8e93] text-[9px] leading-none">via {{ msg.assistants.join(' · ') }}</span>
            </div>
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
            <div v-if="msg.role === 'user' && !agent.isChatting.value && !msg.error" class="flex justify-end items-center gap-0.5 px-1 py-0.5">
              <button class="w-5 h-5 flex items-center justify-center rounded text-[#c7c7cc] hover:text-red-500 transition-colors" v-tooltip="'撤回：移除此后全部对话'" @click="askWithdraw(idx)"><AppIcon name="undo" size="11" /></button>
              <button class="w-5 h-5 flex items-center justify-center rounded text-[#c7c7cc] hover:text-[#2563eb] transition-colors" v-tooltip="'复制'" @click="copyUserMsg(msg)"><AppIcon name="copy" size="11" /></button>
            </div>
            <div v-if="msg.role !== 'user' && (msg.isThinking || msg.content)">
              <div class="bg-white border border-[#e5e5ea] rounded-2xl rounded-tl-[4px] px-3 py-2 shadow-sm w-full" :class="{ 'thinking-bubble': msg.isThinking }">
                <!-- 委托轨迹行：子 Agent 执行中且正文尚未流出时，顶替"思考中"占位（气泡内唯一状态位） -->
                <div v-if="msg.isThinking && !renderedContent(msg.id || '') && agentRun.currentAgent" class="flex items-center gap-1.5 py-0.5">
                  <span class="w-1.5 h-1.5 rounded-full bg-brand-500 animate-pulse flex-shrink-0"></span>
                  <span class="text-[12px] font-medium text-[#3a3a3c]">{{ agentRun.currentAgent.name }}</span>
                  <span class="text-[12px] text-[#8e8e93]">· {{ wProgress || '处理中' }}{{ elapsedSec > 0 ? ` · 已用 ${fmtElapsed()}` : '' }}</span>
                </div>
                <div v-else-if="agent.isTypewriting(msg)" class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
                <AnnotationLayer v-else-if="annotateId === (msg.id || '')" :active="true" @add="annotations.push($event)">
                  <MessageBody class="text-[12px] text-[#3a3a3c] leading-relaxed" :raw="msg.content" :msg-id="msg.id || ''" :annotate="true" :ref="(el: any) => { mbRefs[msg.id || ''] = el }" @send="submitClarification" @revision-change="(r: any) => { if (annotateId === (msg.id || '')) tableRevisions = r }" />
                </AnnotationLayer>
                <MessageBody v-else-if="hasInteractive(msg.content)" class="text-[12px] text-[#3a3a3c] leading-relaxed" :raw="msg.content" :msg-id="msg.id || ''" @send="submitClarification" />
                <div v-else class="text-[12px] text-[#3a3a3c] leading-relaxed markdown-body" :style="{ opacity: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中') ? 1 : 0, transform: (renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中') ? 'translateY(0)' : 'translateY(12px)', transition: 'opacity 1s ease, transform 1s ease' }" v-html="renderedContent(msg.id || '') || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
                <!-- 思考流（本轮瞬态：流式滚动自动跟底，正文开始折叠为一行可展开，刷新/历史不存在） -->
                <div v-if="agent.thought.value && (msg.id || '') === agent.thoughtMsgId.value" class="mt-1">
                  <div v-if="!agent.thoughtCollapsed.value" ref="thoughtBoxRef"
                    class="max-h-[96px] overflow-y-auto rounded-lg bg-[#f7f7f9] border border-[#ececf0] px-2.5 py-1.5 text-[11px] leading-relaxed text-[#8e8e93] whitespace-pre-wrap break-words">{{ agent.thought.value }}</div>
                  <template v-else>
                    <button type="button" class="flex items-center gap-1 text-[10px] text-[#c7c7cc] hover:text-[#2563eb] transition-colors" @click="thoughtExpanded = !thoughtExpanded">
                      <AppIcon :name="thoughtExpanded ? 'chevron-down' : 'chevron-right'" size="10" />
                      思考了 {{ agent.thoughtSecs.value }} 秒 · {{ agent.thought.value.length.toLocaleString() }} 字
                    </button>
                    <div v-if="thoughtExpanded" ref="thoughtBoxRef"
                      class="mt-1 max-h-[160px] overflow-y-auto rounded-lg bg-[#f7f7f9] border border-[#ececf0] px-2.5 py-1.5 text-[11px] leading-relaxed text-[#8e8e93] whitespace-pre-wrap break-words">{{ agent.thought.value }}</div>
                  </template>
                </div>
              </div>
              <div v-if="!msg.isThinking && msg.content" class="flex items-center gap-1 px-1.5 py-1.5">
                <button @click="copyAnswer(idx)" class="w-6 h-6 flex items-center justify-center rounded-md text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5 transition-all" v-tooltip="'复制答案'"><AppIcon name="copy" class="w-3 h-3" /></button>
                <button v-if="idx === currentSession.messages.length - 1 && !agent.isChatting.value && !msg.error"
                  class="annotate-skip w-6 h-6 flex items-center justify-center rounded-md transition-all" :class="annotateId === (msg.id || '') ? 'text-brand-600 bg-brand-50' : 'text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5'"
                  v-tooltip="annotateId === (msg.id || '') ? '取消修订' : '修订'"
                  @click="annotateId === (msg.id || '') ? exitAnnotate() : enterAnnotate(msg.id || '')"><AppIcon name="edit" size="12" /></button>
                <button v-if="msg.workspace" @click="showWorkspaceIdx = showWorkspaceIdx === idx ? null : idx" class="w-6 h-6 flex items-center justify-center rounded-md transition-all" :class="showWorkspaceIdx === idx ? 'text-[#2563eb] bg-[#2563eb]/10' : 'text-[#c7c7cc] hover:text-[#2563eb] hover:bg-[#2563eb]/5'" v-tooltip="'查看 Workspace'">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M9 17V7m0 10a2 2 0 01-2-2V9a2 2 0 012-2h6a2 2 0 012 2v6a2 2 0 01-2 2m-6 0h6"/></svg>
                </button>
              </div>
              <div v-if="showWorkspaceIdx === idx && msg.workspace" class="mt-1 p-3 bg-[#f8f9fa] border border-[#e5e5ea] rounded-xl text-[12px] font-mono whitespace-pre-wrap leading-relaxed text-[#3a3a3c] max-h-[300px] overflow-y-auto">{{ msg.workspace }}</div>
              <AnnotatePanel v-if="annotateId === (msg.id || '')" class="annotate-skip mt-1.5" :items="annotations" :tables="tableRevisions"
                @remove="annotations.splice($event, 1)" @submit="submitAnnotation" @cancel="exitAnnotate"
                @reset-table="mbRefs[annotateId || '']?.resetTableRevision($event)" />
              <div v-if="!msg.isThinking && msg.documents?.length" class="mt-1.5 flex flex-col gap-1.5">
                <DocumentObject v-for="d in msg.documents" :key="d.file_id" :doc="d" />
              </div>
              <div v-if="!msg.isThinking && msg.clarification" class="mt-1.5">
                <ClarifyCard
                  :clarification="msg.clarification"
                  :answered="idx < currentSession.messages.length - 1"
                  :reply-text="clarificationReply(msg.id)"
                  @submit="submitClarification"
                />
              </div>
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- Input Bar -->
    <div class="flex-shrink-0 px-4 pb-4 pt-2 bg-surface-50/30">
      <div class="max-w-3xl mx-auto">
        <PlanDock />
        <div class="flex items-start justify-between gap-2 mb-1.5">
          <AttachmentChips v-if="pendingAttachments.length" :files="pendingAttachments" removable @remove="removeAttachment" />
          <p class="text-[10px] text-surface-300 pr-1 flex-shrink-0 mt-0.5 ml-auto">↵ 发送 · Shift+↵ 换行</p>
        </div>
        <div class="flex items-end gap-2 bg-white rounded-2xl border border-surface-200 px-3 py-2 shadow-[0_2px_8px_rgba(0,0,0,0.06),0_0_0_1px_rgba(0,0,0,0.02)] transition-all duration-200 focus-within:border-brand-300 focus-within:shadow-[0_0_0_3px_rgba(37,99,235,0.1),0_4px_12px_rgba(0,0,0,0.08)]">
          <button @click="openAttachmentDialog" class="w-7 h-7 flex items-center justify-center flex-shrink-0 rounded-md text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors" v-tooltip="'附加文件（图片/文档）'">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg>
          </button>
          <textarea ref="textareaRef" v-model="inputMessage" rows="1" maxlength="5000" placeholder="向 AI 提问..." class="flex-1 border-none resize-none outline-none text-[13px] text-surface-800 bg-transparent leading-relaxed placeholder:text-surface-400 max-h-[80px] py-1" @keydown.enter.exact.prevent="handleSendMessage" @paste="onPaste" @input="(e: any) => { if (!inputMessage.trim()) { e.target.style.height = ''; return }; e.target.style.height = 'auto'; e.target.style.height = Math.min(e.target.scrollHeight, 80) + 'px' }" />
          <button v-if="agent.isChatting.value" @click="stopAgent" class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 bg-red-500 hover:bg-red-600 text-white transition-all duration-200" v-tooltip="'停止'">
            <svg class="w-3.5 h-3.5" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
          </button>
          <button v-else @click="handleSendMessage" :disabled="!inputMessage.trim()" class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 transition-all duration-200" :class="inputMessage.trim() ? 'bg-brand-600 text-white hover:bg-brand-700 shadow-sm' : 'bg-surface-100 text-surface-400 cursor-not-allowed'" v-tooltip="'发送'">
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
  <WithdrawDialog :visible="!!withdrawTarget" :anchor-text="withdrawTarget?.text || ''" :files="withdrawTarget?.files || []"
    @confirm="doWithdraw" @cancel="withdrawTarget = null" />

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
