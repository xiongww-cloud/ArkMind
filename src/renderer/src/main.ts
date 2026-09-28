import { createApp } from 'vue'
import VueApexCharts from 'vue3-apexcharts'
import { createPinia } from 'pinia'
import { openUrl } from '@tauri-apps/plugin-opener'
import App from './App.vue'
import router from './router'
import { vTooltip } from './directives/tooltip'
import './assets/main.css'
import './assets/markdown.css'

// 全局处理链接点击：外部链接用默认浏览器打开
document.addEventListener('click', (e) => {
  const target = e.target as HTMLElement
  const anchor = target.closest('a[href]')
  if (!anchor) return
  
  const href = anchor.getAttribute('href')
  if (!href) return
  
  // 判断是否为外部链接（http/https 开头）
  if (href.startsWith('http://') || href.startsWith('https://')) {
    e.preventDefault()
    e.stopPropagation()
    openUrl(href).catch(() => {
      console.error('打开链接失败:', href)
    })
  }
})

const app = createApp(App)
const pinia = createPinia()

app.use(pinia)
app.use(router)
app.use(VueApexCharts)
app.directive('tooltip', vTooltip)
app.mount('#app')
