import type { Directive, DirectiveBinding } from 'vue'

/**
 * v-tooltip —— 全站统一悬浮气泡
 *
 * 样式基线 = Sidebar tab 气泡：bg-gray-800 / 白字 11px / px-2 py-1 / rounded / shadow-lg / opacity 150ms 淡入
 *
 * 用法：
 *   v-tooltip="'编辑'"                 默认显示在元素上方
 *   v-tooltip.right="item.label"      显示在右侧（Sidebar 向）
 *   v-tooltip.bottom / .left          下方 / 左侧
 *   v-tooltip.auto="longText"         仅当内容被截断（scroll > client）时显示
 *   文本为空 / falsy                   不显示
 */

type Placement = 'top' | 'bottom' | 'left' | 'right'

const bindings = new WeakMap<HTMLElement, DirectiveBinding<unknown>>()
const cleanups = new WeakMap<HTMLElement, () => void>()

let tip: HTMLDivElement | null = null
let activeTrigger: HTMLElement | null = null

const GAP = 8
const VP_PAD = 8

function ensureTip(): HTMLDivElement {
  if (tip) return tip
  const el = document.createElement('div')
  el.style.cssText = [
    'position:fixed',
    'z-index:9999',
    'pointer-events:none',
    'background:#1f2937',
    'color:#fff',
    'font-size:11px',
    'padding:4px 8px',
    'border-radius:4px',
    'box-shadow:0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)',
    'max-width:320px',
    'box-sizing:border-box',
    'opacity:0',
    'transition:opacity 150ms ease',
  ].join(';')
  document.body.appendChild(el)
  tip = el
  return el
}

function getText(binding: DirectiveBinding<unknown>): string {
  const v = binding.value
  if (v == null || v === false) return ''
  return typeof v === 'string' ? v : String(v)
}

function getPlacement(binding: DirectiveBinding<unknown>): Placement {
  if (binding.modifiers.right) return 'right'
  if (binding.modifiers.left) return 'left'
  if (binding.modifiers.bottom) return 'bottom'
  return 'top'
}

function isTruncated(el: HTMLElement): boolean {
  return el.scrollWidth > el.clientWidth || el.scrollHeight > el.clientHeight
}

function hide() {
  if (tip) tip.style.opacity = '0'
  activeTrigger = null
  window.removeEventListener('scroll', onHideGlobal, true)
  window.removeEventListener('resize', onHideGlobal)
}

function onHideGlobal() {
  hide()
}

function show(el: HTMLElement) {
  const binding = bindings.get(el)
  if (!binding) return
  const text = getText(binding)
  if (!text) return
  if (binding.modifiers.auto && !isTruncated(el)) return

  const tipEl = ensureTip()
  // 长文本 / 多行：换行；短标签：单行（对齐 Sidebar nowrap）
  if (text.length > 40 || text.includes('\n')) {
    tipEl.style.whiteSpace = 'pre-wrap'
    tipEl.style.wordBreak = 'break-word'
  } else {
    tipEl.style.whiteSpace = 'nowrap'
    tipEl.style.wordBreak = 'normal'
  }
  tipEl.textContent = text

  // 先隐藏定位，测量尺寸
  tipEl.style.opacity = '0'
  tipEl.style.left = '0px'
  tipEl.style.top = '0px'
  const tipW = tipEl.offsetWidth
  const tipH = tipEl.offsetHeight
  const rect = el.getBoundingClientRect()
  const placement = getPlacement(binding)

  let top: number
  let left: number

  switch (placement) {
    case 'right': {
      left = rect.right + GAP
      top = rect.top + rect.height / 2 - tipH / 2
      if (left + tipW > window.innerWidth - VP_PAD) left = rect.left - tipW - GAP
      break
    }
    case 'left': {
      left = rect.left - tipW - GAP
      top = rect.top + rect.height / 2 - tipH / 2
      if (left < VP_PAD) left = rect.right + GAP
      break
    }
    case 'bottom': {
      top = rect.bottom + GAP
      left = rect.left + rect.width / 2 - tipW / 2
      if (top + tipH > window.innerHeight - VP_PAD) top = rect.top - tipH - GAP
      break
    }
    default: {
      // top：优先上方，空间不足翻到下方
      top = rect.top - tipH - GAP
      left = rect.left + rect.width / 2 - tipW / 2
      if (top < VP_PAD) top = rect.bottom + GAP
      break
    }
  }

  // 视口 clamp
  if (left < VP_PAD) left = VP_PAD
  if (left + tipW > window.innerWidth - VP_PAD) left = Math.max(VP_PAD, window.innerWidth - tipW - VP_PAD)
  if (top < VP_PAD) top = Math.min(rect.bottom + GAP, window.innerHeight - tipH - VP_PAD)
  if (top + tipH > window.innerHeight - VP_PAD) top = Math.max(VP_PAD, window.innerHeight - tipH - VP_PAD)

  tipEl.style.left = `${Math.round(left)}px`
  tipEl.style.top = `${Math.round(top)}px`

  activeTrigger = el
  // 强制 reflow 后淡入
  tipEl.getBoundingClientRect()
  tipEl.style.opacity = '1'

  window.addEventListener('scroll', onHideGlobal, true)
  window.addEventListener('resize', onHideGlobal)
}

export const vTooltip: Directive<HTMLElement, unknown> = {
  mounted(el, binding) {
    bindings.set(el, binding)
    const onEnter = () => show(el)
    const onLeave = () => {
      if (activeTrigger === el) hide()
    }
    el.addEventListener('mouseenter', onEnter)
    el.addEventListener('mouseleave', onLeave)
    cleanups.set(el, () => {
      el.removeEventListener('mouseenter', onEnter)
      el.removeEventListener('mouseleave', onLeave)
      if (activeTrigger === el) hide()
    })
  },
  updated(el, binding) {
    bindings.set(el, binding)
  },
  unmounted(el) {
    cleanups.get(el)?.()
    cleanups.delete(el)
    bindings.delete(el)
  },
}
