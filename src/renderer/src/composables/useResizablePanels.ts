import { ref } from 'vue'

interface UseResizablePanelsOptions {
  sidebarWidth?: number
  listWidth?: number
  minWidth?: number
  maxWidth?: number
}

export function useResizablePanels(options: UseResizablePanelsOptions = {}) {
  const {
    sidebarWidth: initSidebar = 220,
    listWidth: initList = 260,
    minWidth = 140,
    maxWidth = 500
  } = options

  const sidebarWidth = ref(initSidebar)
  const listWidth = ref(initList)
  const isResizing = ref<'sidebar' | 'list' | null>(null)
  const resizeStartX = ref(0)
  const resizeStartW = ref(0)

  function onResizeStart(side: 'sidebar' | 'list', e: MouseEvent | TouchEvent) {
    isResizing.value = side
    resizeStartX.value = (e as MouseEvent).clientX || (e as TouchEvent).touches[0].clientX
    resizeStartW.value = side === 'sidebar' ? sidebarWidth.value : listWidth.value
    document.addEventListener('mousemove', onResizeMove)
    document.addEventListener('mouseup', onResizeEnd)
  }

  function onResizeMove(e: MouseEvent) {
    if (!isResizing.value) return
    const dx = e.clientX - resizeStartX.value
    const w = Math.max(minWidth, Math.min(maxWidth, resizeStartW.value + dx))
    if (isResizing.value === 'sidebar') sidebarWidth.value = w
    else listWidth.value = w
  }

  function onResizeEnd() {
    isResizing.value = null
    document.removeEventListener('mousemove', onResizeMove)
    document.removeEventListener('mouseup', onResizeEnd)
  }

  return {
    sidebarWidth,
    listWidth,
    isResizing,
    onResizeStart
  }
}
