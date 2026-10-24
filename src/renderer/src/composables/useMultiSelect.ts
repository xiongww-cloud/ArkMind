import { ref, computed, type Ref } from 'vue'

export interface MultiSelectOptions<T> {
  /** 多选总开关（如 NotesPage 仅笔记本视图启用），返回 false 时选择操作全部忽略 */
  enabled?: () => boolean
  /** 区间选择时的过滤谓词（如 DeviceTable 排除子网行） */
  isSelectable?: (item: T) => boolean
  /** 取 id 的键，默认取 item.id */
  getKey?: (item: T) => string
}

/**
 * Excel 语义多选：
 * - 单击单选；Ctrl/⌘ 单击加减；Shift 区间（锚点起算）
 * - handleItemMouseDown：修饰键按下时不动选区（留给 click）；否则拖/点未选中项先单选
 * - markDragFinished：真实拖拽结束后吞掉浏览器补发的 click，避免打散选区
 */
export function useMultiSelect<T>(items: Ref<T[]>, opts: MultiSelectOptions<T> = {}) {
  const keyOf = opts.getKey ?? ((i: T) => (i as any).id as string)
  const enabled = () => (opts.enabled ? opts.enabled() : true)
  const selectable = (item: T) => (opts.isSelectable ? opts.isSelectable(item) : true)

  const selectedIds = ref<Set<string>>(new Set())
  const anchorId = ref<string | null>(null)

  function clear() {
    selectedIds.value = new Set()
    anchorId.value = null
  }

  function selectSingle(id: string) {
    if (!enabled()) return
    const s = new Set<string>()
    s.add(id)
    selectedIds.value = s
    anchorId.value = id
  }

  function toggle(id: string) {
    if (!enabled()) return
    const s = new Set(selectedIds.value)
    if (s.has(id)) s.delete(id)
    else s.add(id)
    selectedIds.value = s
    anchorId.value = id
  }

  function selectRange(id: string) {
    if (!enabled()) return
    if (!anchorId.value) {
      selectSingle(id)
      return
    }
    const ids = items.value.filter(selectable).map(keyOf)
    const a = ids.indexOf(anchorId.value)
    const b = ids.indexOf(id)
    if (a < 0 || b < 0) {
      selectSingle(id)
      return
    }
    const [lo, hi] = a <= b ? [a, b] : [b, a]
    selectedIds.value = new Set(ids.slice(lo, hi + 1))
  }

  function isSelected(id: string) {
    return selectedIds.value.has(id)
  }
  const selectedCount = computed(() => selectedIds.value.size)

  // ─── 统一鼠标入口 ────────────────────────────────────────────────────────
  let suppressNextClick = false
  /** 真实拖拽结束：吞掉随后浏览器补发的 click（400ms 内生效） */
  function markDragFinished() {
    suppressNextClick = true
    setTimeout(() => {
      suppressNextClick = false
    }, 400)
  }

  /** click 处理：ctrl/⌘ toggle，shift 区间，普通单选；拖后补发的 click 忽略 */
  function handleItemClick(e: MouseEvent, id: string) {
    if (!enabled()) return
    if (suppressNextClick) {
      suppressNextClick = false
      return
    }
    if (e.ctrlKey || e.metaKey) toggle(id)
    else if (e.shiftKey) selectRange(id)
    else selectSingle(id)
  }

  /** mousedown 处理：修饰键按下时不动选区；否则拖/点未选中项先单选。
   *  返回 true = 可以进入拖拽启动流程。 */
  function handleItemMouseDown(e: MouseEvent, id: string): boolean {
    if (!enabled()) return false
    if (e.ctrlKey || e.metaKey || e.shiftKey) return false
    if (!isSelected(id)) selectSingle(id)
    return true
  }

  return {
    selectedIds,
    anchorId,
    clear,
    selectSingle,
    selectRange,
    toggle,
    isSelected,
    selectedCount,
    handleItemClick,
    handleItemMouseDown,
    markDragFinished
  }
}
