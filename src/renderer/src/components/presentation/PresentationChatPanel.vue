<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { useAgentChat, renderMd } from '@/composables/useAgentChat'
import type { ChatMessage } from '@/composables/useAgentChat'
import { usePresentationsStore } from '@/stores/presentations'
import { isTauri } from '@/utils/presentationAi'
import AttachmentChips from '@/components/chat/AttachmentChips.vue'

const props = defineProps<{
  pid: string
  deckId: string
}>()

const emit = defineEmits<{
  (e: 'page-generated', page: number, html: string): void
  (e: 'page-updated', page: number, html: string): void
}>()

const store = usePresentationsStore()
const p = computed(() => store.getById(props.pid))
const deckReady = computed(() => p.value?.status === 'generated' && (p.value?.htmlContent || '').length > 0)

// ── useAgentChat（复用流式处理+渲染） ──
const agent = useAgentChat()
const messagesRef = ref<HTMLElement | null>(null)

function scrollFn() {
  nextTick(() => {
    const el = messagesRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
}

// ── 进度状态 ──
const progressMsg = ref('')
const progressPage = ref(0)

// ── 输入 ──
const input = ref('')

// ── 附件 ──
interface PendingAttachment {
  path: string
  name: string
  kind: 'image' | 'doc'
  previewUrl?: string
}
const pendingAttachments = ref<PendingAttachment[]>([])
const MAX_ATTACH = 5
const IMG_EXTS = new Set(['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'])
const DOC_EXTS = new Set(['pdf', 'docx', 'xlsx', 'xls', 'pptx', 'ppt', 'txt', 'md', 'csv', 'json'])
const ALL_EXTS = new Set([...IMG_EXTS, ...DOC_EXTS])

function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 ? name.slice(i + 1).toLowerCase() : ''
}

async function openAttachmentDialog() {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const result = await open({
    multiple: true,
    canCreateDirectories: false,
    filters: [{ name: '文档与图片', extensions: [...ALL_EXTS] }]
  })
  const paths = result ? (Array.isArray(result) ? result : [result]) : []
  for (const pathStr of paths) {
    if (pendingAttachments.value.length >= MAX_ATTACH) break
    const name = pathStr.replace(/^.*[\\\/]/, '')
    const ext = extOf(name)
    if (!ALL_EXTS.has(ext)) continue
    if (pendingAttachments.value.some((a) => a.path === pathStr)) continue
    const kind: 'image' | 'doc' = IMG_EXTS.has(ext) ? 'image' : 'doc'
    pendingAttachments.value.push({ path: pathStr, name, kind })
  }
}

async function onPaste(e: ClipboardEvent) {
  const files = e.clipboardData?.files
  if (!files || files.length === 0) return
  if (pendingAttachments.value.length >= MAX_ATTACH) return
  e.preventDefault()
  for (const file of Array.from(files)) {
    if (pendingAttachments.value.length >= MAX_ATTACH) break
    const ext = extOf(file.name)
    if (ext && !ALL_EXTS.has(ext)) continue
    const buf = await file.arrayBuffer()
    const r = (await invoke('chat_save_temp_image', {
      bytes: new Uint8Array(buf),
      ext: ext || 'png',
      origName: file.name || null
    })) as { success?: boolean; path?: string; name?: string }
    if (r?.success && r.path) {
      const kind: 'image' | 'doc' = IMG_EXTS.has(ext) ? 'image' : 'doc'
      const previewUrl = kind === 'image' ? URL.createObjectURL(file) : undefined
      pendingAttachments.value.push({ path: r.path, name: r.name || file.name, kind, previewUrl })
    }
  }
}

function removeAttachment(index: number) {
  pendingAttachments.value.splice(index, 1)
}

// ── 发送消息 ──
async function sendMessage(text: string) {
  if (!text.trim() || agent.isChatting.value) return
  if (!isTauri()) return

  const atts = [...pendingAttachments.value]
  pendingAttachments.value = []
  input.value = ''

  // push 用户消息
  const userMsgId = crypto.randomUUID()
  store.pushMessage(props.pid, {
    id: userMsgId,
    role: 'user',
    content: text,
    timestamp: Date.now()
  })

  // 创建 AI 消息占位
  const aiMsgId = crypto.randomUUID()
  store.pushMessage(props.pid, {
    id: aiMsgId,
    role: 'assistant',
    content: '',
    timestamp: Date.now(),
    typing: true
  })
  scrollFn()

  // 构建 deckStatus
  const deckStatus = deckReady.value
    ? `已生成 ${p.value?.slides?.length ?? 0} 页，标题：${p.value?.title}`
    : '尚未生成'

  // 调用 agent
  await agent.startAgentChat(text, {
    getMessage: () => {
      const msg = store.getById(props.pid)?.discussion.find((m) => m.id === aiMsgId)
      return (msg as ChatMessage) ?? null
    },
    scrollFn,
    attachments: atts.map((a) => ({ path: a.path, name: a.name })),
    invokeFn: async () => {
      await invoke('presentation_agent_chat', {
        data: {
          query: text,
          deckId: props.deckId,
          deckStatus,
          attachments: atts.map((a) => ({ path: a.path, name: a.name }))
        }
      })
    },
    onDone: () => {
      progressMsg.value = ''
      const msg = store.getById(props.pid)?.discussion.find((m) => m.id === aiMsgId)
      if (msg) msg.typing = false
    },
    onError: () => {
      progressMsg.value = ''
      const msg = store.getById(props.pid)?.discussion.find((m) => m.id === aiMsgId)
      if (msg) msg.typing = false
    }
  })
}

// ── 历史持久化 ──
async function loadHistory() {
  if (!isTauri()) return
  try {
    const { kbLoadSessionMessages } = await import('@/api/db-client')
    const sessionId = `pres-${props.deckId}`
    const session = (await kbLoadSessionMessages(sessionId)) as { messages?: Array<{ role: string; content: string; created_at?: string }> }
    if (session?.messages?.length && p.value && p.value.discussion.length <= 1) {
      // 只在当前没有对话时恢复历史
      p.value.discussion = session.messages.map((m, i) => ({
        id: `hist-${i}`,
        role: m.role as 'user' | 'assistant',
        content: m.content,
        timestamp: m.created_at ? new Date(m.created_at).getTime() : Date.now()
      }))
      // 渲染 markdown
      for (const m of p.value.discussion) {
        if (m.role === 'assistant' && m.content) {
          agent.rendered.value[m.id] = await renderMd(m.content)
        }
      }
    }
  } catch {
    // 忽略加载失败
  }
}

// ── 监听页面生成/更新事件 ──
const unlisteners: Array<() => void> = []

onMounted(async () => {
  await loadHistory()

  unlisteners.push(
    await listen<{ page: number; html: string }>('presentation:page-generated', (ev) => {
      const { page } = ev.payload
      progressPage.value = page
      progressMsg.value = `正在生成第 ${page} 页…`
      emit('page-generated', ev.payload.page, ev.payload.html)
    })
  )
  unlisteners.push(
    await listen<{ page: number; html: string }>('presentation:page-updated', (ev) => {
      const { page } = ev.payload
      progressPage.value = page
      progressMsg.value = `正在修改第 ${page} 页…`
      emit('page-updated', ev.payload.page, ev.payload.html)
    })
  )
})

onUnmounted(() => {
  for (const u of unlisteners) {
    try { u() } catch { /* noop */ }
  }
})
</script>

<template>
  <div class="chat-panel">
    <!-- 进度通知条 -->
    <Transition name="progress-fade">
      <div v-if="progressMsg" class="progress-bar">
        <div class="progress-spinner"></div>
        <span class="progress-text">{{ progressMsg }}</span>
      </div>
    </Transition>

    <!-- 消息列表 -->
    <div ref="messagesRef" class="messages-scroll">
      <!-- 欢迎提示（首轮） -->
      <div v-if="(p?.discussion?.filter((m) => m.role === 'user').length ?? 0) === 0" class="welcome">
        <div class="welcome-icon">
          <svg class="w-8 h-8 text-brand-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path d="M12 18v-5.25m0 0a6.01 6.01 0 001.5-.189m-1.5.189a6.01 6.01 0 01-1.5-.189m3.75 7.478a12.06 12.06 0 01-4.5 0m3.75 2.383a14.406 14.406 0 01-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 10-7.517 0c.85.493 1.509 1.333 1.509 2.316V18" />
          </svg>
        </div>
        <p class="welcome-title">演示材料 AI 助手</p>
        <p class="welcome-desc">告诉我你的需求，我会直接为你制作演示材料。</p>
        <div class="welcome-examples">
          <button class="example-btn" @click="sendMessage('帮我生成一份项目汇报，10页')">帮我生成一份项目汇报</button>
          <button class="example-btn" @click="sendMessage('用知识库里的资料制作产品介绍')">用知识库资料制作产品介绍</button>
        </div>
      </div>

      <!-- 消息列表 -->
      <div
        v-for="m in p?.discussion"
        :key="m.id"
        class="msg-row"
        :class="m.role"
      >
        <div class="avatar" :class="m.role">
          {{ m.role === 'user' ? '我' : 'AI' }}
        </div>
        <div class="msg-body">
          <!-- 用户消息 -->
          <div v-if="m.role === 'user'" class="bubble user-bubble">
            <pre class="user-text">{{ m.content }}</pre>
          </div>
          <!-- AI 消息 -->
          <div v-else class="bubble ai-bubble">
            <div
              class="ai-text markdown-body"
              v-html="agent.rendered.value[m.id] || agent.renderedProgress.value || m.content || '思考中'"
            ></div>
          </div>
        </div>
      </div>

      <!-- 加载指示 -->
      <div v-if="agent.isChatting.value && !progressMsg" class="loading-dots">
        <span class="dot"></span><span class="dot" style="animation-delay:0.15s"></span><span class="dot" style="animation-delay:0.3s"></span>
      </div>
    </div>

    <!-- 输入区 -->
    <div class="input-area">
      <!-- 附件预览 -->
      <AttachmentChips
        v-if="pendingAttachments.length"
        :files="pendingAttachments"
        removable
        @remove="removeAttachment"
      />
      <!-- 输入行 -->
      <div class="input-row">
        <button
          class="attach-btn"
          title="添加附件"
          @click="openAttachmentDialog"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path d="M15.172 7l-6.586 6.586a2 2 0 102.828 2.828l6.414-6.586a4 4 0 00-5.656-5.656l-6.415 6.585a6 6 0 108.486 8.486L20.5 13" />
          </svg>
        </button>
        <textarea
          v-model="input"
          rows="1"
          class="chat-input"
          placeholder="描述你的演示需求…"
          @keydown.enter.exact.prevent="sendMessage(input)"
          @paste="onPaste"
        ></textarea>
        <button
          class="send-btn"
          :class="input.trim() && !agent.isChatting.value ? 'active' : ''"
          :disabled="!input.trim() || agent.isChatting.value"
          @click="sendMessage(input)"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path d="M6 12L3.269 3.126A59.768 59.768 0 0121.485 12 59.77 59.77 0 013.27 20.876L5.999 12zm0 0h7.5" />
          </svg>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #fff;
}

/* ── 进度条 ── */
.progress-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  background: linear-gradient(90deg, #eff6ff, #f0fdf4);
  border-bottom: 1px solid #e5e7eb;
  flex-shrink: 0;
}
.progress-spinner {
  width: 14px; height: 14px;
  border: 2px solid #bfdbfe;
  border-top-color: #2563eb;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.progress-text {
  font-size: 12px;
  color: #2563eb;
  font-weight: 500;
}
.progress-fade-enter-active, .progress-fade-leave-active {
  transition: opacity 0.2s ease;
}
.progress-fade-enter-from, .progress-fade-leave-to {
  opacity: 0;
}

/* ── 消息列表 ── */
.messages-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.messages-scroll::-webkit-scrollbar { width: 4px; }
.messages-scroll::-webkit-scrollbar-thumb { background: #e5e7eb; border-radius: 4px; }

/* ── 欢迎 ── */
.welcome {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 40px 20px;
  text-align: center;
}
.welcome-icon {
  width: 48px; height: 48px;
  display: flex; align-items: center; justify-content: center;
  border-radius: 16px;
  background: linear-gradient(135deg, #eff6ff, #f0fdf4);
  margin-bottom: 12px;
}
.welcome-title {
  font-size: 14px; font-weight: 600; color: #111827; margin-bottom: 4px;
}
.welcome-desc {
  font-size: 12px; color: #6b7280; margin-bottom: 16px;
}
.welcome-examples {
  display: flex; flex-direction: column; gap: 6px; width: 100%; max-width: 240px;
}
.example-btn {
  padding: 8px 12px;
  border: 1px solid #e5e7eb;
  border-radius: 10px;
  font-size: 12px;
  color: #374151;
  background: #fff;
  cursor: pointer;
  transition: all 0.15s;
  text-align: left;
}
.example-btn:hover {
  border-color: #2563eb;
  color: #2563eb;
  background: #eff6ff;
}

/* ── 消息行 ── */
.msg-row {
  display: flex;
  gap: 8px;
  align-items: flex-start;
}
.msg-row.user {
  flex-direction: row-reverse;
}
.avatar {
  width: 28px; height: 28px;
  border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  font-size: 10px; font-weight: 600;
  flex-shrink: 0;
}
.avatar.user {
  background: #2563eb; color: #fff;
}
.avatar.assistant {
  background: linear-gradient(135deg, #2563eb, #93c5fd); color: #fff;
}
.msg-body {
  min-width: 0; max-width: 85%;
}
.bubble {
  padding: 8px 12px;
  border-radius: 16px;
  font-size: 12px;
  line-height: 1.6;
}
.user-bubble {
  background: #2563eb;
  color: #fff;
  border-top-right-radius: 4px;
}
.user-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
  font-size: inherit;
}
.ai-bubble {
  background: #fff;
  border: 1px solid #e5e7eb;
  border-top-left-radius: 4px;
  box-shadow: 0 1px 3px rgba(0,0,0,0.04);
}
.ai-text {
  color: #1f2937;
}
.ai-text :deep(p) { margin: 0 0 6px; }
.ai-text :deep(p:last-child) { margin-bottom: 0; }
.ai-text :deep(ul), .ai-text :deep(ol) { padding-left: 20px; margin: 4px 0; }
.ai-text :deep(li) { margin: 2px 0; }
.ai-text :deep(code) { background: #f3f4f6; padding: 1px 4px; border-radius: 4px; font-size: 11px; }
.ai-text :deep(pre) { background: #f3f4f6; padding: 8px; border-radius: 8px; overflow-x: auto; margin: 6px 0; }
.ai-text :deep(pre code) { background: none; padding: 0; }
.ai-text :deep(h1), .ai-text :deep(h2), .ai-text :deep(h3) { margin: 8px 0 4px; font-weight: 600; }
.ai-text :deep(table) { border-collapse: collapse; margin: 6px 0; width: 100%; }
.ai-text :deep(th), .ai-text :deep(td) { border: 1px solid #e5e7eb; padding: 4px 8px; font-size: 11px; }
.ai-text :deep(th) { background: #f9fafb; font-weight: 600; }

/* ── 加载指示 ── */
.loading-dots {
  display: flex; gap: 4px; padding: 4px 0;
}
.dot {
  width: 5px; height: 5px; border-radius: 50%;
  background: #94a3b8;
  animation: bounce 1s infinite ease-in-out;
}
@keyframes bounce {
  0%, 80%, 100% { transform: scale(0.7); opacity: 0.4; }
  40% { transform: scale(1); opacity: 1; }
}

/* ── 输入区 ── */
.input-area {
  flex-shrink: 0;
  padding: 8px 12px;
  border-top: 1px solid #e5e7eb;
  background: #fff;
}
.input-row {
  display: flex;
  align-items: flex-end;
  gap: 6px;
  background: #fff;
  border: 1px solid #e5e7eb;
  border-radius: 14px;
  padding: 6px 8px;
  box-shadow: 0 1px 4px rgba(0,0,0,0.04);
  transition: border-color 0.15s;
}
.input-row:focus-within {
  border-color: #2563eb;
}
.attach-btn {
  width: 28px; height: 28px;
  display: flex; align-items: center; justify-content: center;
  border: none; background: none;
  color: #9ca3af; cursor: pointer;
  border-radius: 8px;
  transition: all 0.15s;
  flex-shrink: 0;
}
.attach-btn:hover { background: #f3f4f6; color: #374151; }
.chat-input {
  flex: 1;
  border: none; outline: none;
  resize: none;
  font-size: 12px;
  line-height: 1.5;
  min-height: 20px;
  max-height: 80px;
  font-family: inherit;
  color: #111827;
}
.chat-input::placeholder { color: #9ca3af; }
.send-btn {
  width: 28px; height: 28px;
  display: flex; align-items: center; justify-content: center;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.15s;
  flex-shrink: 0;
  background: #f3f4f6;
  color: #9ca3af;
}
.send-btn.active {
  background: #2563eb;
  color: #fff;
}
.send-btn.active:hover {
  background: #1d4ed8;
}

/* ── thinking spinner ── */
:global(.thinking-spinner) {
  display: inline-block;
  width: 14px; height: 14px;
  border: 2px solid #bfdbfe;
  border-top-color: #2563eb;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  vertical-align: middle;
  margin-right: 6px;
}
</style>
