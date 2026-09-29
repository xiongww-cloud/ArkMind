import { createApp } from 'vue'
import { createPinia } from 'pinia'
import ChatWindow from './components/chat/ChatWindow.vue'
import { vTooltip } from './directives/tooltip'
import { initAgentRunListeners } from './stores/agentRun'
import './assets/main.css'
import './assets/markdown.css'

const app = createApp(ChatWindow)
// 浮窗聊天是独立 Vue app（独立 webview 入口，各自一份 JS 上下文）：
// 指令与 pinia 需单独注册——否则 useStore 抛 no active Pinia、v-tooltip 静默失效
const pinia = createPinia()
app.use(pinia)
app.directive('tooltip', vTooltip)
app.mount('#chat-root')

// 独立 webview 收不到主窗 main.ts 的注册，委托轨迹监听须在本入口再注册一次
initAgentRunListeners()
