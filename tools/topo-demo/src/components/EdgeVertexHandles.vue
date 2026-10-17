<script setup lang="ts">
import { onBeforeUnmount, watch } from 'vue'
import type { Graph, Edge } from '@antv/x6'

const props = defineProps<{ graph: Graph | null; wrap: HTMLElement | null }>()

interface Handle {
  kind: 'move' | 'insert'
  index: number
  x: number
  y: number
}

let overlay: HTMLDivElement | null = null
let cleanupFns: Array<() => void> = []
let drag: { kind: 'move' | 'insert'; index: number } | null = null

function buildHandle(h: Handle): HTMLDivElement {
  const div = document.createElement('div')
  div.className = 'vh ' + h.kind
  div.style.left = h.x + 'px'
  div.style.top = h.y + 'px'
  div.title = h.kind === 'move' ? '拖动调整折点 · 双击删除' : '拖动添加折点'
  div.addEventListener('pointerdown', (e) => {
    e.preventDefault()
    e.stopPropagation()
    drag = { kind: h.kind, index: h.index }
  })
  if (h.kind === 'move') {
    div.addEventListener('dblclick', (e) => {
      e.preventDefault()
      e.stopPropagation()
      removeVertex(h.index)
    })
  }
  return div
}

function update() {
  const g = props.graph
  if (!g || !overlay) return
  const cells = g.getSelectedCells()
  if (cells.length !== 1 || !cells[0].isEdge()) {
    overlay.innerHTML = ''
    overlay.style.display = 'none'
    return
  }
  const edge = cells[0] as Edge
  const pts = [edge.getSourcePoint(), ...edge.getVertices(), edge.getTargetPoint()]
  const wrap = overlay.parentElement
  const rect = wrap ? wrap.getBoundingClientRect() : { left: 0, top: 0 }
  const toClient = (p: { x: number; y: number }) => {
    const c = g.localToClient(p.x, p.y)
    return { x: c.x - rect.left, y: c.y - rect.top }
  }
  const frag = document.createDocumentFragment()
  for (let i = 1; i < pts.length - 1; i++) {
    frag.appendChild(buildHandle({ kind: 'move', index: i - 1, ...toClient(pts[i]) }))
  }
  for (let i = 0; i < pts.length - 1; i++) {
    const mid = { x: (pts[i].x + pts[i + 1].x) / 2, y: (pts[i].y + pts[i + 1].y) / 2 }
    frag.appendChild(buildHandle({ kind: 'insert', index: i, ...toClient(mid) }))
  }
  overlay.innerHTML = ''
  overlay.appendChild(frag)
  overlay.style.display = 'block'
}

function onPointerMove(e: PointerEvent) {
  const g = props.graph
  if (!drag || !g) return
  const local = g.clientToLocal(e.clientX, e.clientY)
  const pt = { x: Math.round(local.x), y: Math.round(local.y) }
  const cells = g.getSelectedCells()
  if (cells.length !== 1 || !cells[0].isEdge()) return
  const edge = cells[0] as Edge
  const vertices = edge.getVertices().map((v) => ({ x: v.x, y: v.y }))
  if (drag.kind === 'move') {
    if (drag.index < vertices.length) vertices[drag.index] = pt
  } else {
    vertices.splice(drag.index, 0, pt)
    drag = { kind: 'move', index: drag.index }
  }
  edge.setVertices(vertices)
}

function onPointerUp() {
  drag = null
  update()
}

function removeVertex(index: number) {
  const g = props.graph
  if (!g) return
  const cells = g.getSelectedCells()
  if (cells.length !== 1 || !cells[0].isEdge()) return
  const edge = cells[0] as Edge
  const vertices = edge.getVertices().map((v) => ({ x: v.x, y: v.y }))
  vertices.splice(index, 1)
  edge.setVertices(vertices)
  update()
}

function setup() {
  teardown()
  const g = props.graph
  if (!g) return
  const wrap = props.wrap ?? (g.container?.parentElement as HTMLElement | null)
  if (!wrap) return
  overlay = document.createElement('div')
  overlay.className = 'vh-overlay'
  wrap.appendChild(overlay)
  const refresh = () => update()
  const events = [
    'selection:changed',
    'scale:changed',
    'translate',
    'edge:change:vertices',
    'edge:change:source',
    'edge:change:target',
    'node:change:position'
  ]
  events.forEach((ev) => g.on(ev, refresh))
  cleanupFns = events.map((ev) => () => g.off(ev, refresh))
  cleanupFns.push(() => window.removeEventListener('pointermove', onPointerMove))
  cleanupFns.push(() => window.removeEventListener('pointerup', onPointerUp))
  window.addEventListener('pointermove', onPointerMove)
  window.addEventListener('pointerup', onPointerUp)
}

function teardown() {
  cleanupFns.forEach((f) => f())
  cleanupFns = []
  if (overlay) {
    overlay.remove()
    overlay = null
  }
}

watch(
  () => props.graph,
  () => setup(),
  { immediate: true }
)

onBeforeUnmount(() => teardown())
</script>

<template>
  <div class="vh-host" style="display: none" />
</template>
