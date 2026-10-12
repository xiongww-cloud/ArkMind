<script setup lang="ts">
import { ref, computed, reactive, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { fabResolveSession, fabNewSession, kbFabLoadHistory, kbWithdrawMessages, type WithdrawnFile } from '@/api/db-client'
import WithdrawDialog from '@/components/chat/WithdrawDialog.vue'
import { useAgentChat, type ChatMessage, extractDocumentsFromToolLog, parseDocumentsMetadata, parseClarificationMetadata } from '@/composables/useAgentChat'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'
import DocumentObject from '@/components/chat/DocumentObject.vue'
import ClarifyCard from '@/components/chat/ClarifyCard.vue'
import MessageBody from '@/components/chat/MessageBody.vue'
import AnnotationLayer from '@/components/chat/AnnotationLayer.vue'
import AnnotatePanel from '@/components/chat/AnnotatePanel.vue'
import PlanDock from '@/components/chat/PlanDock.vue'
import { hasInteractive, buildRevisionMessage, type Annotation, type TableRevision } from '@/utils/uiBlocks'
import { uploadPasteFile } from '@/utils/pasteUpload'
import { useAgentRunStore } from '@/stores/agentRun'
import { useLiveProgress } from '@/composables/useLiveProgress'

const agent = useAgentChat()
const agentRun = useAgentRunStore()

// --- Attachments ---
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
  // 打开原生文件对话框前，告知后端进入"对话框模式"，失焦不隐藏 chat 窗口
  invoke('set_chat_dialog_mode', { active: true }).catch(() => {})
  let result: any = null
  try {
    result = await open({ multiple: true, canCreateDirectories: false, filters: [{ name: 'Files', extensions: [...ALL_EXTS] }] })
  } finally {
    invoke('set_chat_dialog_mode', { active: false }).catch(() => {})
  }
  if (!result) return
  const paths: string[] = Array.isArray(result) ? result : [result as string]
  let added = 0
  for (const p of paths) {
    if (pendingAttachments.value.length >= MAX_ATTACH) break
    const name = p.replace(/^.*[\\\/]/, '')
    const ext = extOf(name)
    if (!ALL_EXTS.has(ext)) continue
    if (pendingAttachments.value.some(a => a.path === p)) continue
    pendingAttachments.value.push({ id: crypto.randomUUID(), path: p, name, kind: IMG_EXTS.has(ext) ? 'image' : 'doc' })
    added++
  }
  // 添加附件即展开全历史，便于结合上下文提问
  if (added > 0) revealOlder.value = true
}
function removeAttachment(idx: number) {
  const removed = pendingAttachments.value.splice(idx, 1)[0]
  if (!removed) return
  // 上传中：标记中断，分片循环下一轮中止并由后端清理临时文件
  if (removed.uploading) { removed._cancelled = true; return }
  // 仅手动移除时回收 blob（发送后消息气泡仍引用同一 URL，不可提前 revoke）
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
    // File 即 Blob，零拷贝秒出缩略图
    previewUrl: kind === 'image' ? URL.createObjectURL(file) : undefined,
    uploading: true,
    progress: 0,
  }
  pendingAttachments.value.push(chip)
  revealOlder.value = true
  try {
    const r = await uploadPasteFile(file, ext, (pct) => { chip.progress = pct }, () => !!chip._cancelled)
    // 期间可能已被取消/移除
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
    if (String(err) !== 'Error: 已取消' && !chip._cancelled) {
      console.warn('[Chat] paste save failed:', err)
      // 超限等明确错误提示用户
      const msg = String(err?.message || err)
      if (msg.includes('上限') || msg.includes('超过')) {
        import('@/composables/useToast').then(m => m.toast({ message: msg, type: 'error' }))
      }
    }
    return false
  }
}
async function onPaste(e: ClipboardEvent) {
  let added = 0
  const files = e.clipboardData?.files
  if (files && files.length > 0) {
    e.preventDefault()
    for (const file of Array.from(files)) {
      if (pendingAttachments.value.length >= MAX_ATTACH) break
      const ext = extOf(file.name) || (file.type.split('/')[1] === 'jpeg' ? 'jpg' : file.type.split('/')[1] || 'bin')
      if (!ALL_EXTS.has(ext) && !IMG_EXTS.has(ext)) continue
      if (await uploadPastedFile(file, ext, `pasted.${ext}`)) added++
    }
    return
  }
  const items = e.clipboardData?.items
  let itemAdded = 0
  if (items) {
    for (const item of items) {
      if (!item.type.startsWith('image/')) continue
      if (pendingAttachments.value.length >= MAX_ATTACH) break
      e.preventDefault()
      const file = item.getAsFile(); if (!file) continue
      const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : item.type.split('/')[1]
      if (await uploadPastedFile(file, ext, `pasted.${ext}`)) itemAdded++
    }
  }
  added += itemAdded
  // 系统剪贴板中的文件不暴露给 clipboardData（浏览器安全限制），
  // 若本轮未取到任何文件且剪贴板确为文件列表，才回退从后端读取（纯文本粘贴不打后端）
  const clipboardHasFiles = Array.from(e.clipboardData?.types || []).includes('Files')
  if (added === 0 && clipboardHasFiles) {
    try {
      const r: any = await invoke('chat_read_clipboard_files')
      if (r?.success && Array.isArray(r.files) && r.files.length) {
        for (const f of r.files) {
          if (pendingAttachments.value.length >= MAX_ATTACH) break
          pendingAttachments.value.push({ id: crypto.randomUUID(), path: f.path, name: f.name, kind: f.kind || 'doc' })
          added++
        }
      }
    } catch (err) { console.warn('[Chat] clipboard read failed:', err) }
  }
}
function displayContent(content: string): string { const i = content.indexOf('<<<ATTACHMENTS>>>'); return i >= 0 ? content.slice(0, i).trimEnd() : content }
function extractAttachmentNames(content: string): string[] { const names: string[] = []; const re = /[\u3010]\u9644\u4ef6\uff1a(.+?)[\u3011]/g; let m; while ((m = re.exec(content)) !== null) names.push(m[1]); return names }

// ─── State ──────────────────────────────────────────────────
const isMac = typeof navigator !== 'undefined' && (
  navigator.platform.toUpperCase().indexOf('MAC') >= 0 ||
  navigator.userAgent.toUpperCase().indexOf('MAC') >= 0
)
const inputText = ref('')
const chatListRef = ref<HTMLElement | null>(null)
const composerRef = ref<HTMLTextAreaElement | null>(null)
const messages = ref<ChatMessage[]>([])
// 当前对话 id（全局指针，B 单流：任一窗口发送即切换；供分页与发送上下文使用）
const sessionId = ref('')

// ─── 快呼语义：召唤初始只展示上一轮问答（正常亮度），新问题/主动展开才见全历史 ───
const revealOlder = ref(false)
const lastRoundCount = ref(0)

/// 载入/复位时快照"最后一轮"（尾 assistant + 其前 user，或尾 user 单条）——初始仅显示这几条
function applyLastRoundSnapshot() {
  const ms = messages.value
  let count = 0
  if (ms.length) {
    const last = ms[ms.length - 1]
    if (last.role === 'assistant' && last.id) count = 1
    const back = ms[ms.length - 1 - count]
    if (count && back?.role === 'user' && back.id) count++
    if (!count && last.id) count = 1
  }
  lastRoundCount.value = count
}

const visibleMessages = computed(() => {
  if (revealOlder.value || !lastRoundCount.value) return messages.value
  return messages.value.slice(Math.max(0, messages.value.length - lastRoundCount.value))
})
const hasStream = computed(() => visibleMessages.value.length > 0 || agent.isChatting.value)

async function expandToOlder() {
  revealOlder.value = true
  // 展开全部：一次性分页拉完本会话更早的记录（每页 50，安全上限 20 页）
  if (!hasMore.value) { scrollChat(true); return }
  loadingMore.value = true
  try {
    let guard = 0
    while (hasMore.value && guard++ < 20) {
      const oldest = messages.value.find(m => !!(m as { dbId?: number }).dbId) as { dbId?: number } | undefined
      if (!oldest?.dbId) { hasMore.value = false; break }
      const result = await kbFabLoadHistory(sessionId.value || null, 50, oldest.dbId)
      const older = (result.success && result.messages) ? result.messages : []
      if (older.length) {
        messages.value.unshift(...older.map(mapHistoryMessage))
        for (const msg of older) {
          if (msg.role === 'assistant' && msg.content) {
            const key = msg.id as string
            agent.renderMd(msg.content).then(h => { if (h) agent.rendered.value[key] = h })
          }
        }
      }
      if (older.length < 50) hasMore.value = false
    }
  } catch (e) {
    console.error('[Chat] 展开全部消息失败:', e)
  }
  loadingMore.value = false
  scrollChat(true)
}

function autoGrow(e: Event) {
  const el = e.target as HTMLTextAreaElement
  el.style.height = 'auto'
  el.style.height = Math.min(el.scrollHeight, 84) + 'px'
}

function focusComposer() {
  nextTick(() => { composerRef.value?.focus() })
  // 召唤时序：chat:reset 先于 show()/set_focus() 发出，此刻的 focus 可能被 OS 吞掉，窗口真正出现后补一刀
  setTimeout(() => { composerRef.value?.focus({ preventScroll: true }) }, 150)
}

// ─── History ────────────────────────────────────────────────
const loadingMore = ref(false)
const hasMore = ref(true)

function mapHistoryMessage(m: any) {
  const docs = parseDocumentsMetadata(m.metadata)
  const clar = parseClarificationMetadata(m.metadata)
  let assistants: string[] | undefined
  try {
    const arr = JSON.parse(m.metadata)?.assistants
    if (Array.isArray(arr) && arr.length) assistants = arr.filter((x: any) => typeof x === 'string' && x)
  } catch { /* 非 JSON 忽略 */ }
  return {
    id: 'hist-' + m.id,
    dbId: m.id,
    role: m.role,
    content: m.content,
    timestamp: m.created_at ? new Date(m.created_at).getTime() : Date.now(),
    ...(docs ? { documents: docs } : {}),
    ...(clar ? { clarification: clar } : {}),
    ...(assistants ? { assistants } : {}),
  }
}

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

/// 加载当前对话历史（全局指针，后端兜底）——打开/召唤用，不做轮转判定
async function loadHistory() {
  try {
    const r = await kbFabLoadHistory(null, 10)
    sessionId.value = r.sessionId || ''
    const msgs = r.messages || []
    messages.value = msgs.map(mapHistoryMessage)
    hasMore.value = msgs.length >= 10
    applyLastRoundSnapshot()
    for (const msg of messages.value) {
      if (msg.role === 'assistant' && msg.content) {
        agent.renderMd(msg.content).then(h => { if (h && msg.id) agent.rendered.value[msg.id] = h })
      }
    }
  } catch (e) {
    console.error('[Chat] 加载历史失败:', e)
  }
  restorePlanDock()
}

// Dock 恢复路径：重启/切会话/新建会话后即时投影持久化计划（后端 session.plan 是唯一事实源，
// 此前只有下一次 run 启动才 emit——快照在库中但 Dock 看不见）。空闲才拉：run 中的快照由事件流自带。
// setRunSid('') 必须在 applyPlan 前：matchSid 归属过滤会拦截"新会话快照 vs 旧 run sid"（空闲清归属亦是切会话语义本身）。
async function restorePlanDock() {
  if (agent.isChatting.value) return
  agentRun.setRunSid('')
  const sid = sessionId.value || ''
  try {
    const r = await invoke<{ success: boolean; plan: { title: string; steps: any[] } | null }>('plan_get', { sid })
    agentRun.applyPlan({ sid, plan: r?.plan ?? null })
  } catch { /* 查询失败保持现状：下一次 run 的全量快照会纠正 */ }
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
    const result = await kbFabLoadHistory(sessionId.value || null, 10, oldestDbId)
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

// ★ 滚动跟随状态机：scrollChat 原为无条件吸底，流式输出期间用户完全滚不动；
//   距底 30px 内视为贴底，滚开即脱离跟随，用户主动动作（发送/停止/展开全部）强制回底
const stickToBottom = ref(true)
function onChatScroll() {
  const el = chatListRef.value
  if (!el) return
  stickToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 30
  if (el.scrollTop > 100) return
  loadOlderMessages()
}

function scrollChat(force = false) {
  if (force) stickToBottom.value = true
  nextTick(() => {
    const el = chatListRef.value
    if (el && stickToBottom.value) el.scrollTop = el.scrollHeight
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

// ─── 窗口控制 ──────────────────────────────────────────────
async function hideWindow() {
  try {
    await invoke('toggle_chat_window')  // 隐藏而非销毁，保留输入文本
  } catch (err) {
    console.error('[Chat] 隐藏失败:', err)
  }
}

// ─── 发送 ───────────────────────────────────────────────────
/// 面板清零（30min 轮转与显式新会话共用）
function resetPanel() {
  agent.stopAllTypewriters(messages.value)
  messages.value = []
  lastRoundCount.value = 0
  hasMore.value = false
}

/// 显式开新会话（不等续接窗口轮转）：**懒实体化**——后端只打标记不建行，
/// 下一条消息发送时才真正创建（点了不发消息不会堆积空白会话）；回答进行中不打断，旧回复落旧会话
async function newSession() {
  if (agent.isChatting.value) {
    import('@/composables/useToast').then(m => m.toast({ message: '当前回复将在原会话完成', type: 'info' }))
  }
  try {
    await fabNewSession()
    sessionId.value = ''
    resetPanel()
    revealOlder.value = true
    pendingAttachments.value = []
  } catch (e) { console.error('[Chat] 新建会话失败:', e) }
}

async function send() {
  const q = inputText.value.trim()
  if (!q || agent.isChatting.value) return
  // 附件分片上传中：等待完成（占位 chip 的 path 尚未就绪，直接发送会带上空路径）
  if (pendingAttachments.value.some(a => a.uploading)) {
    import('@/composables/useToast').then(m => m.toast({ message: '附件仍在上传，请稍候', type: 'error' }))
    return
  }

  // 发送前轮转判定：距上次提问超出续接窗口（设置可调，默认 10min；或会话已删/追问超时）→ 新会话并清面板
  try {
    const r = await fabResolveSession()
    if (r.rotated && messages.value.length) resetPanel()
    sessionId.value = r.sessionId
  } catch (e) { console.error('[Chat] 会话解析失败:', e) }

  // 新问题即展开全历史
  revealOlder.value = true

  agent.stopAllTypewriters(messages.value)
  agentRun.resetParticipants()
  agentRun.setRunSid(sessionId.value || '')
  const atts = [...pendingAttachments.value]
  inputText.value = ''; pendingAttachments.value = []
  nextTick(() => { if (composerRef.value) composerRef.value.style.height = 'auto' })

  const userMsg: ChatMessage = { id: crypto.randomUUID(), role: 'user', content: q, attachments: atts.length ? atts as any : undefined, timestamp: Date.now() }
  messages.value.push(userMsg)

  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true, toolCalls: [], rawToolCalls: [], toolResults: [], timestamp: Date.now() }
  messages.value.push(aiMsg)
  while (messages.value.length > 20) { messages.value.shift() }
  scrollChat(true)

  await agent.startAgentChat(q, {
    getMessage: () => aiMsg,
    scrollFn: scrollChat,
    sessionId: sessionId.value || 'fab-main-session',
    attachments: atts.map(a => ({ path: a.path, name: a.name })),
    onDone: (result, ctx) => {
      if (ctx.finalAnswer && ctx.finalAnswer.trim()) {
        aiMsg.toolCalls = result.tool_log || []
        const docs = extractDocumentsFromToolLog(result.tool_log)
        if (docs.length) aiMsg.documents = docs
        // ★ 追问卡片：done 载荷携带结构化 clarification（正文已是文本版，双轨）
        if (result.clarification && Array.isArray(result.clarification.questions) && result.clarification.questions.length) {
          aiMsg.clarification = result.clarification
        }
        // ★ via 署名：本轮参与过的子 Agent
        if (agentRun.participants.length) aiMsg.assistants = [...agentRun.participants]
      }
    },
    onError: (error) => {
      aiMsg.content = error
    },
  })
}

/// 追问卡片提交：合成答复文本走正常发送管线（send 自带忙碌守卫）
function submitClarification(text: string) {
  inputText.value = text
  send()
}

/// 已答复卡片的答案视图数据源：该消息之后最近一条用户消息正文（刷新/重开会话仍可解析）
function clarificationReply(msgId?: string): string {
  if (!msgId) return ''
  const i = messages.value.findIndex(x => x.id === msgId)
  if (i < 0) return ''
  return messages.value.slice(i + 1).find(x => x.role === 'user')?.content || ''
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

// ─── 撤回：任意用户消息起，其后全部移出会话与上下文；可选连带删除生成文件 ───
const withdrawTarget = ref<{ text: string; files: WithdrawnFile[]; idx: number; nth: number; attachments: any[] } | null>(null)
function askWithdraw(anchor: ChatMessage) {
  if (!anchor || anchor.role !== 'user') return
  // visibleMessages 可能是尾部切片：截断点按全量 messages 的真实下标计算
  const idx = messages.value.findIndex(x => x.id === anchor.id)
  if (idx < 0) return
  const files = messages.value.slice(idx + 1).flatMap(m => (m.documents || []).map(d => ({ id: d.file_id, name: d.file_name })))
  // 同文本序号：后端锚点按"第 n 条同文本 active 消息"定位（历史行含 ATTACHMENTS 后缀，按基底文本比对）
  const base = (c: string) => (c || '').split('\n\n<<<ATTACHMENTS>>>')[0]
  const text = base(anchor.content || '')
  const nth = messages.value.slice(0, idx + 1).filter(m => m.role === 'user' && base(m.content || '') === text).length - 1
  withdrawTarget.value = { text, files, idx, nth, attachments: (anchor as any).attachments || [] }
}
async function copyUserMsg(m: ChatMessage) {
  try { await navigator.clipboard.writeText((m.content || '').trim()); import('@/composables/useToast').then(t => t.toast({ message: '已复制', type: 'success' })) }
  catch { import('@/composables/useToast').then(t => t.toast({ message: '复制失败', type: 'error' })) }
}
async function doWithdraw(deleteFiles: boolean) {
  const t = withdrawTarget.value
  if (!t) return
  withdrawTarget.value = null
  try {
    const r = await kbWithdrawMessages(sessionId.value || '', t.text, deleteFiles, t.nth)
    if (r.success) {
      messages.value = messages.value.slice(0, t.idx)
      exitAnnotate()
      import('@/composables/useToast').then(m => m.toast({ message: `已撤回 ${r.withdrawn_count} 条消息${r.files_deleted ? `，删除 ${r.files_deleted} 个文件` : ''}`, type: 'success' }))
      // 撤回回喂：修改后一键重发
      if (t.text) {
        inputText.value = t.text
        if (t.attachments?.length) pendingAttachments.value = t.attachments
        nextTick(() => { const el = composerRef.value; if (el) { el.focus(); el.style.height = 'auto'; el.style.height = Math.min(el.scrollHeight, 80) + 'px' } })
      }
    } else {
      import('@/composables/useToast').then(m => m.toast({ message: r.error || '撤回失败：范围为空', type: 'error' }))
    }
  } catch (e: any) {
    import('@/composables/useToast').then(m => m.toast({ message: `撤回失败：${e?.message || e}`, type: 'error' }))
  }
}
function enterAnnotate(id: string) {
  if (agent.isChatting.value) return
  annotations.length = 0
  tableRevisions.value = []
  annotateId.value = id
  // 批注面板在消息尾部展开，先归队到底确保可见
  stickToBottom.value = true
  nextTick(() => { const el = chatListRef.value; if (el) el.scrollTop = el.scrollHeight })
}
function exitAnnotate() {
  annotateId.value = null
  annotations.length = 0
  tableRevisions.value = []
}
function submitAnnotation() {
  const tables = mbRefs[annotateId.value || '']?.getRevisions() ?? []
  const text = buildRevisionMessage(tables, [...annotations])
  if (!text) { import('@/composables/useToast').then(m => m.toast({ message: '尚无修订 —— 改表格或选中文字提出要求', type: 'info' })); return }
  exitAnnotate()
  submitClarification(text)
}
// 开始流式回复即退出批注（编辑态不跨轮残留）；切换/新建会话同理
watch(() => agent.isChatting.value, (v) => { if (v) exitAnnotate() })
watch(sessionId, () => { exitAnnotate(); restorePlanDock() })

/// 输入框键盘契约：⏎ 发送 / ⇧⏎ 换行（textarea 默认）/ Esc 收起窗口
function onComposerKey(e: KeyboardEvent) {
  if (e.key === 'Escape') { e.preventDefault(); hideWindow(); return }
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() }
}

async function stopAgent() {
  await agent.stopAgent(() => messages.value, sessionId.value)
  scrollChat(true)
}

// ─── Lifecycle ──────────────────────────────────────────────
let unlistenReset: (() => void) | null = null
let unlistenFocus: (() => void) | null = null

onMounted(async () => {
  await loadHistory()

  // 召唤/收起复位（Rust 在 show/hide 前发）：重拉"当前对话"（承接 B 单流指针切换与
  // 列表侧删除；流式中跳过——进行中的 aiMsg 仍挂在 messages 上），再回淡显态并聚焦
  unlistenReset = await listen('chat:reset', async () => {
    if (!agent.isChatting.value) await loadHistory()
    revealOlder.value = false
    applyLastRoundSnapshot()
    await nextTick()
    focusComposer()
  })
  // 窗口获得焦点：只聚焦输入框——打开永不轮转（Gemini 手感，提问才切会话）
  try {
    unlistenFocus = await getCurrentWebviewWindow().onFocusChanged(({ payload }) => {
      if (payload) focusComposer()
    })
  } catch {}
  // 首开兜底：chat 窗口刚创建时 listen 尚未注册，Rust 的 chat:reset 会丢失——mount 完成后主动聚焦
  focusComposer()
})

onUnmounted(() => {
  unlistenReset?.()
  unlistenFocus?.()
  if (elapsedTimer) clearInterval(elapsedTimer)
})
</script>

<template>
<!-- 全窗口透明覆盖层，点击空白处关闭（保留输入文本） -->
<div class="chat-overlay" @click.self="hideWindow">
  <div class="chat-panel">
    <!-- 对话记录（输入上方，就地向上生长；高度动画全在 CSS，窗口零位移） -->
    <transition name="stream">
      <div v-if="hasStream" class="chat-stream">
        <button v-if="!revealOlder && (hasMore || messages.length > lastRoundCount)" class="older-btn" @click="expandToOlder">↑ 展开全部对话</button>
        <div ref="chatListRef" class="messages-scroll" @scroll="onChatScroll">
          <div v-if="loadingMore" class="text-center py-1">
            <span class="text-[10px] text-[#94a3b8]">加载中...</span>
          </div>
          <div v-for="(m, mi) in visibleMessages" :key="m.id || mi" class="msg-block">
            <div v-if="m.role === 'user'" class="msg-q">
              <span class="msg-q__who">我</span><span class="msg-q__text">{{ displayContent(m.content) }}</span>
              <AttachmentChips v-if="m.attachments?.length" :files="m.attachments" class="mt-1" />
              <div v-else-if="m.content?.includes('<<<ATTACHMENTS>>>')" class="flex flex-wrap gap-1 mt-1">
                <span v-for="name in extractAttachmentNames(m.content)" :key="name" class="px-1.5 py-0.5 rounded text-[10px] bg-[#f1f5f9] text-[#64748b]">{{ name }}</span>
              </div>
            </div>
            <div v-else class="msg-a">
              <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
              <!-- 委托轨迹行：子 Agent 执行中且正文尚未流出时，顶替"思考中"占位 -->
              <div v-else-if="m.thinking && !agent.rendered.value[m.id || ''] && agentRun.currentAgent" class="flex items-center gap-1.5 py-0.5">
                <span class="w-1.5 h-1.5 rounded-full bg-brand-500 animate-pulse flex-shrink-0"></span>
                <span class="text-[12px] font-medium text-[#3a3a3c]">{{ agentRun.currentAgent.name }}</span>
                <span class="text-[12px] text-[#8e8e93]">· {{ wProgress || '处理中' }}{{ elapsedSec > 0 ? ` · 已用 ${fmtElapsed()}` : '' }}</span>
              </div>
              <AnnotationLayer v-else-if="annotateId === (m.id || '')" class="msg-a__body" :active="true" @add="annotations.push($event)">
                <MessageBody :raw="m.content" :msg-id="m.id || ''" :annotate="true" :ref="(el: any) => { mbRefs[m.id || ''] = el }" @send="submitClarification" @revision-change="(r: any) => { if (annotateId === (m.id || '')) tableRevisions = r }" />
              </AnnotationLayer>
              <MessageBody v-else-if="!m.thinking && hasInteractive(m.content)" class="msg-a__body" :raw="m.content" :msg-id="m.id || ''" @send="submitClarification" />
              <div v-else class="msg-a__body markdown-body" :class="{ 'thinking-bubble': m.thinking }"
                v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
              <!-- 思考流（本轮瞬态：流式滚动自动跟底，正文开始折叠为一行可展开） -->
              <div v-if="agent.thought.value && (m.id || '') === agent.thoughtMsgId.value" class="mt-1">
                <div v-if="!agent.thoughtCollapsed.value" ref="thoughtBoxRef"
                  class="max-h-[80px] overflow-y-auto rounded-lg bg-surface-50 border border-surface-100 px-2.5 py-1.5 text-[11px] leading-relaxed text-surface-400 whitespace-pre-wrap break-words">{{ agent.thought.value }}</div>
                <template v-else>
                  <button type="button" class="flex items-center gap-1 text-[10px] text-surface-300 hover:text-brand-600 transition-colors" @click="thoughtExpanded = !thoughtExpanded">
                    <svg class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path :d="thoughtExpanded ? 'M6 9l6 6 6-6' : 'M9 6l6 6-6 6'"/></svg>
                    思考了 {{ agent.thoughtSecs.value }} 秒 · {{ agent.thought.value.length.toLocaleString() }} 字
                  </button>
                  <div v-if="thoughtExpanded" ref="thoughtBoxRef"
                    class="mt-1 max-h-[130px] overflow-y-auto rounded-lg bg-surface-50 border border-surface-100 px-2.5 py-1.5 text-[11px] leading-relaxed text-surface-400 whitespace-pre-wrap break-words">{{ agent.thought.value }}</div>
                </template>
              </div>
              <div v-if="!m.thinking" class="msg-a__meta">
                <span>{{ fmtTime(m.timestamp) }}</span>
                <span v-if="m.assistants?.length" class="via-chip">via {{ m.assistants.join(' · ') }}</span>
                <button v-if="m.id === messages[messages.length - 1]?.id && !agent.isChatting.value && !m.error"
                  :class="['annotate-skip w-5 h-5 -mr-1 flex items-center justify-center rounded transition-colors', annotateId === (m.id || '') ? 'text-brand-600 bg-brand-50' : 'text-surface-300 hover:text-brand-600']"
                  v-tooltip="annotateId === (m.id || '') ? '取消修订' : '修订'"
                  @click="annotateId === (m.id || '') ? exitAnnotate() : enterAnnotate(m.id || '')">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M16.9 4.4l2.7 2.7-11 11-3.6.9.9-3.6 11-11z"/><path d="M14.5 6.8l2.7 2.7"/></svg>
                </button>
              </div>
              <AnnotatePanel v-if="annotateId === (m.id || '')" class="annotate-skip mt-1" :items="annotations" :tables="tableRevisions"
                @remove="annotations.splice($event, 1)" @submit="submitAnnotation" @cancel="exitAnnotate"
                @reset-table="mbRefs[annotateId || '']?.resetTableRevision($event)" />
              <div v-if="!m.thinking && m.documents?.length" class="mt-1 flex flex-col gap-1.5">
                <DocumentObject v-for="d in m.documents" :key="d.file_id" :doc="d" />
              </div>
              <div v-if="!m.thinking && m.clarification" class="mt-1">
                <ClarifyCard
                  :clarification="m.clarification"
                  :answered="m.id !== messages[messages.length - 1]?.id"
                  :reply-text="clarificationReply(m.id)"
                  @submit="submitClarification"
                />
              </div>
            </div>
            <div v-if="m.role === 'user' && !agent.isChatting.value && !m.error" class="flex justify-end items-center gap-0.5 px-0.5 -mt-0.5">
              <button class="w-5 h-5 flex items-center justify-center rounded text-surface-300 hover:text-red-500 transition-colors" v-tooltip="'撤回：移除此后全部对话'" @click="askWithdraw(m)">
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14L4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 010 11H11"/></svg>
              </button>
              <button class="w-5 h-5 flex items-center justify-center rounded text-surface-300 hover:text-brand-600 transition-colors" v-tooltip="'复制'" @click="copyUserMsg(m)">
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 012-2h10"/></svg>
              </button>
            </div>
          </div>
        </div>
      </div>
    </transition>

    <!-- 执行计划 Dock（输入卡上方，滚动不离场；后端 agent:plan 事件驱动） -->
    <PlanDock :running="agent.isChatting.value" />
    <!-- 输入卡（底部常驻） -->
    <div class="composer">
      <div v-if="pendingAttachments.length" class="composer-attach">
        <AttachmentChips :files="pendingAttachments" removable @remove="removeAttachment" />
      </div>
      <textarea ref="composerRef" v-model="inputText" rows="1" class="composer-input"
        placeholder="问点什么…"
        @input="autoGrow" @keydown="onComposerKey" @paste="onPaste"></textarea>
      <div class="composer-bar">
        <div class="flex items-center gap-1">
          <button @click.stop="newSession" class="icon-btn" v-tooltip="'开始新会话'">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/></svg>
          </button>
          <button v-if="!isMac" @click.stop="openAttachmentDialog" class="icon-btn" v-tooltip="'附加文件'">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg>
          </button>
        </div>
        <span class="composer-hint">⏎ 发送 · ⇧⏎ 换行 · Esc 关闭</span>
        <div class="flex items-center gap-1">
          <button v-if="agent.isChatting.value" @click.stop="stopAgent" class="icon-btn icon-btn--stop" v-tooltip="'停止'">
            <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
          </button>
          <button v-else @click.stop="send" :disabled="!inputText.trim()" class="send-btn" :class="{ 'send-btn--on': inputText.trim() }">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5"/></svg>
          </button>
        </div>
      </div>
    </div>
  </div>
  <WithdrawDialog :visible="!!withdrawTarget" :anchor-text="withdrawTarget?.text || ''" :files="withdrawTarget?.files || []"
    @confirm="doWithdraw" @cancel="withdrawTarget = null" />
</div>
</template>

<style scoped>
/* ═══ 全窗口透明覆盖层（面板外的透明区点击即收起） ═══ */
.chat-overlay {
  width: 100vw;
  height: 100vh;
  background: transparent;
}

/* ═══ 快呼面板：锚定窗口底部（输入在下、对话向上生长；窗口零位移，生长全在 CSS） ═══ */
.chat-panel {
  position: absolute;
  left: 50%; transform: translateX(-50%);
  bottom: 52px;
  width: 580px;
  max-height: 616px;
  display: flex; flex-direction: column;
  background: rgba(255,255,255,0.94);
  backdrop-filter: blur(24px);
  -webkit-backdrop-filter: blur(24px);
  border-radius: 18px;
  border: 1px solid rgba(15,23,42,0.06);
  box-shadow: 0 24px 70px -18px rgba(15,23,42,0.22), 0 10px 30px -12px rgba(15,23,42,0.12);
  overflow: hidden;
  user-select: none;
}

/* ═══ 输入卡（底部常驻） ═══ */
.composer { flex: 0 0 auto; }
.composer-attach { padding: 10px 14px 0; }
.composer-input {
  width: 100%; border: none; outline: none; resize: none;
  font-size: 13.5px; line-height: 1.55; color: #1f2937;
  padding: 14px 16px 2px; min-height: 26px; max-height: 84px;
  background: transparent; font-family: inherit; user-select: text;
}
.composer-input::placeholder { color: #b6bcc7; font-size: 13px; }
.composer-bar {
  display: flex; align-items: center; justify-content: space-between;
  padding: 6px 10px 10px 14px;
}
.composer-hint { font-size: 10px; color: #c3c7ce; letter-spacing: 0.02em; }

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

.send-btn {
  width: 28px; height: 28px; border-radius: 8px;
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0; border: none; cursor: pointer;
  background: linear-gradient(135deg, #e2e8f0, #f1f5f9); color: #94a3b8; transition: all 0.2s;
}
.send-btn--on { background: linear-gradient(135deg, #06b6d4, #2563eb); color: white; box-shadow: 0 1px 8px rgba(37, 99, 235, 0.3); }
.send-btn--on:hover { box-shadow: 0 2px 12px rgba(37, 99, 235, 0.45); transform: scale(1.05); }

/* ═══ 对话区（输入上方，就地向上生长） ═══ */
.chat-stream {
  display: flex; flex-direction: column;
  max-height: 516px;
  border-bottom: 1px solid rgba(15,23,42,0.05);
}
.older-btn {
  margin: 10px auto 0; padding: 0 6px; flex-shrink: 0; width: fit-content;
  font-size: 10px; color: #b6bcc7; cursor: pointer;
  background: none; border: none; border-radius: 6px;
  transition: color 0.15s;
}
.older-btn:hover { color: #2563eb; }

.messages-scroll {
  overflow-y: auto; overflow-x: hidden;
  padding: 6px 14px 14px;
  user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(37, 99, 235, 0.2) transparent;
}
.messages-scroll::-webkit-scrollbar { width: 4px; }
.messages-scroll::-webkit-scrollbar-track { background: transparent; }
.messages-scroll::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

/* ═══ 紧凑问答排版（无头像无气泡） ═══ */
.msg-block { margin-bottom: 14px; animation: fadeIn 0.25s ease-out; }
.msg-block:last-child { margin-bottom: 2px; }

.msg-q { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 0; font-size: 12px; color: #64748b; line-height: 1.55; word-break: break-word; }
.msg-q__who { font-size: 10px; color: #a5adb8; background: #f1f5f9; border-radius: 4px; padding: 0 5px; margin-right: 7px; flex-shrink: 0; }
.msg-q__text { white-space: pre-wrap; font-family: inherit; }

.msg-a { color: #1f2937; }
.msg-a__body { font-size: 12.5px; line-height: 1.55; }
.msg-a__meta { display: flex; align-items: center; gap: 6px; margin-top: 4px; font-size: 9px; color: #c7c7cc; }
.via-chip { padding: 1px 6px; border-radius: 999px; background: #f2f2f7; color: #8e8e93; line-height: 1.4; }


/* ═══ 面板生长/淡入 ═══ */
.stream-enter-active { transition: opacity 0.28s ease, transform 0.28s cubic-bezier(0.33, 1, 0.68, 1); }
.stream-enter-from { opacity: 0; transform: translateY(-6px); }
.stream-leave-active { transition: none; }

/* ═══ Markdown ═══ */
.chat-md { font-size: 12.5px; line-height: 1.5; }
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
