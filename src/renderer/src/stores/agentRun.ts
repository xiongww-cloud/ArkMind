import { defineStore } from 'pinia'
import { listen } from '@tauri-apps/api/event'

/** 委托可观测性：子 Agent 执行轨迹状态（每个 webview 窗口一份 pinia 实例，
 *  主窗 AgentChat 与浮窗 ChatWindow 在各自入口分别注册监听） */
export interface AgentRunState {
  /** 当前正在执行的子 Agent（null = 主 Agent 或空闲）；只驱动气泡内轨迹行 */
  currentAgent: { id: string; name: string } | null
  /** 本轮参与过的子 Agent 名称（去重保序），agent:done 时由前端写入消息的 assistants 署名 */
  participants: string[]
  /** 生成心跳：按子 Agent id 分桶的产出进度。展示口径：写参数期显示"生成内容 · N 字"、
   *  工具执行期显示"正在：动作"、交付型正文期显示"生成 · N 字"（不再有虚指的"步数"） */
  writingByAgent: Record<string, { chars: number; tools: number; deliverable: boolean; stool?: string; cur?: string; pending?: number }>
}

let listenersBound = false

export const useAgentRunStore = defineStore('agentRun', {
  state: (): AgentRunState => ({
    currentAgent: null,
    participants: [],
    writingByAgent: {},
  }),
  actions: {
    setAgent(id: string, name: string) {
      this.currentAgent = { id, name }
      this.writingByAgent = {}
      if (name && !this.participants.includes(name)) this.participants.push(name)
    },
    clearAgent() {
      this.currentAgent = null
      this.writingByAgent = {}
    },
    /** 每轮发送开始时调用：清空上轮署名（done 处理器读取 participants 之后才会再次重置） */
    resetParticipants() {
      this.participants = []
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
 * 全局注册 agent 事件监听（每个 webview 入口各调一次：主窗 main.ts、浮窗 chat.ts——
 * tauri emit 广播到所有窗口，但各窗口 JS 上下文独立，pinia store 也各自一份）：
 * - agent:status：有 agent_name → 记录当前子 Agent 并累积 participants；
 *   无身份（主 Agent thinking/answer）或 delegate-done → 仅清除 currentAgent
 * - agent:done / agent:error：仅清除 currentAgent——participants 必须活过 done 事件
 *   （全局监听注册更早会先触发，组件 done 处理器读署名在前，清空只会在下一轮发送起点）
 */
export function initAgentRunListeners() {
  if (listenersBound) return
  listenersBound = true
  const store = useAgentRunStore()
  listen<{ stage?: string; agent_id?: string; agent_name?: string; chars?: number; tools?: number; class?: string; stool?: string; label?: string; tool?: string; pending?: number }>('agent:status', (ev) => {
    const p = ev.payload
    if (p?.stage === 'writing') {
      // 生成心跳：按 agent_id 落桶，绝不触碰 currentAgent（无 agent_name 的 payload 会被 applyStatus 误清）
      // 心跳到达即意味着工具执行已结束，cur（"正在执行的动作"）随之失效
      store.writingByAgent[p.agent_id || ''] = {
        chars: p.chars ?? 0,
        tools: p.tools ?? 0,
        deliverable: p.class === 'deliverable',
        stool: p.stool || '',
        pending: p.pending ?? 0,
      }
    } else if (p?.stage === 'tool_start') {
      store.applyStatus(p)
      if (p.agent_id) {
        const prev = store.writingByAgent[p.agent_id]
        store.writingByAgent[p.agent_id] = { chars: prev?.chars ?? 0, tools: prev?.tools ?? 0, deliverable: prev?.deliverable ?? false, stool: '', cur: p.label || p.tool || '' }
      }
    } else if (p?.stage === 'delegate-done') {
      store.clearAgent()
    } else {
      store.applyStatus(p)
    }
  }).catch(() => { listenersBound = false })
  listen('agent:done', () => store.clearAgent()).catch(() => { listenersBound = false })
  listen('agent:error', () => store.clearAgent()).catch(() => { listenersBound = false })
}
