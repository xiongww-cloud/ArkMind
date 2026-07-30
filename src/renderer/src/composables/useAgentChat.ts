import { ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { sanitizeHtml } from '@/utils/sanitize'
import { pickRetrievalPhrase } from '@/utils/chat'
import { renderObjectReference } from '@/components/renderer'

// ─── Markdown Renderer ──────────────────────────────────────────
export async function renderMd(text: string): Promise<string> {
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

// ─── Tool Status Texts ──────────────────────────────────────────
const toolStatusTexts: Record<string, string[]> = {
  'ext_tool_search': ['让我查查资料...', '我看看知识库...', '正在检索相关文档...', '我去翻翻资料...', '让我找找相关的文档...', '检索一下知识库...', '看看知识库里有没有...', '我来搜搜看...'],
  'ext_tool_search_knowledge_base': ['让我查查资料...', '我看看知识库...', '正在检索相关文档...', '我去翻翻资料...', '让我找找相关的文档...', '检索一下知识库...'],
  'ext_tool_object': ['让我找找有没有相关的文件...', '我看看有哪些相关的资料...', '查一下相关的对象...', '看看有没有相关的记录...', '我来找找相关的文件...', '翻一下相关的资料...'],
  'ext_tool_memory': ['让我回忆一下...', '我查查之前的记忆...', '想想之前有没有聊过这个...', '翻翻记忆...', '看看之前的记录...', '回忆一下之前的对话...', '我把这个记下来...', '记一下这个...', '保存一下这个信息...'],
  'ext_tool_chunk_navigate': ['让我翻页看看...', '继续查看下一页...', '往下看看...', '翻到下一页...', '继续翻翻...', '看看后面的内容...'],
  'ext_tool_relations': ['看看关联了哪些对象...', '查查关联关系...', '看看有哪些关联...'],
  'ext_tool_manage_todo': ['看看待办事项...', '查一下待办...', '我看看待办列表...'],
  'ext_tool_manage_project': ['查查项目情况...', '看看项目进展...', '我来查查项目...'],
  'ext_tool_manage_partner': ['看看合作伙伴...', '查查合作方信息...'],
  'ext_tool_manage_contact': ['查查联系人...', '看看联系人信息...'],
  'ext_tool_manage_policy': ['看看有什么规则...', '查查策略...'],
}
const defaultStatusTexts = ['处理中...', '请稍等...', '正在处理...']
export function pickToolStatus(toolName: string): string {
  const arr = toolStatusTexts[toolName] || defaultStatusTexts
  return arr[Math.floor(Math.random() * arr.length)]
}

// ─── Types ──────────────────────────────────────────────────────
export interface ChatMessage {
  id?: string
  role: 'user' | 'assistant'
  content: string
  error?: string
  thinking?: boolean
  isThinking?: boolean
  toolCalls?: any[]
  rawToolCalls?: any[]
  toolResults?: any[]
  contextDocs?: any[]
  workspace?: string
  typewriterText?: string
  typewriterDone?: boolean
  timestamp?: number
}

// ─── Agent:done result type ─────────────────────────────────────
export interface AgentDoneResult {
  success: boolean
  answer?: string
  tool_log?: any[]
  context_docs?: any[]
  sources?: any[]
  workspace?: string
  error?: string
}

// ─── Agent:done callback context ────────────────────────────────
export interface AgentDoneContext {
  finalAnswer: string
  isRealStreaming: boolean
  streamedContent: string
}

// ─── Options ────────────────────────────────────────────────────
export interface AgentChatOptions {
  /** 获取当前 AI 消息引用 */
  getMessage: () => ChatMessage | null
  /** 滚动函数 */
  scrollFn: () => void
  /** agent:done 回调（处理组件特有逻辑） */
  onDone?: (result: AgentDoneResult, ctx: AgentDoneContext) => void
  /** agent:error 回调 */
  onError?: (error: string) => void
  /** agent:start-answer 回调 */
  onStartAnswer?: () => void
  /** 完成回调（done/error 共用，用于 mascot idle 等） */
  onFinish?: () => void
  /** 会话 ID（用于持久化会话状态） */
  sessionId?: string
}

// ─── Composable ─────────────────────────────────────────────────
export function useAgentChat() {
  // 共享状态
  const isChatting = ref(false)
  const thinkingProgress = ref('')
  const renderedProgress = ref('')
  const rendered = ref<Record<string, string>>({})

  // ── setThinkingProgress ──
  function setThinkingProgress(text: string) {
    thinkingProgress.value = text
    renderedProgress.value = ''
    if (text) {
      renderMd(text.replace(/~/g, '～')).then(h => { renderedProgress.value = h })
    }
  }

  // ── Typewriter（统一用 msgId string） ──
  const activeTypewriters = new Set<string>()
  const typewriterTimers = new Map<string, ReturnType<typeof setInterval>>()

  function startTypewriter(msgId: string, text: string, msg: ChatMessage) {
    stopTypewriter(msgId)
    if (!text) { msg.content = ''; msg.typewriterText = ''; msg.typewriterDone = true; return }
    const full = text.replace(/~/g, '～')
    rendered.value[msgId] = ''  // 清空，等待逐字渲染
    let pos = 0
    msg.content = ''
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
        renderMd(full).then(h => {
          rendered.value[msgId] = h
          msg.typewriterDone = true
        }).catch(() => { msg.typewriterDone = true })
        return
      }
      msg.typewriterText = full.slice(0, pos)
      renderMd(msg.typewriterText).then(h => { rendered.value[msgId] = h })
    }, 8)
    typewriterTimers.set(msgId, timer)
  }

  function stopTypewriter(msgId: string) {
    const timer = typewriterTimers.get(msgId)
    if (timer) { clearInterval(timer); typewriterTimers.delete(msgId) }
    activeTypewriters.delete(msgId)
  }

  function stopAllTypewriters(msgs?: ChatMessage[]) {
    if (msgs) {
      for (const msgId of activeTypewriters) {
        const msg = msgs.find(m => m.id === msgId)
        if (msg) {
          const full = msg.typewriterText || msg.content || ''
          renderMd(full.replace(/~/g, '～')).then(h => {
            rendered.value[msgId] = h
            msg.typewriterDone = true
          }).catch(() => { msg.typewriterDone = true })
        }
      }
    }
    for (const msgId of activeTypewriters) {
      const timer = typewriterTimers.get(msgId)
      if (timer) clearInterval(timer)
    }
    activeTypewriters.clear()
    typewriterTimers.clear()
  }

  function isTypewriting(msg: ChatMessage): boolean {
    return msg.typewriterText !== undefined && !msg.typewriterDone
  }

  // ── stopAgent ──
  async function stopAgent(getMessages: () => ChatMessage[]) {
    try {
      const { agentCancel } = await import('@/api/db-client')
      await agentCancel()
      isChatting.value = false
      const msgs = getMessages()
      const lastMsg = msgs[msgs.length - 1]
      if (lastMsg && lastMsg.role === 'assistant' && (lastMsg.thinking || lastMsg.isThinking)) {
        lastMsg.thinking = false
        lastMsg.isThinking = false
        if (!lastMsg.content) lastMsg.content = '已停止执行。'
      }
    } catch (e) {
      console.error('[AgentChat] 停止失败:', e)
    }
  }

  // ── startAgentChat ──
  async function startAgentChat(query: string, options: AgentChatOptions) {
    const { getMessage, scrollFn, onDone, onError, onStartAnswer, onFinish, sessionId } = options

    isChatting.value = true
    setThinkingProgress('')

    const presetText = pickRetrievalPhrase()
    setThinkingProgress(presetText)
    scrollFn()

    try {
      let hasRoundThought = false

      const unlistenProgress = await listen<{ msg: string }>('agent:progress', (ev) => {
        if (!hasRoundThought) { setThinkingProgress(ev.payload.msg) }
        scrollFn()
      })
      const unlistenRoundThought = await listen<{text?: string}>('agent:round-thought', (ev) => {
        hasRoundThought = true
        const msg = getMessage()
        if (msg) {
          msg.thinking = true
          msg.isThinking = true
        }
        if (ev.payload.text && ev.payload.text.trim()) {
          setThinkingProgress(ev.payload.text.trim())
        } else {
          setThinkingProgress(pickRetrievalPhrase())
        }
        scrollFn()
      })

      invoke('kb_agent_chat', { data: { query, sessionId: sessionId || undefined } }).catch((err) => {
        console.error('[AgentChat] invoke failed:', err)
        const msg = getMessage()
        if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = String(err) }
        isChatting.value = false
        scrollFn()
      })

      const unlistenDone = await listen<any>('agent:done', async (ev) => {
        const result = ev.payload
        const msg = getMessage()
        if (msg) { msg.thinking = false; msg.isThinking = false }
        setThinkingProgress('')

        if (result && result.success) {
          const finalAnswer = (result.answer || '').replace(/<refs>[\s\S]*<\/refs>/g, '').trim()
          
          // 解析 <refs> 标签
          const refsMatch = (result.answer || '').match(/<refs>([\s\S]*?)<\/refs>/)
          let objectRefs = []
          
          if (refsMatch) {
            try {
              objectRefs = JSON.parse(refsMatch[1])
            } catch {}
          }
          
          const responseType = result.result_type || (objectRefs.length > 0 ? 'object_reference' : 'answer')
          
          if (msg) {
            // 如果有对象引用，渲染对象卡片
            if (objectRefs.length > 0) {
              const refsHtml = renderObjectReference({ objects: objectRefs })
              renderMd(finalAnswer.replace(/~/g, '～')).then(mdHtml => {
                msg.content = finalAnswer
                if (msg.id) rendered.value[msg.id!] = `<div class="object-references">${refsHtml}</div><div class="answer-content">${mdHtml}</div>`
              })
            } else {
              // 普通 Markdown 渲染 + 打字机效果
              startTypewriter(msg.id || '', finalAnswer, msg)
            }
          }
          onDone?.(result, { finalAnswer, isRealStreaming: false, streamedContent: '', responseType })
        } else {
          if (msg) { msg.content = result?.answer || result?.error || '请求失败' }
          onError?.(result?.error || result?.answer || '请求失败')
        }

        isChatting.value = false
        onFinish?.()
        scrollFn()
        cleanup()
      })

      const unlistenError = await listen<any>('agent:error', (ev) => {
        const msg = getMessage()
        if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = ev.payload.error || 'AI 服务异常' }
        isChatting.value = false
        onError?.(ev.payload.error || 'AI 服务异常')
        onFinish?.()
        scrollFn()
        cleanup()
      })

      function cleanup() {
        try {
          unlistenProgress()
          unlistenRoundThought()
          unlistenDone()
          unlistenError()
        } catch {}
      }
    } catch (e: any) {
      const msg = getMessage()
      if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = typeof e === 'string' ? e : (e.message || 'AI 服务异常') }
      isChatting.value = false
      onFinish?.()
      scrollFn()
    }
  }

  return {
    isChatting,
    thinkingProgress,
    renderedProgress,
    rendered,
    setThinkingProgress,
    pickToolStatus,
    renderMd,
    startTypewriter,
    stopTypewriter,
    stopAllTypewriters,
    isTypewriting,
    startAgentChat,
    stopAgent,
  }
}
