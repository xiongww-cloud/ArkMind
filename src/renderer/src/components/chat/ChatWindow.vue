<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { fabResolveSession, kbFabLoadHistory } from '@/api/db-client'
import { useAgentChat, type ChatMessage, extractDocumentsFromToolLog, parseDocumentsMetadata, parseClarificationMetadata } from '@/composables/useAgentChat'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'
import DocumentObject from '@/components/chat/DocumentObject.vue'
import ClarifyCard from '@/components/chat/ClarifyCard.vue'
import { uploadPasteFile } from '@/utils/pasteUpload'
import { useAgentRunStore } from '@/stores/agentRun'

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

// ─── 快呼语义：召唤只淡显上一轮问答，新问题/主动展开才见全历史 ───
const revealOlder = ref(false)
const fadedIds = ref<Set<string>>(new Set())
const lastRoundCount = ref(0)

/// 载入/复位时快照"最后一轮"（尾 assistant + 其前 user，或尾 user 单条）作淡显对象
function applyLastRoundFaint() {
  const ms = messages.value
  const ids = new Set<string>()
  let count = 0
  if (ms.length) {
    const last = ms[ms.length - 1]
    if (last.role === 'assistant' && last.id) { ids.add(last.id); count = 1 }
    const back = ms[ms.length - 1 - count]
    if (count && back?.role === 'user' && back.id) { ids.add(back.id); count++ }
    if (!count && last.id) { ids.add(last.id); count = 1 }
  }
  fadedIds.value = ids
  lastRoundCount.value = count
}

const visibleMessages = computed(() => {
  if (revealOlder.value || !lastRoundCount.value) return messages.value
  return messages.value.slice(Math.max(0, messages.value.length - lastRoundCount.value))
})
const hasStream = computed(() => visibleMessages.value.length > 0 || agent.isChatting.value)
function isFaded(m: ChatMessage): boolean { return !revealOlder.value && !!m.id && fadedIds.value.has(m.id) }

async function expandToOlder() {
  revealOlder.value = true
  // 展开全部：一次性分页拉完本会话更早的记录（每页 50，安全上限 20 页）
  if (!hasMore.value) { scrollChat(); return }
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
  scrollChat()
}

function autoGrow(e: Event) {
  const el = e.target as HTMLTextAreaElement
  el.style.height = 'auto'
  el.style.height = Math.min(el.scrollHeight, 84) + 'px'
}

function focusComposer() {
  nextTick(() => { composerRef.value?.focus() })
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

/// 加载当前对话历史（全局指针，后端兜底）——打开/召唤用，不做轮转判定
async function loadHistory() {
  try {
    const r = await kbFabLoadHistory(null, 10)
    sessionId.value = r.sessionId || ''
    const msgs = r.messages || []
    messages.value = msgs.map(mapHistoryMessage)
    hasMore.value = msgs.length >= 10
    applyLastRoundFaint()
    for (const msg of messages.value) {
      if (msg.role === 'assistant' && msg.content) {
        agent.renderMd(msg.content).then(h => { if (h && msg.id) agent.rendered.value[msg.id] = h })
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

// ─── 窗口控制 ──────────────────────────────────────────────
async function hideWindow() {
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
  // 附件分片上传中：等待完成（占位 chip 的 path 尚未就绪，直接发送会带上空路径）
  if (pendingAttachments.value.some(a => a.uploading)) {
    import('@/composables/useToast').then(m => m.toast({ message: '附件仍在上传，请稍候', type: 'error' }))
    return
  }

  // 发送前轮转判定（唯一判定点）：距上次提问 >30min（或会话已删/追问超时）→ 新会话并清面板
  try {
    const r = await fabResolveSession()
    if (r.rotated && messages.value.length) {
      agent.stopAllTypewriters(messages.value)
      messages.value = []
      fadedIds.value = new Set()
      lastRoundCount.value = 0
      hasMore.value = false
    }
    sessionId.value = r.sessionId
  } catch (e) { console.error('[Chat] 会话解析失败:', e) }

  // 新问题即展开全历史并解除淡显
  revealOlder.value = true
  fadedIds.value = new Set()

  agent.stopAllTypewriters(messages.value)
  agentRun.resetParticipants()
  const atts = [...pendingAttachments.value]
  inputText.value = ''; pendingAttachments.value = []
  nextTick(() => { if (composerRef.value) composerRef.value.style.height = 'auto' })

  const userMsg: ChatMessage = { id: crypto.randomUUID(), role: 'user', content: q, attachments: atts.length ? atts as any : undefined, timestamp: Date.now() }
  messages.value.push(userMsg)

  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true, toolCalls: [], rawToolCalls: [], toolResults: [], timestamp: Date.now() }
  messages.value.push(aiMsg)
  while (messages.value.length > 20) { messages.value.shift() }
  scrollChat()

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

/// 输入框键盘契约：⏎ 发送 / ⇧⏎ 换行（textarea 默认）/ Esc 收起窗口
function onComposerKey(e: KeyboardEvent) {
  if (e.key === 'Escape') { e.preventDefault(); hideWindow(); return }
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() }
}

async function stopAgent() {
  await agent.stopAgent(() => messages.value)
  scrollChat()
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
    applyLastRoundFaint()
    await nextTick()
    focusComposer()
  })
  // 窗口获得焦点：只聚焦输入框——打开永不轮转（Gemini 手感，提问才切会话）
  try {
    unlistenFocus = await getCurrentWebviewWindow().onFocusChanged(({ payload }) => {
      if (payload) focusComposer()
    })
  } catch {}
})

onUnmounted(() => {
  unlistenReset?.()
  unlistenFocus?.()
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
            <div v-if="m.role === 'user'" class="msg-q" :class="{ faded: isFaded(m) }">
              <span class="msg-q__who">我</span><span class="msg-q__text">{{ displayContent(m.content) }}</span>
              <AttachmentChips v-if="m.attachments?.length" :files="m.attachments" class="mt-1" />
              <div v-else-if="m.content?.includes('<<<ATTACHMENTS>>>')" class="flex flex-wrap gap-1 mt-1">
                <span v-for="name in extractAttachmentNames(m.content)" :key="name" class="px-1.5 py-0.5 rounded text-[10px] bg-[#f1f5f9] text-[#64748b]">{{ name }}</span>
              </div>
            </div>
            <div v-else class="msg-a" :class="{ faded: isFaded(m) }">
              <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
              <!-- 委托轨迹行：子 Agent 执行中且正文尚未流出时，顶替"思考中"占位 -->
              <div v-else-if="m.thinking && !agent.rendered.value[m.id || ''] && agentRun.currentAgent" class="flex items-center gap-1.5 py-0.5">
                <span class="w-1.5 h-1.5 rounded-full bg-brand-500 animate-pulse flex-shrink-0"></span>
                <span class="text-[12px] font-medium text-[#3a3a3c]">{{ agentRun.currentAgent.name }}</span>
                <span class="text-[12px] text-[#8e8e93]">· 处理中</span>
              </div>
              <div v-else class="msg-a__body markdown-body" :class="{ 'thinking-bubble': m.thinking }"
                v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
              <div v-if="!m.thinking" class="msg-a__meta">
                <span>{{ fmtTime(m.timestamp) }}</span>
                <span v-if="m.assistants?.length" class="via-chip">via {{ m.assistants.join(' · ') }}</span>
              </div>
              <div v-if="!m.thinking && m.documents?.length" class="mt-1 flex flex-col gap-1.5">
                <DocumentObject v-for="d in m.documents" :key="d.file_id" :doc="d" />
              </div>
              <div v-if="!m.thinking && m.clarification" class="mt-1">
                <ClarifyCard
                  :clarification="m.clarification"
                  :answered="m.id !== messages[messages.length - 1]?.id"
                  @submit="submitClarification"
                />
              </div>
            </div>
          </div>
        </div>
      </div>
    </transition>

    <!-- 输入卡（底部常驻） -->
    <div class="composer">
      <div v-if="pendingAttachments.length" class="composer-attach">
        <AttachmentChips :files="pendingAttachments" removable @remove="removeAttachment" />
      </div>
      <textarea ref="composerRef" v-model="inputText" rows="1" class="composer-input"
        placeholder="问点什么…"
        @input="autoGrow" @keydown="onComposerKey" @paste="onPaste"></textarea>
      <div class="composer-bar">
        <span class="composer-hint">⏎ 发送 · ⇧⏎ 换行 · Esc 关闭</span>
        <div class="flex items-center gap-1">
          <button v-if="!isMac" @click.stop="openAttachmentDialog" class="icon-btn" v-tooltip="'附加文件'">
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg>
          </button>
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

.faded { opacity: 0.42; }

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
