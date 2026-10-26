<script setup lang="ts">
import { ref, reactive, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import type { PropType } from 'vue'
import { findModelSuggestion } from '@/utils/modelSuggestion'
import { lintParams, fixParams, paramsEquivalent, stripControlContent } from '@/utils/paramText'
import type { LintIssue } from '@/utils/paramText'
import { toast } from '@/composables/useToast'
import { confirm, dangerConfirm, choiceConfirm } from '@/composables/useConfirm'
import { useMultiSelect } from '@/composables/useMultiSelect'
import { useProductStore } from '@/stores/product'
import AppIcon from '@/components/common/AppIcon.vue'

const productStore = useProductStore()

const props = defineProps({
  items: { type: Array as PropType<any[]>, required: true },
  products: { type: Array as PropType<any[]>, default: () => [] },
  readonly: { type: Boolean, default: false },
  invalidUids: { type: Array as PropType<string[]>, default: () => [] }
})

const guid = () => Math.random().toString(36).substring(2, 11)
function selectAll(e: FocusEvent) {
  ;(e.target as HTMLInputElement)?.select()
}
function escBlur(e: KeyboardEvent) {
  ;(e.target as HTMLTextAreaElement)?.blur()
}

// ─── 参数格展开 / 收起（默认收起；点击参数即展开，表头按钮一键全展开/收起）──
const expandedMap = reactive<Record<string, boolean>>({})
function isExpanded(uid: string) {
  return !!expandedMap[uid]
}
function expandRow(uid: string) {
  expandedMap[uid] = true
}
function collapseRow(uid: string) {
  expandedMap[uid] = false
}

const expandableUids = computed(() =>
  props.items
    .filter((i: any) => i.rowType !== 'subnet' && (i.mergedText || '').trim())
    .map((i: any) => i.uid as string)
)
const allExpanded = computed(() => {
  const ids = expandableUids.value
  return ids.length > 0 && ids.every((uid: string) => !!expandedMap[uid])
})
function toggleAllExpand() {
  const next = !allExpanded.value
  for (const uid of expandableUids.value) expandedMap[uid] = next
}
const units = ['台', '套', '个', '年', '块', '根', '对']

function blankDevice() {
  return {
    uid: guid(),
    rowType: 'device',
    productId: '',
    model: '',
    customName: '',
    unitPrice: 0,
    quantity: 1,
    unit: '台',
    params: [],
    mergedText: '',
    syncParams: true
  }
}

// ─── 多选（共享 useMultiSelect：Excel 语义；仅设备可选中，子网行排除）──────
const {
  selectedIds: selUids,
  isSelected,
  selectSingle,
  handleItemClick,
  handleItemMouseDown,
  markDragFinished
} = useMultiSelect(computed(() => props.items) as any, {
  getKey: (i: any) => i.uid,
  isSelectable: (i: any) => i.rowType !== 'subnet'
})

function onRowClick(idx: number, e: MouseEvent) {
  // 修饰键点击已在捕获层处理；拖后补发的 click 由 composable 吞掉
  if (e.ctrlKey || e.metaKey || e.shiftKey) return
  const item = props.items[idx]
  if (!item || item.rowType === 'subnet') return
  handleItemClick(e, item.uid)
}
/** 捕获层：Ctrl/⌘/Shift + 点击行内任意位置（含输入框）都参与多选 */
function onRowCapture(idx: number, e: MouseEvent) {
  if (!(e.ctrlKey || e.metaKey || e.shiftKey)) return
  const item = props.items[idx]
  if (!item || item.rowType === 'subnet') return
  handleItemClick(e, item.uid)
}
/** 聚焦任意文本输入即切换选中该设备（单选） */
function ensureSel(uid: string) {
  selectSingle(uid)
}
function removeSelUid(uid: string) {
  if (!isSelected(uid)) return
  const s = new Set(selUids.value)
  s.delete(uid)
  selUids.value = s
}

// ─── 通用撤销 / 重做（快照历史；连续输入防抖合并为一步）────────────────────
const undoStack = ref<string[]>([])
const redoStack = ref<string[]>([])
const canUndo = computed(() => undoStack.value.length > 0)
const canRedo = computed(() => redoStack.value.length > 0)
let lastSnapshot = ''
let snapTimer: ReturnType<typeof setTimeout> | null = null
let restoring = false

function serializeItems(): string {
  return JSON.stringify(props.items)
}
function resetHistory() {
  if (snapTimer) {
    clearTimeout(snapTimer)
    snapTimer = null
  }
  lastSnapshot = serializeItems()
  undoStack.value = []
  redoStack.value = []
}
function flushPending() {
  if (snapTimer) {
    clearTimeout(snapTimer)
    snapTimer = null
  }
  const cur = serializeItems()
  if (cur !== lastSnapshot) {
    undoStack.value.push(lastSnapshot)
    if (undoStack.value.length > 50) undoStack.value.shift()
    redoStack.value = []
    lastSnapshot = cur
  }
}
function restoreSnapshot(json: string) {
  const arr = JSON.parse(json)
  restoring = true
  props.items.splice(0, props.items.length, ...arr)
  nextTick(() => {
    restoring = false
    resizeAllParams()
  })
  lastSnapshot = json
}
function undo() {
  flushPending()
  const prev = undoStack.value.pop()
  if (prev === undefined) return
  redoStack.value.push(serializeItems())
  restoreSnapshot(prev)
}
function redo() {
  flushPending()
  const next = redoStack.value.pop()
  if (next === undefined) return
  undoStack.value.push(serializeItems())
  restoreSnapshot(next)
}

// items 引用变化（切换草稿/版本/清单）→ 重置历史
watch(
  () => props.items,
  (v, old) => {
    if (v !== old) resetHistory()
  }
)
// 深度变化 → 防抖记一步（400ms 内连续编辑合并）
watch(
  () => props.items,
  () => {
    if (restoring) return
    if (snapTimer) clearTimeout(snapTimer)
    snapTimer = setTimeout(() => {
      snapTimer = null
      const cur = serializeItems()
      if (cur === lastSnapshot) return
      undoStack.value.push(lastSnapshot)
      if (undoStack.value.length > 50) undoStack.value.shift()
      redoStack.value = []
      lastSnapshot = cur
    }, 400)
  },
  { deep: true }
)

function onKeyUndoRedo(e: KeyboardEvent) {
  if (!(e.metaKey || e.ctrlKey)) return
  const t = e.target as HTMLElement | null
  // 焦点在文本编辑器内 → 交给原生撤销
  if (t && t.closest && t.closest('input, textarea, [contenteditable="true"]')) return
  if (e.key === 'z' && !e.shiftKey) {
    e.preventDefault()
    undo()
  } else if ((e.key === 'z' && e.shiftKey) || e.key === 'y') {
    e.preventDefault()
    redo()
  }
}

// ─── 删除撤销条（快捷入口，点击等同 undo）──────────────────────────────────
const undoBar = ref<{ label: string } | null>(null)
let undoTimer: ReturnType<typeof setTimeout> | null = null
function showUndoBar(label: string) {
  undoBar.value = { label }
  if (undoTimer) clearTimeout(undoTimer)
  undoTimer = setTimeout(() => {
    undoBar.value = null
  }, 8000)
}
function dismissUndoBar() {
  undoBar.value = null
  if (undoTimer) {
    clearTimeout(undoTimer)
    undoTimer = null
  }
}

// ─── 设备型号幽灵补全 ─────────────────────────────────────────────────────────
const modelGhosts = ref<Record<string, { tail: string; name: string; id: string; price?: number }>>({})

function setModelGhost(item: any, suggestion: ReturnType<typeof findModelSuggestion>) {
  const cur = { ...modelGhosts.value }
  const typed = String(item.model ?? '')
  if (
    suggestion &&
    typed &&
    suggestion.name.length > typed.length &&
    suggestion.name.toLowerCase().startsWith(typed.toLowerCase())
  ) {
    cur[item.uid] = {
      tail: suggestion.name.slice(typed.length),
      name: suggestion.name,
      id: suggestion.id,
      price: suggestion.price
    }
  } else {
    delete cur[item.uid]
  }
  modelGhosts.value = cur
}

function syncModelFromInput(e: any, item: any) {
  if (e.isComposing || e.inputType === 'insertCompositionText') return
  const typedValue = e.target.value
  item.model = typedValue
  item.productId = null
  if (!typedValue) {
    setModelGhost(item, null)
    return
  }
  setModelGhost(item, findModelSuggestion(typedValue, props.products))
}

function onDeviceCompositionEnd(e: any, item: any) {
  const typedValue = e.target.value
  item.model = typedValue
  autoGrow(e.target)
  if (!typedValue) {
    item.productId = null
    setModelGhost(item, null)
    return
  }
  setModelGhost(item, findModelSuggestion(typedValue, props.products))
}

/** 型号输入（textarea 自动换行）：同步数据 + 撑高 */
function onModelInput(e: any, item: any) {
  syncModelFromInput(e, item)
  autoGrow(e.target)
}

/** 名称输入（textarea 自动换行）：仅撑高 */
function onNameInput(e: Event) {
  autoGrow(e.target as HTMLTextAreaElement)
}

function acceptModelGhost(item: any) {
  const g = modelGhosts.value[item.uid]
  if (!g) return
  item.model = g.name
  item.productId = g.id || null
  if (g.price && !item.unitPrice) item.unitPrice = g.price
  setModelGhost(item, null)
}

function onDeviceKeydown(e: KeyboardEvent, item: any) {
  if (e.isComposing) return
  if (e.key === 'Tab') {
    if (modelGhosts.value[item.uid]) {
      e.preventDefault()
      acceptModelGhost(item)
    }
    return
  }
  if (e.key === 'ArrowRight') {
    const el = e.target as HTMLInputElement
    const atEnd = el.selectionStart === el.value.length && el.selectionEnd === el.value.length
    if (atEnd && modelGhosts.value[item.uid]) {
      e.preventDefault()
      acceptModelGhost(item)
    }
  }
}

/** 型号确认（Enter/blur）：匹配产品库回填关联/价格/参数 */
function handleDeviceConfirm(item: any, e?: any) {
  if (e?.isComposing) return
  setModelGhost(item, null)
  const text = (e?.target?.value || item.model || '').trim()
  if (!text) {
    item.productId = null
    item.model = ''
    return
  }
  item.model = text
  const match = props.products.find(
    (p: any) => (p.name || '').toLowerCase() === text.toLowerCase()
  )
  if (match) {
    item.productId = match.id
    item.model = match.name || match.model
    item.syncParams = false
    if (match.price && !item.unitPrice) item.unitPrice = match.price
    item.mergedText = item.mergedText || match.full_specs || ''
  } else {
    item.productId = null
    item.syncParams = true
  }
}

// ─── 单位下拉 ────────────────────────────────────────────────────────────────
const uo = ref<string | null>(null)
const upos = reactive({ top: 0, left: 0 })
function tud(uid: string, e: MouseEvent) {
  if (uo.value === uid) {
    uo.value = null
    return
  }
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
  upos.top = r.bottom + 4
  upos.left = r.left
  uo.value = uid
}
function sud(u: string) {
  const item = props.items.find((i) => i.uid === uo.value)
  if (item) item.unit = u
  uo.value = null
}
function cud(e: MouseEvent) {
  if (uo.value && !(e.target as HTMLElement).closest('[data-unit-dropdown]')) uo.value = null
}

// ─── 行操作 ──────────────────────────────────────────────────────────────────
// ─── 子网主题色（预设符合软件风格的柔和色板，按 uid 哈希稳定分配）──────────
const SUBNET_THEMES = [
  {
    key: 'indigo',
    bg: 'bg-indigo-50/60', soft: 'bg-indigo-50/[0.22]', border: 'border-indigo-100',
    text: 'text-indigo-600', textSoft: 'text-indigo-500/80', icon: 'text-indigo-400',
    input: 'text-indigo-700 placeholder:text-indigo-300 focus:ring-indigo-300',
    chevronHover: 'hover:text-indigo-600 hover:bg-white/70',
    add: 'border-indigo-200 text-indigo-400 hover:text-indigo-600 hover:border-indigo-400',
    del: 'border-indigo-200 text-indigo-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-indigo-300'
  },
  {
    key: 'sky',
    bg: 'bg-sky-50/60', soft: 'bg-sky-50/[0.22]', border: 'border-sky-100',
    text: 'text-sky-600', textSoft: 'text-sky-500/80', icon: 'text-sky-400',
    input: 'text-sky-700 placeholder:text-sky-300 focus:ring-sky-300',
    chevronHover: 'hover:text-sky-600 hover:bg-white/70',
    add: 'border-sky-200 text-sky-400 hover:text-sky-600 hover:border-sky-400',
    del: 'border-sky-200 text-sky-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-sky-300'
  },
  {
    key: 'emerald',
    bg: 'bg-emerald-50/60', soft: 'bg-emerald-50/[0.22]', border: 'border-emerald-100',
    text: 'text-emerald-600', textSoft: 'text-emerald-500/80', icon: 'text-emerald-400',
    input: 'text-emerald-700 placeholder:text-emerald-300 focus:ring-emerald-300',
    chevronHover: 'hover:text-emerald-600 hover:bg-white/70',
    add: 'border-emerald-200 text-emerald-400 hover:text-emerald-600 hover:border-emerald-400',
    del: 'border-emerald-200 text-emerald-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-emerald-300'
  },
  {
    key: 'amber',
    bg: 'bg-amber-50/60', soft: 'bg-amber-50/[0.22]', border: 'border-amber-100',
    text: 'text-amber-600', textSoft: 'text-amber-500/80', icon: 'text-amber-400',
    input: 'text-amber-700 placeholder:text-amber-300 focus:ring-amber-300',
    chevronHover: 'hover:text-amber-600 hover:bg-white/70',
    add: 'border-amber-200 text-amber-400 hover:text-amber-600 hover:border-amber-400',
    del: 'border-amber-200 text-amber-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-amber-300'
  },
  {
    key: 'violet',
    bg: 'bg-violet-50/60', soft: 'bg-violet-50/[0.22]', border: 'border-violet-100',
    text: 'text-violet-600', textSoft: 'text-violet-500/80', icon: 'text-violet-400',
    input: 'text-violet-700 placeholder:text-violet-300 focus:ring-violet-300',
    chevronHover: 'hover:text-violet-600 hover:bg-white/70',
    add: 'border-violet-200 text-violet-400 hover:text-violet-600 hover:border-violet-400',
    del: 'border-violet-200 text-violet-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-violet-300'
  },
  {
    key: 'rose',
    bg: 'bg-rose-50/60', soft: 'bg-rose-50/[0.22]', border: 'border-rose-100',
    text: 'text-rose-600', textSoft: 'text-rose-500/80', icon: 'text-rose-400',
    input: 'text-rose-700 placeholder:text-rose-300 focus:ring-rose-300',
    chevronHover: 'hover:text-rose-600 hover:bg-white/70',
    add: 'border-rose-200 text-rose-400 hover:text-rose-600 hover:border-rose-400',
    del: 'border-rose-200 text-rose-300 hover:text-red-500 hover:border-red-300',
    line: 'bg-rose-300'
  }
]
function themeFor(uid: string) {
  let h = 0
  for (let i = 0; i < uid.length; i++) h = (h * 31 + uid.charCodeAt(i)) >>> 0
  return SUBNET_THEMES[h % SUBNET_THEMES.length]
}

/** 子网容器：平铺模型下，子网行之后、下一个子网行之前的设备归属该子网；序号只编设备（子网不占号） */
const groups = computed(() => {
  const gs: Array<{
    subnet: any | null
    subnetIdx: number | null
    theme: (typeof SUBNET_THEMES)[number]
    devices: Array<{ item: any; idx: number; no: number }>
  }> = []
  let no = 0
  let cur = {
    subnet: null as any | null,
    subnetIdx: null as number | null,
    theme: SUBNET_THEMES[0],
    devices: [] as Array<{ item: any; idx: number; no: number }>
  }
  props.items.forEach((it: any, idx: number) => {
    if (it.rowType === 'subnet') {
      gs.push(cur)
      cur = { subnet: it, subnetIdx: idx, theme: themeFor(it.uid || String(idx)), devices: [] }
    } else {
      no++
      cur.devices.push({ item: it, idx, no })
    }
  })
  gs.push(cur)
  return gs
})

/** 子网块的结束下标（不含）：从子网行到下一个子网行之前 */
function subnetBlockEnd(start: number): number {
  const its = props.items
  let i = start + 1
  while (i < its.length && its[i]?.rowType !== 'subnet') i++
  return i
}

// 子网收起 / 展开（默认展开）
const subnetCollapsed = reactive<Record<string, boolean>>({})
function isSubnetOpen(uid: string) {
  return !subnetCollapsed[uid]
}
function toggleSubnet(uid: string) {
  subnetCollapsed[uid] = !subnetCollapsed[uid]
  // 子网内设备行是 v-if 重建的：展开后必须重新做 textarea 高度自适应
  resizeAllParams()
}

function subnetStats(devices: Array<{ item: any }>) {
  const count = devices.length
  const amount = devices.reduce(
    (s, d) => s + (d.item.rowType === 'subnet' ? 0 : (d.item.unitPrice || 0) * (d.item.quantity || 0)),
    0
  )
  return `${count} 台 · ¥${amount.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
}

async function removeRow(idx: number) {
  if (props.readonly) return
  const item = props.items[idx]
  if (!item) return
  if (item.rowType === 'subnet') {
    const end = subnetBlockEnd(idx)
    const n = end - idx - 1
    const name = item.name || '未命名子网'
    if (n === 0) {
      const ok = await dangerConfirm({
        title: '删除子网',
        message: `删除子网「${name}」？此操作可撤销。`,
        confirmText: '删除'
      })
      if (!ok) return
      props.items.splice(idx, 1)
      removeSelUid(item.uid)
      showUndoBar(`子网「${name}」`)
      return
    }
    const choice = await choiceConfirm({
      title: '删除子网',
      message: `子网「${name}」内含 ${n} 台设备，选择删除范围：\n\n· 仅删除子网 — 子网容器移除，其中 ${n} 台设备保留为普通行\n· 删除子网和设备 — 连同设备一并删除\n\n此操作可撤销。`,
      confirmText: '删除子网和设备',
      extraText: '仅删除子网',
      cancelText: '取消'
    })
    if (choice === 'confirm') {
      const removed = props.items.slice(idx, end)
      props.items.splice(idx, end - idx)
      removed.forEach((r: any) => removeSelUid(r.uid))
      showUndoBar(`子网「${name}」及其中 ${n} 台设备`)
    } else if (choice === 'extra') {
      props.items.splice(idx, 1)
      removeSelUid(item.uid)
      showUndoBar(`子网「${name}」（设备已保留）`)
    }
    return
  } else {
    props.items.splice(idx, 1)
    removeSelUid(item.uid)
    showUndoBar(`「${item.customName || item.model || '未命名设备'}」`)
  }
}

// ─── 行内悬浮「+」：向下添加设备 / 子网 ─────────────────────────────────────
const addMenu = ref<{ idx: number; x: number; y: number } | null>(null)
function openAddMenu(idx: number, e: MouseEvent) {
  e.stopPropagation()
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
  addMenu.value = { idx, x: r.left, y: r.bottom + 6 }
}
function insertAt(idx: number, type: 'device' | 'subnet', dir: 'above' | 'below' = 'below') {
  if (props.readonly) return
  const row =
    type === 'device' ? blankDevice() : { uid: guid(), rowType: 'subnet', name: '新子网' }
  const at = Math.max(0, Math.min(dir === 'below' ? idx + 1 : idx, props.items.length))
  props.items.splice(at, 0, row)
  addMenu.value = null
  if (type === 'device') nextTick(() => focusCell(at, 'customName'))
}

/** 在子网内添加设备：top = 成为第一个子设备；bottom = 成为最后一个子设备 */
function insertIntoSubnet(subnetIdx: number, pos: 'top' | 'bottom') {
  if (props.readonly) return
  const at = pos === 'top' ? subnetIdx + 1 : subnetBlockEnd(subnetIdx)
  props.items.splice(at, 0, blankDevice())
  addMenu.value = null
  nextTick(() => focusCell(at, 'customName'))
}
function closeAddMenu(e?: Event) {
  if (addMenu.value && !(e?.target as HTMLElement)?.closest?.('[data-add-menu]')) {
    addMenu.value = null
  }
}

// ─── 拖拽排序（RAF 节流 + 仅变化时写状态，拖动不卡）────────────────────────
const ghostRef = ref<HTMLElement | null>(null)
const dsi = ref<number | null>(null)
const dti = ref<number | null>(null)
const idr = ref(false)
const dragInsert = ref<{ targetIdx: number; side: 'above' | 'below' } | null>(null)
let dragTimer: ReturnType<typeof setTimeout> | null = null
let rafId = 0
let lastX = 0
let lastY = 0

function preventSelect(e: Event) {
  e.preventDefault()
}

/** ghost 直接改 DOM（不走响应式，mousemove 零重渲染） */
function placeGhost(x: number, y: number) {
  const g = ghostRef.value
  if (g) g.style.transform = `translate(${x + 12}px, ${y + 12}px)`
}

function ogd(e: MouseEvent, idx: number) {
  if (props.readonly) return
  dsi.value = idx
  lastX = e.clientX
  lastY = e.clientY
  document.body.style.userSelect = 'none'
  document.addEventListener('selectstart', preventSelect)
  document.addEventListener('mousemove', odm)
  dragTimer = setTimeout(() => {
    idr.value = true
    nextTick(() => placeGhost(lastX, lastY))
  }, 200)
  document.addEventListener('mouseup', oup)
}

/** 行内长按空白处（非输入/按钮区域）触发拖拽；拖未选中设备前先单选它 */
function onRowMousedown(e: MouseEvent, idx: number) {
  if (props.readonly) return
  // 修饰键多选点击：阻止浏览器原生文本选择/原生拖拽（click 事件不受影响，多选逻辑照常）
  if (e.shiftKey || e.ctrlKey || e.metaKey) {
    e.preventDefault()
    return
  }
  const t = e.target as HTMLElement
  if (t.closest('input, textarea, button, select, a, [data-no-drag]')) return
  const item = props.items[idx]
  if (item && item.rowType === 'subnet') {
    ogd(e, idx)
    return
  }
  // 修饰键按下 = 即将多选点击：不动选区也不起拖，留给 click 的 Ctrl/Shift 逻辑
  if (item && !handleItemMouseDown(e, item.uid)) return
  ogd(e, idx)
}

function odm(e: MouseEvent) {
  e.preventDefault()
  lastX = e.clientX
  lastY = e.clientY
  if (idr.value) placeGhost(e.clientX, e.clientY)
  if (!idr.value || rafId) return
  // 每帧最多算一次命中目标；dti/dragInsert 仅在真正变化时写入，避免整表重渲染
  rafId = requestAnimationFrame(() => {
    rafId = 0
    updateDragTarget(lastX, lastY)
  })
}

function updateDragTarget(x: number, y: number) {
  const el = document.elementFromPoint(x, y)
  const row = el?.closest('[data-item-idx]') as HTMLElement | null
  if (!row) {
    if (dti.value !== null) dti.value = null
    if (dragInsert.value !== null) dragInsert.value = null
    return
  }
  const tidx = parseInt(row.getAttribute('data-item-idx') || '')
  if (isNaN(tidx) || tidx === dsi.value) {
    if (dti.value !== null) dti.value = null
    if (dragInsert.value !== null) dragInsert.value = null
    return
  }
  const isSubnetTarget = row.hasAttribute('data-subnet-row')
  const draggingSubnet = props.items[dsi.value ?? -1]?.rowType === 'subnet'
  if (draggingSubnet) {
    if (!isSubnetTarget) {
      // 子网只能落在：其他子网行，或根级区域（首个子网行之前的设备行）
      let rootOk = true
      for (let i = tidx - 1; i >= 0; i--) {
        if (props.items[i]?.rowType === 'subnet') {
          rootOk = false
          break
        }
      }
      if (!rootOk) {
        if (dti.value !== null) dti.value = null
        if (dragInsert.value !== null) dragInsert.value = null
        return
      }
    }
  }
  if (dti.value !== tidx) dti.value = tidx
  const rect = row.getBoundingClientRect()
  let side: 'above' | 'below' = y - rect.top < rect.height / 2 ? 'above' : 'below'
  if (isSubnetTarget && !draggingSubnet) side = 'below'
  const cur = dragInsert.value
  if (!cur || cur.targetIdx !== tidx || cur.side !== side) {
    dragInsert.value = { targetIdx: tidx, side }
  }
}

function oup() {
  document.removeEventListener('mousemove', odm)
  document.removeEventListener('mouseup', oup)
  document.removeEventListener('selectstart', preventSelect)
  document.body.style.userSelect = ''
  if (dragTimer) {
    clearTimeout(dragTimer)
    dragTimer = null
  }
  if (rafId) {
    cancelAnimationFrame(rafId)
    rafId = 0
  }
  if (!idr.value) return
  markDragFinished() // 真实拖拽后浏览器可能补发 click，多选层会忽略
  const si = dsi.value
  const insert = dragInsert.value
  dsi.value = null
  dti.value = null
  idr.value = false
  dragInsert.value = null
  if (si === null || !insert || si >= props.items.length) return
  const its = props.items
  const dragged = its[si]
  if (!dragged) return

  if (dragged.rowType === 'subnet') {
    // 子网整体移动：块 = [si, 下一个子网行)
    const t = insert.targetIdx
    if (t === si || t >= its.length) return
    const tItem = its[t]
    if (!tItem) return
    const blockEnd = subnetBlockEnd(si)
    const blockLen = blockEnd - si
    // 目标插入位置（原始坐标）：子网行 above=行前 / below=块后；根级设备行 above=设备前 / below=设备后
    let pos: number
    if (tItem.rowType === 'subnet') {
      pos = insert.side === 'below' ? subnetBlockEnd(t) : t
    } else {
      pos = insert.side === 'below' ? t + 1 : t
    }
    if (pos > si && pos < blockEnd) return // 落在自身块内，无效
    if (pos === si || pos === blockEnd) return // 原位，无变化
    const block = its.splice(si, blockLen)
    let ins = pos
    if (pos > blockEnd) ins = pos - blockLen
    if (ins === si) return
    its.splice(ins, 0, ...block)
    return
  }

  // 设备移动（支持多选：拖选中行 → 整组按落点连续插入；子网不参与多选）
  const selIdxs: number[] = []
  its.forEach((it: any, i: number) => {
    if (it.rowType !== 'subnet' && selUids.value.has(it.uid)) selIdxs.push(i)
  })
  if (!selIdxs.includes(si)) selIdxs.push(si)
  selIdxs.sort((a, b) => a - b)
  // 拖拽目标本身在选中集合内 → 无需移动
  if (selIdxs.length > 1 && its[insert.targetIdx] && selUids.value.has(its[insert.targetIdx].uid)) {
    return
  }
  const blocks = selIdxs.map((i) => its[i])
  const pos = Math.min(insert.side === 'below' ? insert.targetIdx + 1 : insert.targetIdx, its.length)
  let ins = pos
  for (const i of selIdxs) if (i < pos) ins -= 1
  for (let k = selIdxs.length - 1; k >= 0; k--) its.splice(selIdxs[k], 1)
  ins = Math.max(0, Math.min(ins, its.length))
  its.splice(ins, 0, ...blocks)
}

// ─── 参数格：自适应高度 ──────────────────────────────────────────────────────
function autoGrow(el: HTMLTextAreaElement | null | undefined) {
  if (!el) return
  el.style.height = 'auto'
  // box-sizing:border-box 下需补回边框，否则末行被裁切
  const border = el.offsetHeight - el.clientHeight
  el.style.height = el.scrollHeight + border + 'px'
}
function resizeAllParams() {
  nextTick(() => {
    document
      .querySelectorAll('textarea[data-param-cell], textarea[data-model-cell]')
      .forEach((el) => autoGrow(el as HTMLTextAreaElement))
  })
}
onMounted(() => {
  document.addEventListener('click', cud)
  document.addEventListener('click', closeAddMenu)
  window.addEventListener('keydown', onKeyUndoRedo)
  resetHistory()
  resizeAllParams()
})
onUnmounted(() => {
  document.removeEventListener('click', cud)
  document.removeEventListener('click', closeAddMenu)
  document.removeEventListener('mousemove', odm)
  document.removeEventListener('mouseup', oup)
  window.removeEventListener('keydown', onKeyUndoRedo)
  if (undoTimer) clearTimeout(undoTimer)
  if (snapTimer) clearTimeout(snapTimer)
})
watch(() => props.items, resizeAllParams, { deep: true })
// 参数展开/收起会改变裁切容器，需重新撑高内部 textarea
watch(expandedMap, () => resizeAllParams())

function onParamInput(e: Event, item: any) {
  const el = e.target as HTMLTextAreaElement
  item.mergedText = el.value
  autoGrow(el)
  // 文本已变，作废该行的检查结果
  if (issues.value[item.uid]) {
    const next = { ...issues.value }
    delete next[item.uid]
    issues.value = next
  }
}

function copyParam(item: any) {
  const text = item.mergedText || ''
  if (!text) return
  navigator.clipboard
    .writeText(text)
    .then(() => toast({ message: '参数已复制', type: 'success' }))
    .catch(() => toast({ message: '复制失败', type: 'error' }))
}

// ─── 键盘导航（单行单元格）──────────────────────────────────────────────────
function onCellKeydown(e: KeyboardEvent, idx: number, cell: string) {
  if (e.isComposing) return
  if (e.key === 'Enter') {
    e.preventDefault()
    focusCell(nextEditableRow(idx), cell)
  }
  // Tab 交给浏览器原生焦点顺序
}

function nextEditableRow(from: number): number {
  const items = props.items
  let i = from + 1
  while (i < items.length && items[i]?.rowType === 'subnet') i++
  return i < items.length ? i : from
}

function focusCell(idx: number, cell: string) {
  nextTick(() => {
    const el = document.querySelector(`[data-cell="${cell}"][data-row-idx="${idx}"]`) as HTMLElement | null
    el?.focus()
    if (el && 'select' in el && typeof (el as HTMLInputElement).select === 'function') {
      ;(el as HTMLInputElement).select()
    }
  })
}

// ─── Excel TSV 粘贴：多格 → 跨行扩展 ────────────────────────────────────────
const SPREAD_FIELDS = ['customName', 'model', 'quantity', 'unit', 'unitPrice'] as const

function onCellPaste(e: ClipboardEvent, idx: number, cell: string) {
  if (props.readonly) return
  const text = e.clipboardData?.getData('text') || ''
  if (!text.includes('\t') && !text.includes('\n')) return // 单值粘贴走默认
  const startC = SPREAD_FIELDS.indexOf(cell as any)
  if (startC < 0) return
  e.preventDefault()
  const lines = text
    .replace(/\r\n/g, '\n')
    .split('\n')
    .filter((l, i, arr) => !(i === arr.length - 1 && l === ''))
  const items = props.items
  let r = idx
  for (const line of lines) {
    while (r < items.length && items[r]?.rowType === 'subnet') r++
    if (r >= items.length) items.push(blankDevice())
    let c = startC
    for (const v of line.split('\t')) {
      while (c >= SPREAD_FIELDS.length) {
        r++
        c = 0
        while (r < items.length && items[r]?.rowType === 'subnet') r++
        if (r >= items.length) items.push(blankDevice())
      }
      const it = items[r]
      const f = SPREAD_FIELDS[c]
      if (f === 'quantity') it.quantity = Math.max(1, parseFloat(v) || 1)
      else if (f === 'unitPrice') it.unitPrice = parseFloat(v) || 0
      else it[f] = v
      c++
    }
    r++
  }
}

// ─── 拉取产品库参数 ──────────────────────────────────────────────────────────
function findProduct(item: any) {
  const list = Array.isArray(props.products) ? props.products : []
  if (item.productId) {
    const byId = list.find((p: any) => p.id === item.productId)
    if (byId) return byId
  }
  const model = (item.model || '').trim()
  if (!model) return null
  return list.find((p: any) => (p.name || '').toLowerCase() === model.toLowerCase()) || null
}

/** 拉取单行参数；本地不一致时询问覆盖/保留 */
async function pullRow(item: any) {
  if (props.readonly) return
  const product = findProduct(item)
  if (!product) {
    toast({ message: '未匹配到产品库条目，无法拉取', type: 'warning' })
    return
  }
  const remote = product.full_specs || ''
  const local = item.mergedText || ''
  if (paramsEquivalent(local, remote)) {
    toast({ message: '参数与产品库一致，无需拉取', type: 'success' })
    return
  }
  if (local.trim()) {
    const ok = await confirm({
      title: '覆盖本地参数？',
      message: `「${item.customName || item.model}」的参数与产品库不一致。覆盖将以产品库参数替换本地修改。`,
      confirmText: '覆盖',
      cancelText: '保留本地'
    })
    if (!ok) return
  }
  item.mergedText = remote
  item.productId = product.id
  if (issues.value[item.uid]) {
    const next = { ...issues.value }
    delete next[item.uid]
    issues.value = next
  }
  resizeAllParams()
  toast({ message: `已拉取「${product.name}」参数`, type: 'success' })
}

/** 一键立即回写：把该设备当前参数写回产品库（覆盖 full_specs） */
const writtenBack = ref<Set<string>>(new Set())
async function writeBack(item: any) {
  if (props.readonly) return
  const text = (item.mergedText || '').trim()
  if (!text) {
    toast({ message: '该设备没有参数，无需回写', type: 'warning' })
    return
  }
  const product = findProduct(item)
  if (!product) {
    toast({ message: '未匹配到产品库条目，无法回写', type: 'warning' })
    return
  }
  const ok = await confirm({
    title: '立即回写产品库',
    message: `将「${item.customName || item.model || '该设备'}」当前的技术参数立即写回产品库「${product.name}」，覆盖产品库中该产品现有的技术参数。是否继续？`,
    confirmText: '立即回写',
    cancelText: '取消'
  })
  if (!ok) return
  try {
    await productStore.updateProduct(product.id, { fullSpecs: item.mergedText })
    item.productId = product.id
    const s = new Set(writtenBack.value)
    s.add(item.uid)
    writtenBack.value = s
    toast({ message: `已回写「${product.name}」参数`, type: 'success' })
  } catch (e: any) {
    toast({ message: '回写失败：' + (e?.message || '请重试'), type: 'error' })
  }
}

/** 批量拉取：只填充参数为空的已匹配设备 */
function pullAllEmpty() {
  if (props.readonly) return
  let filled = 0
  let skipped = 0
  for (const item of props.items) {
    if (item.rowType === 'subnet') continue
    const product = findProduct(item)
    if (!product || !product.full_specs) {
      if (item.productId || item.model) skipped++
      continue
    }
    if ((item.mergedText || '').trim()) {
      skipped++
      continue
    }
    item.mergedText = product.full_specs
    item.productId = product.id
    filled++
  }
  resizeAllParams()
  if (filled > 0) toast({ message: `已填充 ${filled} 台设备的参数${skipped ? `（跳过 ${skipped} 台）` : ''}`, type: 'success' })
  else toast({ message: '没有可填充的空白参数（设备需已匹配产品库）', type: 'warning' })
}

// ─── 检查 / 一键修复 ─────────────────────────────────────────────────────────
const issues = ref<Record<string, LintIssue[]>>({})
const showIssues = ref(false)

function deviceLabel(item: any, idx: number) {
  return item.customName || item.model || `第 ${idx + 1} 行`
}

function lintAll() {
  const next: Record<string, LintIssue[]> = {}
  let total = 0
  ;props.items.forEach((item) => {
    if (item.rowType === 'subnet' || !(item.mergedText || '').trim()) return
    const found = lintParams(item.mergedText)
    if (found.length) {
      next[item.uid] = found
      total += found.length
    }
  })
  issues.value = next
  showIssues.value = true
  if (total === 0) toast({ message: '全部设备参数检查通过', type: 'success' })
}

function fixAll() {
  if (props.readonly) return
  const totals = { seq: 0, sep: 0, colon: 0, space: 0, punct: 0, ending: 0 }
  let devices = 0
  for (const item of props.items) {
    if (item.rowType === 'subnet' || !(item.mergedText || '').trim()) continue
    const before = item.mergedText
    const { text, fixes } = fixParams(before)
    if (text !== before) {
      item.mergedText = text
      devices++
      totals.seq += fixes.seq
      totals.sep += fixes.sep
      totals.colon += fixes.colon
      totals.space += fixes.space
      totals.punct += fixes.punct
      totals.ending += fixes.ending
    }
  }
  issues.value = {}
  showIssues.value = false
  resizeAllParams()
  const parts: string[] = []
  if (totals.seq) parts.push(`重排序号 ${totals.seq} 处`)
  if (totals.sep) parts.push(`规范分隔符 ${totals.sep} 处`)
  if (totals.colon) parts.push(`全角冒号 ${totals.colon} 处`)
  if (totals.space) parts.push(`清理空格 ${totals.space} 处`)
  if (totals.punct) parts.push(`统一标点 ${totals.punct} 处`)
  if (totals.ending) parts.push(`行尾标点 ${totals.ending} 处`)
  if (parts.length) toast({ message: `已修复 ${devices} 台设备：${parts.join('、')}`, type: 'success' })
  else toast({ message: '未发现需要修复的问题', type: 'success' })
}

/** 删除控标内容：剥 ★/▲ 符号（保留行）、删证明材料段（截图/证明/报告/盖章），删除后自动标点修复。撤销走通用快照。 */
function stripControlAll() {
  if (props.readonly) return
  let devices = 0
  let marks = 0
  let segments = 0
  for (const item of props.items) {
    if (item.rowType === 'subnet' || !(item.mergedText || '').trim()) continue
    const before = item.mergedText
    const r = stripControlContent(before)
    if (r.text !== before) {
      item.mergedText = r.text
      devices++
      marks += r.marksStripped
      segments += r.segmentsRemoved
    }
  }
  issues.value = {}
  showIssues.value = false
  resizeAllParams()
  if (devices === 0) {
    toast({ message: '未发现控标内容', type: 'success' })
    return
  }
  const parts: string[] = []
  if (marks) parts.push(`控标符号 ${marks} 处`)
  if (segments) parts.push(`证明材料段 ${segments} 段`)
  toast({ message: `已清理 ${devices} 台设备：${parts.length ? parts.join('、') : '无命中项'}`, type: 'success' })
}

function jumpToIssue(uid: string) {
  const idx = props.items.findIndex((i) => i.uid === uid)
  if (idx < 0) return
  selectSingle(uid)
  nextTick(() => {
    const row = document.querySelector(`[data-item-idx="${idx}"]`)
    row?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  })
}

const issueCount = computed(() => Object.values(issues.value).reduce((s, a) => s + a.length, 0))

function unitBtnClass(u: string) {
  return props.items.find((i) => i.uid === uo.value)?.unit === u
    ? 'text-blue-600 font-medium'
    : 'text-surface-600'
}
function issueLabel(uid: unknown) {
  const item = props.items.find((i) => i.uid === uid)
  return item ? deviceLabel(item, 0) : String(uid)
}
function dragLabel() {
  const it = props.items[dsi.value ?? 0]
  if (it && it.rowType !== 'subnet' && selUids.value.size > 1 && selUids.value.has(it.uid)) {
    return `移动 ${selUids.value.size} 项`
  }
  return it?.customName || it?.name || it?.model || '拖动'
}

defineExpose({ lintAll, fixAll, stripControlAll, pullAllEmpty, undo, redo, canUndo, canRedo })

// ─── 展示辅助 ────────────────────────────────────────────────────────────────
function subtotal(item: any) {
  return ((item.unitPrice || 0) * (item.quantity || 0)).toLocaleString('zh-CN', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  })
}
</script>

<template>
  <div class="flex flex-col min-h-0 flex-1 overflow-hidden">
    <!-- 表头 -->
    <div
      class="flex items-stretch border-b border-surface-200 bg-surface-50/80 text-[11px] font-semibold text-surface-500 uppercase tracking-wider shrink-0"
    >
      <div class="w-[44px] shrink-0 px-1 py-1.5 flex items-center justify-center">序号</div>
      <div class="w-[120px] shrink-0 px-2 py-1.5 flex items-center justify-center">设备名称</div>
      <div class="w-[160px] shrink-0 px-2 py-1.5 flex items-center justify-center">型号</div>
      <div class="flex-1 min-w-[200px] px-2 py-1.5 flex items-center justify-between">
        <span>参数</span>
        <button
          @click="toggleAllExpand"
          class="flex items-center gap-0.5 text-[11px] font-normal normal-case text-surface-400 hover:text-blue-600 transition-colors"
          :title="allExpanded ? '收起全部参数' : '展开全部参数'"
        >
          <span>{{ allExpanded ? '收起' : '展开' }}</span>
          <AppIcon
            name="chevron-down"
            class="w-3 h-3 transition-transform"
            :class="allExpanded ? 'rotate-180' : ''"
          />
        </button>
      </div>
      <div class="w-[56px] shrink-0 px-1 py-1.5 flex items-center justify-center">数量</div>
      <div class="w-[52px] shrink-0 px-1 py-1.5 flex items-center justify-center">单位</div>
      <div class="w-[92px] shrink-0 px-2 py-1.5 flex items-center justify-center">单价</div>
      <div class="w-[100px] shrink-0 px-2 py-1.5 flex items-center justify-center">小计</div>
    </div>

    <!-- 行列表 -->
    <div
      class="flex-1 overflow-y-auto min-h-0 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full"
    >
      <div
        v-if="items.length === 0"
        class="text-center py-16 text-gray-400 text-xs space-y-3"
      >
        <p>{{ readonly ? '此版本为空。' : '清单为空，先添加设备或子网。' }}</p>
        <div v-if="!readonly" class="flex items-center justify-center gap-2">
          <button @click="insertAt(-1, 'device')" class="btn-secondary">
            <AppIcon name="plus" class="w-3.5 h-3.5" />添加设备
          </button>
          <button @click="insertAt(-1, 'subnet')" class="btn-secondary">
            <AppIcon name="folder" class="w-3.5 h-3.5" />添加子网
          </button>
        </div>
      </div>

      <template v-for="group in groups" :key="group.subnet?.uid || '__root__'">
        <!-- 子网容器头（主题色） -->
        <div
          v-if="group.subnet"
          :data-item-idx="group.subnetIdx"
          data-subnet-row
          @mousedown="onRowMousedown($event, group.subnetIdx ?? 0)"
          class="relative flex items-stretch border-b group transition-all"
          :class="[
            group.theme.bg,
            group.theme.border,
            idr && dti === group.subnetIdx ? 'outline outline-2 outline-dashed outline-blue-400 -outline-offset-2' : '',
            idr && dsi === group.subnetIdx ? 'opacity-40' : ''
          ]"
        >
          <div
            v-if="idr && dragInsert?.targetIdx === group.subnetIdx && dragInsert?.side === 'above'"
            class="absolute -top-[2px] left-0 right-0 z-20 h-[4px] bg-blue-500 rounded-full pointer-events-none"
          />
          <div
            v-if="idr && dragInsert?.targetIdx === group.subnetIdx && dragInsert?.side === 'below'"
            class="absolute -bottom-[2px] left-0 right-0 z-20 h-[4px] bg-blue-500 rounded-full pointer-events-none"
          />

          <div class="flex-1 flex items-center gap-1.5 px-2 py-1.5 min-w-0">
            <button
              @click.stop="toggleSubnet(group.subnet.uid)"
              class="shrink-0 w-4 h-4 rounded flex items-center justify-center transition-colors"
              :class="[group.theme.icon, group.theme.chevronHover]"
              :title="isSubnetOpen(group.subnet.uid) ? '收起子网' : '展开子网'"
            >
              <AppIcon
                name="chevron-down"
                class="w-3 h-3 transition-transform"
                :class="isSubnetOpen(group.subnet.uid) ? '' : '-rotate-90'"
              />
            </button>
            <AppIcon name="folder" class="w-3.5 h-3.5 shrink-0" :class="group.theme.icon" />
            <input
              v-if="!readonly"
              v-model="group.subnet.name"
              @click.stop
              type="text"
              placeholder="子网名称"
              class="w-[160px] shrink-0 bg-transparent text-[12px] h-6 px-1 font-medium outline-none focus:bg-white focus:ring-1 rounded"
              :class="group.theme.input"
            />
            <span v-else class="w-[160px] shrink-0 text-[12px] font-medium truncate" :class="group.theme.text">{{
              group.subnet.name || '未命名子网'
            }}</span>
            <!-- 名称右侧：纯图标按钮（添加设备 + 删除子网） -->
            <button
              v-if="!readonly"
              @click.stop="openAddMenu(group.subnetIdx ?? 0, $event)"
              data-add-menu
              class="shrink-0 w-5 h-5 rounded flex items-center justify-center transition-colors"
              :class="group.theme.add"
              v-tooltip="'在子网中添加设备'"
            >
              <AppIcon name="plus" class="w-3 h-3" />
            </button>
            <button
              v-if="!readonly"
              @click.stop="removeRow(group.subnetIdx ?? 0)"
              class="shrink-0 w-5 h-5 rounded flex items-center justify-center transition-colors"
              :class="group.theme.del"
              v-tooltip="'删除子网（可选保留设备）'"
            >
              <AppIcon name="trash" class="w-3 h-3" />
            </button>
            <div class="flex-1 min-w-0" />
            <span
              data-no-drag
              select-text
              class="shrink-0 text-[11px] ml-1"
              :class="group.theme.textSoft"
              >{{ subnetStats(group.devices) }}</span
            >
          </div>
        </div>

        <!-- 子网内设备（收起时隐藏；左侧蓝色标条标识归属） -->
        <template v-if="!group.subnet || isSubnetOpen(group.subnet.uid)">
          <div
            v-for="{ item, idx, no } in group.devices"
            :key="item.uid"
            :data-item-idx="idx"
            @click.capture="onRowCapture(idx, $event)"
            @click="onRowClick(idx, $event)"
            @mousedown="onRowMousedown($event, idx)"
            class="relative flex items-stretch border-b border-surface-100 transition-all group cursor-pointer"
            :style="isSelected(item.uid) ? { backgroundColor: 'rgba(191, 219, 254, 0.55)' } : {}"
            :class="[
              isSelected(item.uid) ? 'ring-1 ring-inset ring-blue-400/60' : 'hover:bg-surface-50/60',
              group.subnet ? group.theme.soft : 'bg-white',
              idr && dti === idx ? 'outline outline-2 outline-dashed outline-blue-400 -outline-offset-2' : '',
              idr && dsi === idx ? 'opacity-40' : '',
              invalidUids.includes(item.uid) ? 'bg-red-50/40' : ''
            ]"
          >
            <div
              v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'above'"
              class="absolute -top-[2px] left-0 right-0 z-20 h-[4px] bg-blue-500 rounded-full pointer-events-none"
            />
            <div
              v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'below'"
              class="absolute -bottom-[2px] left-0 right-0 z-20 h-[4px] bg-blue-500 rounded-full pointer-events-none"
            />
            <!-- 子网归属标条（主题色） -->
            <div
              v-if="group.subnet"
              class="absolute left-0 top-0 bottom-0 w-[3px] rounded-l"
              :class="group.theme.line"
            />

            <!-- 序号 -->
            <div
              class="w-[44px] shrink-0 flex items-center justify-center text-[11px] text-surface-400 select-none transition-opacity"
              :class="!readonly ? 'group-hover:opacity-0' : ''"
            >
              {{ no }}
            </div>

            <!-- 悬浮删除（占序号位；仅点击小圆钮本体才删除） -->
            <div
              v-if="!readonly"
              class="absolute left-0 top-0 bottom-0 w-[44px] z-20 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none"
            >
              <button
                @click.stop="removeRow(idx)"
                v-tooltip="'删除该行'"
                class="pointer-events-auto w-[18px] h-[18px] rounded-full border border-gray-200 bg-white shadow-sm text-gray-400 flex items-center justify-center hover:text-red-500 hover:border-red-300"
              >
                <AppIcon name="x" class="w-2.5 h-2.5" />
              </button>
            </div>

            <!-- 设备名称（支持换行，完整显示） -->
            <div class="w-[120px] shrink-0 px-1.5 flex items-center">
              <textarea
                v-if="!readonly"
                v-model="item.customName"
                @input="onNameInput"
                @click.stop
                @focus="ensureSel(item.uid)"
                @keydown="onCellKeydown($event, idx, 'customName')"
                @paste="onCellPaste($event, idx, 'customName')"
                data-model-cell
                data-cell="customName"
                :data-row-idx="idx"
                rows="1"
                placeholder="名称"
                class="block w-full min-h-[28px] resize-none overflow-hidden px-1.5 py-1 text-[12px] leading-snug text-gray-700 bg-gray-50/60 border border-transparent hover:border-gray-200 rounded focus:bg-white focus:border-blue-400 focus:ring-1 focus:ring-blue-400 outline-none placeholder:text-gray-350"
              ></textarea>
              <div v-else class="text-[12px] text-gray-700 leading-snug break-all w-full">
                {{ item.customName || '—' }}
              </div>
            </div>

            <!-- 型号（支持换行，完整显示） -->
            <div class="w-[160px] shrink-0 px-1.5 flex items-center">
              <div v-if="!readonly" class="relative w-full">
                <textarea
                  :value="item.model"
                  @input="onModelInput($event, item)"
                  @compositionend="onDeviceCompositionEnd($event, item)"
                  @keydown="onDeviceKeydown($event, item)"
                  @keydown.enter.prevent="handleDeviceConfirm(item, $event)"
                  @keydown.esc="escBlur"
                  @blur="handleDeviceConfirm(item, $event)"
                  @click.stop
                  @focus="ensureSel(item.uid)"
                  @paste="onCellPaste($event, idx, 'model')"
                  data-model-cell
                  data-cell="model"
                  :data-row-idx="idx"
                  rows="1"
                  placeholder="型号"
                  class="block w-full min-h-[28px] resize-none overflow-hidden px-1.5 py-1 text-[12px] leading-snug text-gray-700 bg-gray-50/60 border border-transparent hover:border-gray-200 rounded focus:bg-white focus:border-blue-400 focus:ring-1 focus:ring-blue-400 outline-none placeholder:text-gray-350"
                  :class="invalidUids.includes(item.uid) && !item.productId && !item.model ? 'border-red-300 bg-red-50/50' : ''"
                ></textarea>
                <span
                  v-if="modelGhosts[item.uid]"
                  class="absolute left-1.5 right-1.5 top-1 flex items-start text-[12px] leading-snug text-gray-300 pointer-events-none select-none overflow-hidden"
                >
                  <span class="invisible">{{ item.model }}</span>{{ modelGhosts[item.uid].tail }}
                </span>
              </div>
              <div v-else class="text-[12px] text-gray-700 leading-snug break-all w-full">
                {{ item.model || '—' }}
              </div>
            </div>

            <!-- 参数（默认收起两行，可下拉展开） -->
            <div class="flex-1 min-w-[200px] px-1.5 relative flex items-center group/param">
              <div
                class="w-full"
                :class="isExpanded(item.uid) ? '' : 'max-h-[52px] overflow-hidden'"
              >
                <textarea
                  v-if="!readonly"
                  :value="item.mergedText"
                  @input="onParamInput($event, item)"
                  @focus="ensureSel(item.uid); expandRow(item.uid)"
                  @keydown.esc="escBlur"
                  data-param-cell
                  :data-cell="'params'"
                  :data-row-idx="idx"
                  rows="1"
                  placeholder="1、指标：要求"
                  class="block w-full min-h-[28px] resize-none overflow-hidden text-[12px] leading-relaxed text-gray-700 bg-gray-50/40 border px-2 py-1 rounded outline-none focus:bg-white focus:border-blue-400 focus:ring-1 focus:ring-blue-400 placeholder:text-gray-300"
                  :class="
                    issues[item.uid]
                      ? 'border-amber-400 bg-amber-50/50'
                      : invalidUids.includes(item.uid)
                        ? 'border-red-200'
                        : 'border-transparent hover:border-gray-200'
                  "
                ></textarea>
                <div
                  v-else
                  @click="expandRow(item.uid)"
                  class="text-[12px] leading-relaxed text-gray-700 whitespace-pre-wrap break-words px-2 py-1 min-h-[28px] cursor-pointer"
                  :class="item.mergedText ? '' : 'text-gray-300'"
                >{{ item.mergedText || '—' }}</div>
              </div>
              <!-- 悬浮操作区：问题数 / 回写 / 复制 / 拉取（在裁切层之外，带底色） -->
              <div
                class="absolute bottom-1 right-2 flex items-center gap-0.5 opacity-0 group-hover/param:opacity-100 focus-within:opacity-100 transition-opacity z-10"
              >
                <span
                  v-if="issues[item.uid]"
                  class="inline-flex items-center gap-0.5 px-1 h-5 rounded border border-amber-200 text-[10px] font-medium bg-amber-100 text-amber-700 cursor-pointer shadow-sm"
                  v-tooltip="`${issues[item.uid].length} 处问题，点击查看`"
                  @click.stop="jumpToIssue(item.uid)"
                >
                  <AppIcon name="alert-circle" class="w-3 h-3" />{{ issues[item.uid].length }}
                </span>
                <button
                  v-if="isExpanded(item.uid)"
                  @click.stop="collapseRow(item.uid)"
                  class="w-5 h-5 flex items-center justify-center rounded border border-surface-200 bg-white shadow-sm text-surface-400 hover:text-blue-600"
                  v-tooltip="'收起参数'"
                >
                  <AppIcon name="chevron-down" class="w-3 h-3 rotate-180" />
                </button>
                <button
                  v-if="!readonly"
                  @click.stop="writeBack(item)"
                  class="w-5 h-5 flex items-center justify-center rounded border shadow-sm transition-colors"
                  :class="
                    writtenBack.has(item.uid)
                      ? 'bg-emerald-50 border-emerald-200 text-emerald-600'
                      : 'bg-white border-surface-200 text-surface-400 hover:text-emerald-600'
                  "
                  v-tooltip="
                    writtenBack.has(item.uid)
                      ? '已回写产品库（再次点击可重新回写）'
                      : '立即回写产品库（把当前参数写回产品库）'
                  "
                >
                  <AppIcon name="upload" class="w-3 h-3" />
                </button>
                <button
                  v-if="item.mergedText"
                  @click.stop="copyParam(item)"
                  class="w-5 h-5 flex items-center justify-center rounded border border-surface-200 bg-white shadow-sm text-surface-400 hover:text-blue-600"
                  v-tooltip="'复制参数'"
                >
                  <AppIcon name="clipboard" class="w-3 h-3" />
                </button>
                <button
                  v-if="!readonly"
                  @click.stop="pullRow(item)"
                  class="w-5 h-5 flex items-center justify-center rounded border border-surface-200 bg-white shadow-sm text-surface-400 hover:text-emerald-600"
                  v-tooltip="'拉取产品库参数'"
                >
                  <AppIcon name="refresh" class="w-3 h-3" />
                </button>
              </div>
            </div>

            <!-- 数量 -->
            <div class="w-[56px] shrink-0 px-1 py-1 flex items-center justify-center">
              <input
                v-if="!readonly"
              v-model.number="item.quantity"
              @click.stop
              @focus="selectAll; ensureSel(item.uid)"
              @mouseup.prevent
              @blur="item.quantity = Math.max(1, item.quantity || 1)"
                @keydown="onCellKeydown($event, idx, 'quantity')"
                @paste="onCellPaste($event, idx, 'quantity')"
                data-cell="quantity"
                :data-row-idx="idx"
                type="number"
                min="1"
                placeholder="1"
                class="w-full h-7 px-1 text-center text-[12px] text-gray-700 bg-gray-50/60 border border-transparent hover:border-gray-200 rounded focus:bg-white focus:border-blue-400 focus:ring-1 focus:ring-blue-400 outline-none [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
              />
              <span v-else class="text-[12px] text-gray-700">{{ item.quantity }}</span>
            </div>

            <!-- 单位 -->
            <div class="w-[52px] shrink-0 px-1 py-1 flex items-center justify-center">
              <button
                v-if="!readonly"
                @click.stop="tud(item.uid, $event)"
                data-unit-dropdown
                data-cell="unit"
                :data-row-idx="idx"
                @keydown="onCellKeydown($event, idx, 'unit')"
                class="w-full h-7 flex items-center justify-center gap-0.5 text-[12px] text-gray-600 bg-gray-50/60 border border-transparent hover:border-gray-200 rounded focus:bg-white focus:border-blue-400 outline-none"
              >
                <span>{{ item.unit || '台' }}</span>
                <AppIcon name="chevron-down" class="w-2.5 h-2.5 text-gray-300" />
              </button>
              <span v-else class="text-[12px] text-gray-700">{{ item.unit || '台' }}</span>
            </div>

            <!-- 单价 -->
            <div class="w-[92px] shrink-0 px-1.5 py-1 flex items-center">
              <input
                v-if="!readonly"
                v-model.number="item.unitPrice"
                @click.stop
                @focus="selectAll; ensureSel(item.uid)"
                @mouseup.prevent
                @keydown="onCellKeydown($event, idx, 'unitPrice')"
                @paste="onCellPaste($event, idx, 'unitPrice')"
                data-cell="unitPrice"
                :data-row-idx="idx"
                type="number"
                min="0"
                step="100"
                placeholder="0"
                class="w-full h-7 px-1.5 text-right text-[12px] text-gray-700 bg-gray-50/60 border border-transparent hover:border-gray-200 rounded focus:bg-white focus:border-blue-400 focus:ring-1 focus:ring-blue-400 outline-none [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none"
                :class="invalidUids.includes(item.uid) && (!item.unitPrice || item.unitPrice <= 0) ? 'border-red-300 bg-red-50/50' : ''"
              />
              <span v-else class="text-[12px] text-gray-700 w-full text-right">{{
                item.unitPrice?.toLocaleString('zh-CN') || 0
              }}</span>
            </div>

            <!-- 小计（可选中复制，不触发拖拽） -->
            <div class="w-[100px] shrink-0 px-2 py-1 flex items-center justify-end">
              <span data-no-drag select-text class="text-[12px] font-mono text-blue-600 cursor-text">¥{{ subtotal(item) }}</span>
            </div>
          </div>
        </template>
      </template>
    </div>

    <!-- 检查结果面板 -->
    <div
      v-if="showIssues && issueCount > 0"
      class="shrink-0 border-t border-amber-200 bg-amber-50/70 max-h-40 overflow-y-auto"
    >
      <div class="px-3 py-1.5 flex items-center justify-between border-b border-amber-200/60 sticky top-0 bg-amber-50/95">
        <span class="text-[11px] font-semibold text-amber-700"
          >发现 {{ issueCount }} 处参数问题</span
        >
        <div class="flex items-center gap-2">
          <button
            v-if="!readonly"
            @click="fixAll"
            class="text-[11px] font-medium text-white bg-amber-600 hover:bg-amber-700 rounded px-2 py-0.5"
            >一键修复</button
          >
          <button @click="showIssues = false" class="text-amber-500 hover:text-amber-700">
            <AppIcon name="x" class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
      <div class="px-3 py-1">
        <div
          v-for="(list, uid) in issues"
          :key="uid"
          class="py-1 border-b border-amber-100 last:border-0"
        >
          <button
            @click="jumpToIssue(String(uid))"
            class="text-[11px] font-medium text-amber-800 hover:underline"
            >{{ issueLabel(uid) }}</button
          >
          <div v-for="(iss, j) in list" :key="j" class="text-[11px] text-amber-700/90 ml-3">
            <span v-if="iss.line" class="font-mono mr-1">L{{ iss.line }}</span>{{ iss.message }}
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- 删除撤销条（与 Toast 同区：右下角，悬浮于其上） -->
  <Teleport to="body">
    <div
      v-if="undoBar"
      class="fixed bottom-20 right-6 z-[100] flex items-center gap-3 px-4 py-2 rounded-lg bg-gray-800/95 text-white text-[12px] shadow-xl backdrop-blur"
    >
      <span class="max-w-[320px] truncate">已删除 {{ undoBar.label }}</span>
      <button
        @click="undo(); dismissUndoBar()"
        class="px-2 py-0.5 rounded bg-white/15 hover:bg-white/25 font-medium transition-colors"
        >撤销</button
      >
    </div>
  </Teleport>

  <!-- 子网「+」下拉菜单：在子网中添加设备 -->
  <Teleport to="body">
    <div
      v-if="addMenu"
      data-add-menu
      class="fixed z-[90] bg-white shadow-lg border border-gray-100 rounded-lg py-1 min-w-[132px]"
      :style="{ left: addMenu.x + 'px', top: addMenu.y + 'px' }"
    >
      <div class="px-3 pt-1 pb-1 text-[10px] text-surface-400 uppercase tracking-wider">
        在子网中添加设备
      </div>
      <button
        data-add-menu
        @click.stop="insertIntoSubnet(addMenu.idx, 'top')"
        class="w-full px-3 py-1.5 text-xs text-left whitespace-nowrap hover:bg-blue-50 hover:text-blue-600 flex items-center gap-2"
      >
        <AppIcon name="plus" class="w-3 h-3" />添加到子网顶部
      </button>
      <button
        data-add-menu
        @click.stop="insertIntoSubnet(addMenu.idx, 'bottom')"
        class="w-full px-3 py-1.5 text-xs text-left whitespace-nowrap hover:bg-blue-50 hover:text-blue-600 flex items-center gap-2"
      >
        <AppIcon name="plus" class="w-3 h-3" />添加到子网底部
      </button>
    </div>
  </Teleport>

  <!-- 单位下拉（Teleported；宽度自适应内容） -->
  <Teleport to="body">
    <div
      v-if="uo"
      data-unit-dropdown
      class="fixed z-[80] bg-white shadow-lg border border-gray-100 rounded-lg py-1 inline-flex flex-col"
      :style="{ top: upos.top + 'px', left: upos.left + 'px' }"
    >
      <button
        v-for="u in units"
        :key="u"
        @click.stop="sud(u)"
        data-unit-dropdown
        class="px-3 py-1.5 text-sm text-left whitespace-nowrap transition-colors hover:bg-blue-50 hover:text-blue-600"
        :class="unitBtnClass(u)"
      >
        {{ u }}
      </button>
    </div>
  </Teleport>

  <!-- 拖拽 ghost -->
  <Teleport to="body">
    <div
      v-if="idr"
      ref="ghostRef"
      class="fixed left-0 top-0 px-3 py-1.5 bg-blue-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap will-change-transform"
    >
      <AppIcon name="grip-vertical" class="w-3 h-3 inline-block mr-1.5 -mt-px" />
      {{ dragLabel() }}
    </div>
  </Teleport>
</template>
