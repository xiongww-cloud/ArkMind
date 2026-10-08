import { createApp } from 'vue'
import { createPinia } from 'pinia'
import ChatWindow from './components/chat/ChatWindow.vue'
import { vTooltip } from './directives/tooltip'
import { initAgentRunListeners } from './stores/agentRun'
import { setupLinkInterceptor } from './utils/linkInterceptor'
import './assets/main.css'
import './assets/markdown.css'

// 独立 webview 同样收不到主窗 main.ts 的全局注册——链接拦截必须在本入口再注册一次，
// 否则点击链接会在浮窗内直接导航（WebView2 默认行为）
setupLinkInterceptor()

const app = createApp(ChatWindow)
// 浮窗聊天是独立 Vue app（独立 webview 入口，各自一份 JS 上下文）：
// 指令与 pinia 需单独注册——否则 useStore 抛 no active Pinia、v-tooltip 静默失效
const pinia = createPinia()
app.use(pinia)
app.directive('tooltip', vTooltip)
app.mount('#chat-root')

// 独立 webview 收不到主窗 main.ts 的注册，委托轨迹监听须在本入口再注册一次
initAgentRunListeners()
