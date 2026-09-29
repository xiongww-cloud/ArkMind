import { defineStore } from 'pinia'
import { listen } from '@tauri-apps/api/event'

/** 委托可观测性：跨 ChatWindow/AgentChat 共享的"当前干活的子 Agent"状态 */
export interface AgentRunState {
  /** 当前正在执行的子 Agent（null = 主 Agent 或空闲） */
  currentAgent: { id: string; name: string } | null
}

let listenersBound = false

export const useAgentRunStore = defineStore('agentRun', {
  state: (): AgentRunState => ({
    currentAgent: null,
  }),
  actions: {
    /** agent:status 携带 agent_name 时调用（delegating/thinking） */
    setAgent(id: string, name: string) {
      this.currentAgent = { id, name }
    },
    /** delegate-done / agent:done / 无 agent_name 的 status 时调用 */
    clearAgent() {
      this.currentAgent = null
    },
    /** 统一消费 agent:status 事件 payload */
    applyStatus(payload: { agent_id?: string; agent_name?: string } | undefined | null) {
      if (payload?.agent_name) {
        this.setAgent(payload.agent_id || '', payload.agent_name)
      } else {
        this.clearAgent()
      }
    },
  },
})

/**
 * 全局注册 agent 事件监听（main.ts 调用一次）：
 * - agent:status：有 agent_name → 记录当前子 Agent；无身份（主 Agent thinking/answer）或 delegate-done → 清除
 * - agent:done / agent:error：整轮结束 → 清除
 * 单次注册避免每个发送生命周期重复绑定（两个聊天视图共享同一 pinia 实例）
 */
export function initAgentRunListeners() {
  if (listenersBound) return
  listenersBound = true
  const store = useAgentRunStore()
  listen<{ stage?: string; agent_id?: string; agent_name?: string }>('agent:status', (ev) => {
    const p = ev.payload
    if (p?.stage === 'delegate-done') {
      store.clearAgent()
    } else {
      store.applyStatus(p)
    }
  }).catch(() => { listenersBound = false })
  listen('agent:done', () => store.clearAgent()).catch(() => { listenersBound = false })
  listen('agent:error', () => store.clearAgent()).catch(() => { listenersBound = false })
}
