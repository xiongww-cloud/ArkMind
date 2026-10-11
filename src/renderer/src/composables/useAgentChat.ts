import { ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { sanitizeHtml } from '@/utils/sanitize'
import { pickRetrievalPhrase } from '@/utils/chat'
import { renderObjectReference } from '@/components/renderer'

// ─── Markdown Renderer ──────────────────────────────────────────
/** 行级 markdown 渲染（**粗体**、`代码`、链接等）→ 单行 HTML，表格单元格等内联场景专用 */
export async function renderMdInline(text: string): Promise<string> {
  if (!text) return ''
  try {
    const mod = await import('marked')
    const Marked = mod.Marked || (mod as any).default?.Marked || (mod as any).default
    if (Marked && typeof Marked === 'function') {
      const marked = new Marked({ gfm: true, breaks: false })
      return sanitizeHtml(String(marked.parseInline?.(text) ?? ''))
    }
    const { marked } = mod as any
    return sanitizeHtml(String(marked?.parseInline?.(text) ?? ''))
  } catch { return '' }
}

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
  'ext_tool_manage_note': ['记一下笔记...', '整理笔记内容...'],
  'ext_tool_write_document': ['正在生成文档...', '我来整理成文档...', '生成文件中...', '排版中...', '正在写入内容...'],
}
const defaultStatusTexts = ['处理中...', '请稍等...', '正在处理...']
export function pickToolStatus(toolName: string): string {
  const arr = toolStatusTexts[toolName] || defaultStatusTexts
  return arr[Math.floor(Math.random() * arr.length)]
}

// ─── Types ──────────────────────────────────────────────────────
export interface GeneratedDocument {
  file_id: string
  file_name: string
  file_type: string
  file_size: number
}
/** ext_tool_ask_user 追问的单个问题（与后端 normalize_ask_args 输出契约一致） */
export interface ClarificationQuestion {
  question_id: string
  prompt: string
  type: 'single_choice' | 'multiple_choice' | 'text'
  options: string[]
}
/** clarification 数据结构：{reason, questions[], truncated} */
export interface ClarificationData {
  reason: string
  questions: ClarificationQuestion[]
  truncated?: boolean
}
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
  documents?: GeneratedDocument[]
  clarification?: ClarificationData
  /** 本轮参与过的子 Agent 显示名（via 署名，来自 agentRun.participants / metadata.assistants） */
  assistants?: string[]
}

/// 从 tool_log 提取 ext_tool_write_document 成功生成的文档对象列表。
/// 两条路径：1) 直调 write_document；2) 委托 document_writer 子 Agent →
/// delegate envelope 的 data.artifacts 中 type=document_created 条目。
export function extractDocumentsFromToolLog(toolLog: any[] | undefined | null): GeneratedDocument[] {
  const docs: GeneratedDocument[] = []
  for (const round of toolLog || []) {
    for (const t of round?.tools || []) {
      try {
        const env = JSON.parse(t.result)
        // 路径1：直调
        if (
          t.tool === 'ext_tool_write_document' &&
          env?.result_type === 'document_write_result' &&
          env.data?.success &&
          env.data?.file_id
        ) {
          docs.push({
            file_id: env.data.file_id,
            file_name: env.data.file_name || '未命名文档',
            file_type: env.data.file_type || 'docx',
            file_size: env.data.file_size || 0,
          })
          continue
        }
        // 路径2：委托文档助手（主 Agent tool_log 只有 delegate 条目）
        if (t.tool === 'ext_tool_delegate_agent' && env?.result_type === 'delegate_agent_result') {
          for (const a of env.data?.artifacts || []) {
            if (a?.type === 'document_created' && a.file_id) {
              docs.push({
                file_id: a.file_id,
                file_name: a.file_name || '未命名文档',
                file_type: a.file_type || 'docx',
                file_size: a.file_size || 0,
              })
            }
          }
        }
      } catch { /* 非 JSON 结果忽略 */ }
    }
  }
  // 按 file_id 去重（保序）
  const seen = new Set<string>()
  return docs.filter((d) => (seen.has(d.file_id) ? false : (seen.add(d.file_id), true)))
}

/// 从消息 metadata（chat_messages.metadata JSON 字符串）恢复文档对象列表。
/// 重启后历史消息没有 tool_log，文档卡片靠该字段恢复。
export function parseDocumentsMetadata(metadata: unknown): GeneratedDocument[] | undefined {
  if (!metadata || typeof metadata !== 'string') return undefined
  try {
    const m = JSON.parse(metadata)
    const arr = m?.documents
    if (Array.isArray(arr) && arr.length > 0) {
      return arr
        .filter((d: any) => d && typeof d.file_id === 'string')
        .map((d: any) => ({
          file_id: d.file_id,
          file_name: d.file_name || '未命名文档',
          file_type: d.file_type || 'docx',
          file_size: d.file_size || 0,
        }))
    }
  } catch { /* 非 JSON 忽略 */ }
  return undefined
}

/// 从消息 metadata 恢复追问卡片数据（text+card 双轨中的结构化轨）。
export function parseClarificationMetadata(metadata: unknown): ClarificationData | undefined {
  if (!metadata || typeof metadata !== 'string') return undefined
  try {
    const m = JSON.parse(metadata)
    const c = m?.clarification
    if (c && Array.isArray(c.questions) && c.questions.length > 0) {
      return {
        reason: typeof c.reason === 'string' ? c.reason : '',
        questions: c.questions
          .filter((q: any) => q && typeof q.prompt === 'string' && q.prompt)
          .map((q: any) => ({
            question_id: String(q.question_id || ''),
            prompt: q.prompt,
            type: q.type === 'single_choice' || q.type === 'multiple_choice' ? q.type : 'text',
            options: Array.isArray(q.options) ? q.options.filter((o: any) => typeof o === 'string') : [],
          })),
        truncated: !!c.truncated,
      }
    }
  } catch { /* 非 JSON 忽略 */ }
  return undefined
}

/// 选择题自定义答案与点选结果的合并（单选：自定义覆盖点选——动手打字即表明选项都不满意；
/// 多选：自定义作为附加项并入；text 题：自定义即答案。纯函数，可单测）。
export function mergeClarificationAnswer(
  q: ClarificationQuestion,
  selected: string | string[] | undefined,
  custom: string | undefined,
): string | string[] {
  const c = (custom || '').trim()
  if (q.type === 'multiple_choice') {
    const arr = Array.isArray(selected) ? selected.filter(Boolean) : selected ? [selected] : []
    return c ? [...arr, c] : arr
  }
  if (q.type === 'single_choice') {
    return c || (typeof selected === 'string' ? selected : '')
  }
  return c || (typeof selected === 'string' ? selected : '')
}

/// 把追问卡片的选择合成为用户消息文本（走正常发送管线）。
/// answers: question_id → 单选字符串 / 多选数组；未作答的问题跳过；全空时返回默认继续话术。
export function buildClarificationReply(data: ClarificationData, answers: Record<string, string | string[]>): string {
  const parts: string[] = []
  data.questions.forEach((q, i) => {
    const a = answers[q.question_id]
    const text = (Array.isArray(a) ? a.join('、') : (a || '')).trim()
    if (!text) return
    parts.push(`${i + 1}.${q.prompt.replace(/[？?]\s*$/, '')}：${text}`)
  })
  if (!parts.length) return '暂不补充这些信息，请按最合理的默认假设继续，并在结果中括注说明采用的假设。'
  return `已确认补充信息：${parts.join('；')}。请继续执行。`
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
  clarification?: ClarificationData
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
  /** 附件列表（文件路径+名称） */
  attachments?: Array<{ path: string; name: string }>
  /** 自定义 invoke 函数（用于非 kb_agent_chat 的 agent，如演示材料 agent） */
  invokeFn?: () => Promise<void>
}

// ─── Composable ─────────────────────────────────────────────────
export function useAgentChat() {
  // 共享状态
  const isChatting = ref(false)
  const thinkingProgress = ref('')
  const renderedProgress = ref('')
  const rendered = ref<Record<string, string>>({})

  // ── 思考流（agent:thought 瞬态通道：不入消息、不持久化，正文开始即折叠）──
  const thought = ref('')
  const thoughtCollapsed = ref(false)
  const thoughtSecs = ref(0)
  const thoughtMsgId = ref('')

  // ── setThinkingProgress ──
  const THINKING_SPINNER = '<span class="thinking-spinner"></span>'
  function setThinkingProgress(text: string) {
    thinkingProgress.value = text
    if (text) {
      const esc = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      renderedProgress.value = THINKING_SPINNER + esc.replace(/~/g, '～')
    } else {
      renderedProgress.value = ''
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
    const { getMessage, scrollFn, onDone, onError, onStartAnswer, onFinish, sessionId, attachments, invokeFn } = options

    isChatting.value = true
    setThinkingProgress('思考中')
    // 思考流：本轮专属，重置后随 agent:thought 增量累积
    thought.value = ''
    thoughtCollapsed.value = false
    thoughtSecs.value = 0
    thoughtMsgId.value = getMessage()?.id || ''
    const thoughtT0 = Date.now()
    scrollFn()

    try {
      // ★ 实时进度（优先展示后端 msg，如"已路由到「XX」"，否则随机话术）
      const unlistenProgress = await listen<{ msg?: string }>('agent:progress', (ev) => {
        setThinkingProgress(ev.payload?.msg || pickRetrievalPhrase())
        scrollFn()
      })
      // ★ 思考流增量（to_ui 裁决已在后端把关：只有面向窗口输出的 run 会发）
      const unlistenThought = await listen<{ text: string }>('agent:thought', (ev) => {
        if (thoughtCollapsed.value) return
        thought.value += ev.payload?.text || ''
        scrollFn()
      })
      // ★ 实时 token 流（打字机队列）
      const tokenQueue: string[] = []
      let tokenIdx = 0
      let tokenFlushed = false
      let lastRenderTime = 0
      const flushTokenQueue = () => {
        if (tokenFlushed) return; tokenFlushed = true
        clearInterval(tokenTimer)
        const msg = getMessage(); if (!msg) return
        while (tokenIdx < tokenQueue.length) msg.content += tokenQueue[tokenIdx++]
      }
      const collapseThought = () => {
        if (!thoughtCollapsed.value && thought.value) {
          thoughtCollapsed.value = true
          thoughtSecs.value = Math.max(1, Math.round((Date.now() - thoughtT0) / 1000))
        }
      }
      const tokenTimer = setInterval(() => {
        const msg = getMessage(); if (!msg) return
        if (tokenIdx < tokenQueue.length) { if (tokenIdx === 0) { setThinkingProgress(''); collapseThought() } ; msg.content += tokenQueue[tokenIdx++] }
        const now = Date.now()
        if (msg.id && msg.content && now - lastRenderTime > 200) {
          lastRenderTime = now
          renderMd(msg.content.replace(/~/g, '～')).then(h => { rendered.value[msg.id!] = h })
        }
        scrollFn()
      }, 15)
      const unlistenToken = await listen<{ token: string }>('agent:token', (ev) => {
        tokenQueue.push(ev.payload.token)
      })

      // ★ invoke 已移至所有 listener 注册之后（见下方）
      const unlistenDone = await listen<any>('agent:done', async (ev) => {
        flushTokenQueue()
        if (!thoughtCollapsed.value && thought.value) { thoughtCollapsed.value = true; thoughtSecs.value = Math.max(1, Math.round((Date.now() - thoughtT0) / 1000)) }
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
              renderMd(finalAnswer.replace(/~/g, '～')).then(async mdHtml => {
                if (finalAnswer && !result.cancelled) msg.content = finalAnswer
                if (msg.id) rendered.value[msg.id!] = await sanitizeHtml(`<div class="object-references">${refsHtml}</div><div class="answer-content">${mdHtml}</div>`)
              })
            } else {
              // 普通 Markdown 渲染：成功时用权威答案覆盖已流出片段（取消时保留部分内容）
              if (finalAnswer && !result.cancelled) msg.content = finalAnswer
              renderMd(msg.content.replace(/~/g, '～')).then(h => {
                if (msg && msg.id) rendered.value[msg.id!] = h
              })
            }
          }
          onDone?.(result, { finalAnswer, isRealStreaming: true, streamedContent: msg?.content || '', responseType })
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
        flushTokenQueue()
        collapseThought()
        const msg = getMessage()
        if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = ev.payload.error || 'AI 服务异常' }
        isChatting.value = false
        onError?.(ev.payload.error || 'AI 服务异常')
        onFinish?.()
        scrollFn()
        cleanup()
      })

      // ★ 先注册完所有 listener 再 invoke，避免 agent:done 在 listener 注册前发出导致事件丢失
      if (invokeFn) {
        invokeFn().catch((err: any) => {
          console.error('[AgentChat] invokeFn failed:', err)
          flushTokenQueue()
          const msg = getMessage()
          if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = String(err) }
          isChatting.value = false
          scrollFn()
          cleanup()
        })
      } else {
        invoke('kb_agent_chat', { data: { query, sessionId: sessionId || undefined, attachments: attachments || [] } }).catch((err) => {
          console.error('[AgentChat] invoke failed:', err)
          flushTokenQueue()
          const msg = getMessage()
          if (msg) { msg.thinking = false; msg.isThinking = false; msg.error = String(err) }
          isChatting.value = false
          scrollFn()
          cleanup()
        })
      }

      function cleanup() {
        clearInterval(tokenTimer)
        try {
          unlistenProgress()
          unlistenThought()
          unlistenToken()
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
    thought,
    thoughtCollapsed,
    thoughtSecs,
    thoughtMsgId,
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
