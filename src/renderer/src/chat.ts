import { createApp } from 'vue'
import ChatWindow from './components/chat/ChatWindow.vue'
import { vTooltip } from './directives/tooltip'
import './assets/main.css'
import './assets/markdown.css'

const app = createApp(ChatWindow)
// 浮窗聊天是独立 Vue app（独立 webview 入口），指令需单独注册——否则 v-tooltip 全部静默失效
app.directive('tooltip', vTooltip)
app.mount('#chat-root')
