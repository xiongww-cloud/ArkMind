<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { kbFabInitSession, kbFabLoadHistory } from '@/api/db-client'
import { useAgentChat, type ChatMessage, extractDocumentsFromToolLog, parseDocumentsMetadata } from '@/composables/useAgentChat'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'
import DocumentObject from '@/components/chat/DocumentObject.vue'
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
  // 收起态添加附件后自动展开，以便显示附件行
  if (added > 0 && !expanded.value) expandChat()
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
  if (!expanded.value) expandChat()
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
  const docs = parseDocumentsMetadata(m.metadata)
  return {
    id: 'hist-' + m.id,
    dbId: m.id,
    role: m.role,
    content: m.content,
    timestamp: m.created_at ? new Date(m.created_at).getTime() : Date.now(),
    ...(docs ? { documents: docs } : {}),
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

  // 未展开则并行展开（不 await）
  if (!expanded.value) expandChat()

  agent.stopAllTypewriters(messages.value)
  const atts = [...pendingAttachments.value]
  inputText.value = ''; pendingAttachments.value = []

  const userMsg: ChatMessage = { id: crypto.randomUUID(), role: 'user', content: q, attachments: atts.length ? atts as any : undefined, timestamp: Date.now() }
  messages.value.push(userMsg)

  const aiMsg: ChatMessage = { id: crypto.randomUUID(), role: 'assistant', content: '', thinking: true, toolCalls: [], rawToolCalls: [], toolResults: [], timestamp: Date.now() }
  messages.value.push(aiMsg)
  while (messages.value.length > 20) { messages.value.shift() }
  scrollChat()

  await agent.startAgentChat(q, {
    getMessage: () => aiMsg,
    scrollFn: scrollChat,
    sessionId: 'fab-main-session',
    attachments: atts.map(a => ({ path: a.path, name: a.name })),
    onDone: (result, ctx) => {
      if (ctx.finalAnswer && ctx.finalAnswer.trim()) {
        aiMsg.toolCalls = result.tool_log || []
        const docs = extractDocumentsFromToolLog(result.tool_log)
        if (docs.length) aiMsg.documents = docs
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
              <pre class="bubble-user__text">{{ displayContent(m.content) }}</pre>
              <AttachmentChips v-if="m.attachments?.length" :files="m.attachments" class="mt-1" />
              <div v-else-if="m.content?.includes('<<<ATTACHMENTS>>>')" class="flex flex-wrap gap-1 mt-1">
                <span v-for="name in extractAttachmentNames(m.content)" :key="name" class="flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] bg-white/20 text-white/90">{{ name }}</span>
              </div>
            </div>
            <div v-else class="bubble-ai" :class="{ 'thinking-bubble': m.thinking }">
              <div v-if="m.error" class="text-[11px] text-red-500">{{ m.error }}</div>
              <div v-else class="markdown-body chat-md" v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || agent.thinkingProgress.value || '思考中'"></div>
            </div>
            <div v-if="m.role !== 'user' && !m.thinking && m.documents?.length" class="mt-1 flex flex-col gap-1.5">
              <DocumentObject v-for="d in m.documents" :key="d.file_id" :doc="d" />
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 附件行（独立于输入栏，显示在消息区与输入栏之间） -->
    <div v-if="pendingAttachments.length" class="chat-attachments">
      <AttachmentChips :files="pendingAttachments" removable @remove="removeAttachment" />
    </div>

    <!-- 委托状态条：显示当前干活的子 Agent（agent:status 结构化身份驱动） -->
    <div v-if="agent.isChatting.value && agentRun.currentAgent" class="chat-attachments !py-1 !px-2">
      <div class="flex items-center gap-2 px-2 py-1 rounded-lg bg-brand-50/70 border border-brand-100 text-[11px] text-brand-700">
        <span class="w-1.5 h-1.5 rounded-full bg-brand-500 animate-pulse flex-shrink-0"></span>
        <span class="font-medium">{{ agentRun.currentAgent.name }}</span>
        <span class="text-brand-500">处理中...</span>
      </div>
    </div>

    <!-- 输入栏（始终可见，面板底部） -->
    <div class="chat-input-bar"
      @pointerdown="onPointerDown" @pointermove="onPointerMove" @pointerup="onPointerUp">
      <div class="flex items-center gap-1 flex-1 min-w-0">
        <button @click.stop="expanded ? collapseChat() : expandChat()" class="icon-btn" v-tooltip="expanded ? '收起' : '展开'">
          <svg class="w-3.5 h-3.5 transition-transform" :class="{ 'rotate-180': expanded }" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M18 15l-6-6-6 6"/></svg>
        </button>
        <button v-if="!isMac" @click.stop="openAttachmentDialog" class="icon-btn" v-tooltip="'附加文件'">
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13"/></svg>
        </button>
        <input v-model="inputText" :placeholder="expanded ? '...' : currentSpeechText" class="chat-input"
          v-tooltip="isMac ? '支持 Cmd+V 粘贴图片/文件' : ''"
          @keydown.enter.exact.prevent="send" @paste="onPaste" />
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
  overflow-x: hidden;
  padding: 12px;
  padding-bottom: 44px;  /* 预留附件行空间，避免最后一条消息被附件行遮挡 */
  user-select: text;
  scrollbar-width: thin;
  scrollbar-color: rgba(37, 99, 235, 0.2) transparent;
}
.messages-scroll::-webkit-scrollbar { width: 4px; }
.messages-scroll::-webkit-scrollbar-track { background: transparent; }
.messages-scroll::-webkit-scrollbar-thumb { background: linear-gradient(180deg, #06b6d4, #6366f1); border-radius: 4px; }

/* ═══ 附件行（绝对定位，悬浮于输入栏上方，不参与 flex 撑高） ═══ */
.chat-attachments {
  position: absolute;
  left: 0; right: 0;
  bottom: 48px;          /* 输入栏高度之上 */
  display: flex;
  padding: 6px 8px;
  border-top: 1px solid rgba(0,0,0,0.04);
  background: rgba(255,255,255,0.95);
}

/* ═══ 输入栏（面板底部，flex 子元素） ═══ */
.chat-input-bar {
  flex: 0 0 auto;
  width: 580px; min-height: 48px;
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
