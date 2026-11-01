/**
 * focus 深链定位：把列表中「已选中」的对象滚动进视口。
 * 各页约定：列表容器挂 ref，行级元素带 `data-<entity>-id` 标记；
 * 选中高亮 class 由各页自身渲染（这里只负责滚动到可见位置）。
 */
export function scrollToSelected(
  container: HTMLElement | null | undefined,
  dataAttr: string,
  id: string,
  block: ScrollLogicalPosition = 'nearest'
): void {
  if (!container || !id) return
  // 双 rAF：等 Vue 渲染与选中态 class 落位后再定位
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      const el = container.querySelector(`[data-${dataAttr}="${cssEscape(id)}"]`)
      if (el) el.scrollIntoView({ block, behavior: 'smooth' })
    })
  )
}

/** 属性选择器值转义（id 为 uuid 时通常安全，仍防御引号/反斜杠） */
function cssEscape(v: string): string {
  return v.replace(/["\\]/g, '\\$&')
}
