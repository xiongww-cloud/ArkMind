import { createApp } from 'vue'
import VueApexCharts from 'vue3-apexcharts'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import { vTooltip } from './directives/tooltip'
import { initAgentRunListeners } from './stores/agentRun'
import { setupLinkInterceptor } from './utils/linkInterceptor'
import './assets/main.css'
import './assets/markdown.css'

// 全局处理链接点击：外部链接用默认浏览器打开（浮窗入口 chat.ts 同注册）
setupLinkInterceptor()

const app = createApp(App)
const pinia = createPinia()

app.use(pinia)
app.use(router)
app.use(VueApexCharts)
app.directive('tooltip', vTooltip)
app.mount('#app')

// 委托可观测性：全局单次注册 agent:status/done/error 监听（共享当前子 Agent 状态）
initAgentRunListeners()
