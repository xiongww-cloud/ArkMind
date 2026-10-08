import { openUrl } from '@tauri-apps/plugin-opener'

/**
 * 全局链接拦截：http(s)/mailto 一律交系统默认浏览器/邮件客户端打开。
 * 每个 webview 入口（主窗 main.ts、浮窗 chat.ts）都要注册一次——
 * 独立窗口的 document 收不到主窗注册，漏注册时点击会走 WebView 内导航。
 */
export function setupLinkInterceptor() {
  document.addEventListener('click', (e) => {
    const target = e.target as HTMLElement
    const anchor = target.closest('a[href]')
    if (!anchor) return

    const href = anchor.getAttribute('href')
    if (!href) return

    if (href.startsWith('http://') || href.startsWith('https://') || href.startsWith('mailto:')) {
      e.preventDefault()
      e.stopPropagation()
      openUrl(href).catch(() => {
        console.error('打开链接失败:', href)
      })
    }
  })
}
