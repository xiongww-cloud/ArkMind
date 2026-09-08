import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

// ─── Types ──────────────────────────────────────────────────────
export interface ChatMessage {
  id?: string
  dbId?: string
  role: 'user' | 'assistant'
  content: string
  timestamp?: number
  isThinking?: boolean
  typewriterText?: string
  typewriterDone?: boolean
  toolCalls?: any[]
  rawToolCalls?: any[]
  toolResults?: any[]
  contextDocs?: any[]
  workspace?: string
  attachments?: Array<{ path: string; name: string; kind: 'image' | 'doc' }>
}

export interface ChatSession {
  id: string
  name: string
  messages: ChatMessage[]
  createdAt: string
  updatedAt?: string
}

// ─── Store ──────────────────────────────────────────────────────
export const useChatStore = defineStore('chat', () => {
  // ─── State ────────────────────────────────────────────────────
  const sessions = ref<ChatSession[]>([])
  const currentSessionId = ref<string | null>(null)
  const inputMessage = ref('')
  const pendingAttachments = ref<Array<{ path: string; name: string; kind: 'image' | 'doc'; previewUrl?: string }>>([])
  const sessionDirty = ref(false)
  const isChatting = ref(false)
  const thinkingProgress = ref('')
  const rendered = ref<Record<string, string>>({})
  const sessionHasMore = ref<Record<string, boolean>>({})
  const sidebarOpen = ref(true)

  // ─── Computed ─────────────────────────────────────────────────
  const currentSession = computed(() => 
    sessions.value.find(s => s.id === currentSessionId.value) || null
  )

  const currentMessages = computed(() => 
    currentSession.value?.messages || []
  )

  // ─── Actions ──────────────────────────────────────────────────
  function setCurrentSession(id: string) {
    currentSessionId.value = id
  }

  function updateSessions(newSessions: ChatSession[]) {
    sessions.value = newSessions
  }

  function addSession(session: ChatSession) {
    sessions.value.unshift(session)
  }

  function removeSession(id: string) {
    sessions.value = sessions.value.filter(s => s.id !== id)
    if (currentSessionId.value === id) {
      currentSessionId.value = sessions.value[0]?.id || null
    }
  }

  function updateSessionMessages(sessionId: string, messages: ChatMessage[]) {
    const session = sessions.value.find(s => s.id === sessionId)
    if (session) {
      session.messages = messages
    }
  }

  function addMessage(sessionId: string, message: ChatMessage) {
    const session = sessions.value.find(s => s.id === sessionId)
    if (session) {
      session.messages.push(message)
    }
  }

  function updateMessage(sessionId: string, messageId: string, updates: Partial<ChatMessage>) {
    const session = sessions.value.find(s => s.id === sessionId)
    if (session) {
      const msg = session.messages.find(m => m.id === messageId)
      if (msg) {
        Object.assign(msg, updates)
      }
    }
  }

  function setInputMessage(text: string) {
    inputMessage.value = text
  }

  function clearInputMessage() {
    inputMessage.value = ''
  }

  function setPendingAttachments(attachments: typeof pendingAttachments.value) {
    pendingAttachments.value = attachments
  }

  function addPendingAttachment(attachment: typeof pendingAttachments.value[0]) {
    pendingAttachments.value.push(attachment)
  }

  function removePendingAttachment(idx: number) {
    pendingAttachments.value.splice(idx, 1)
  }

  function clearPendingAttachments() {
    pendingAttachments.value = []
  }

  function setIsChatting(value: boolean) {
    isChatting.value = value
  }

  function setThinkingProgress(text: string) {
    thinkingProgress.value = text
  }

  function setRendered(msgId: string, html: string) {
    rendered.value[msgId] = html
  }

  function clearRendered() {
    rendered.value = {}
  }

  function setSessionHasMore(sessionId: string, hasMore: boolean) {
    sessionHasMore.value[sessionId] = hasMore
  }

  function setSidebarOpen(value: boolean) {
    sidebarOpen.value = value
  }

  function markDirty() {
    sessionDirty.value = true
  }

  function markClean() {
    sessionDirty.value = false
  }

  // ─── Serialization ────────────────────────────────────────────
  function toJSON() {
    return {
      sessions: sessions.value,
      currentSessionId: currentSessionId.value,
      inputMessage: inputMessage.value,
      sessionHasMore: sessionHasMore.value,
      sidebarOpen: sidebarOpen.value,
    }
  }

  function fromJSON(data: any) {
    if (data.sessions) sessions.value = data.sessions
    if (data.currentSessionId) currentSessionId.value = data.currentSessionId
    if (data.inputMessage) inputMessage.value = data.inputMessage
    if (data.sessionHasMore) sessionHasMore.value = data.sessionHasMore
    if (data.sidebarOpen !== undefined) sidebarOpen.value = data.sidebarOpen
  }

  return {
    // State
    sessions,
    currentSessionId,
    inputMessage,
    pendingAttachments,
    sessionDirty,
    isChatting,
    thinkingProgress,
    rendered,
    sessionHasMore,
    sidebarOpen,
    // Computed
    currentSession,
    currentMessages,
    // Actions
    setCurrentSession,
    updateSessions,
    addSession,
    removeSession,
    updateSessionMessages,
    addMessage,
    updateMessage,
    setInputMessage,
    clearInputMessage,
    setPendingAttachments,
    addPendingAttachment,
    removePendingAttachment,
    clearPendingAttachments,
    setIsChatting,
    setThinkingProgress,
    setRendered,
    clearRendered,
    setSessionHasMore,
    setSidebarOpen,
    markDirty,
    markClean,
    toJSON,
    fromJSON,
  }
})
