/**
 * useGlobalPaste — 全局粘贴快捷键（Ctrl+V / ⌘V）监听
 *
 * macOS WebView 中 paste 事件依赖焦点，弹窗/非输入框区域按 ⌘V 不触发 @paste，
 * 需在 document 上监听 keydown 兜底。若焦点在可编辑元素内则让系统事件处理。
 *
 * @param active  是否启用监听（弹窗打开时传 true）
 * @param onPaste 触发回调
 */
import { onMounted, onUnmounted, watch, type Ref } from 'vue'
import { isMac } from '@/utils/platform'

export function useGlobalPaste(active: Ref<boolean>, onPaste: () => void) {
  function handler(e: KeyboardEvent) {
    if (!active.value) return
    const meta = isMac ? e.metaKey : e.ctrlKey
    if (!meta || (e.key !== 'v' && e.key !== 'V')) return
    // 焦点在输入框/文本域/可编辑元素内 → 交系统粘贴事件（@paste）处理
    const el = document.activeElement
    if (
      el &&
      (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || (el as HTMLElement).isContentEditable)
    ) {
      return
    }
    e.preventDefault()
    onPaste()
  }

  onMounted(() => document.addEventListener('keydown', handler))
  onUnmounted(() => document.removeEventListener('keydown', handler))
}
