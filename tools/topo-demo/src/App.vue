<script setup lang="ts">
import { nextTick, onMounted, reactive, ref, shallowRef, shallowReactive } from 'vue'
import { Graph, Node, Edge } from '@antv/x6'
import { createGraph } from './graph/setup'
import { TOPO_REGION, TOPO_TEXT } from './graph/node'
import { createSelInfo } from './graph/sel'
import { isEditableTarget, downloadText, timestampName } from './graph/utils'
import StencilPanel from './components/StencilPanel.vue'
import Inspector from './components/Inspector.vue'

const containerRef = ref<HTMLDivElement | null>(null)
const minimapRef = ref<HTMLDivElement | null>(null)
const wrapRef = ref<HTMLDivElement | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)
const editorInputRef = ref<HTMLInputElement | null>(null)

const graphRef = shallowRef<Graph | null>(null)
const scale = ref(1)
const canUndo = ref(false)
const canRedo = ref(false)
const sel = shallowReactive(createSelInfo())

const editorVisible = ref(false)
const editorValue = ref('')
const editorPos = reactive({ x: 0, y: 0 })
let editorCell: Node | Edge | null = null

onMounted(() => {
  if (!containerRef.value || !minimapRef.value) return
  const graph = createGraph(containerRef.value, minimapRef.value)
  graphRef.value = graph

  graph.on('scale:changed', ({ sx }: { sx: number }) => (scale.value = sx))
  graph.on('history:change', refreshHistory)
  graph.on('history:undo', refreshHistory)
  graph.on('history:redo', refreshHistory)
  graph.on('selection:changed', updateSel)
  graph.on('node:dblclick', ({ node, e }: { node: Node; e: MouseEvent }) => openEditor(node, e))
  graph.on('edge:dblclick', ({ edge, e }: { edge: Edge; e: MouseEvent }) => openEditor(edge, e))

  bindKeys(graph)
  updateSel()
})

function refreshHistory() {
  const g = graphRef.value
  if (!g) return
  canUndo.value = g.canUndo()
  canRedo.value = g.canRedo()
}

interface EdgeLabel {
  attrs?: { label?: { text?: string } }
}

function edgeLabelText(cell: Edge): string {
  const labels = cell.getLabels() as EdgeLabel[]
  const first = labels && labels.length ? labels[0] : undefined
  return first && first.attrs && first.attrs.label ? first.attrs.label.text ?? '' : ''
}

function updateSel() {
  const g = graphRef.value
  if (!g) return
  const cells = g.getSelectedCells()
  sel.nodes = cells.filter((c): c is Node => c.isNode())
  sel.edges = cells.filter((c): c is Edge => c.isEdge())
  if (sel.nodes.length === 1 && sel.edges.length === 0) {
    sel.kind = 'node'
    sel.name = String(sel.nodes[0].attr('label/text') ?? '')
  } else if (sel.edges.length === 1 && sel.nodes.length === 0) {
    sel.kind = 'edge'
    const e = sel.edges[0]
    sel.labelText = edgeLabelText(e)
    sel.lineStyle = e.attr('line/strokeDasharray') === '1 7' ? 'dotted' : e.attr('line/strokeDasharray') ? 'dashed' : 'solid'
    sel.marker = e.attr('line/sourceMarker') ? 'both' : e.attr('line/targetMarker') ? 'target' : 'none'
    sel.color = String(e.attr('line/stroke') ?? '#4b5563')
  } else if (cells.length > 1) {
    sel.kind = 'multi'
  } else {
    sel.kind = 'none'
  }
}

function bindKeys(g: Graph) {
  const bind = (keys: string[], cb: (e: KeyboardEvent) => void) =>
    g.bindKey(keys, (e) => {
      if (isEditableTarget(e)) return
      cb(e)
    })

  bind(['meta+z', 'ctrl+z'], () => g.canUndo() && g.undo())
  bind(['meta+shift+z', 'ctrl+shift+z'], () => g.canRedo() && g.redo())
  bind(['meta+c', 'ctrl+c'], () => {
    const cells = g.getSelectedCells()
    if (cells.length) g.copy(cells)
  })
  bind(['meta+x', 'ctrl+x'], () => {
    const cells = g.getSelectedCells()
    if (cells.length) g.cut(cells)
  })
  bind(['meta+v', 'ctrl+v'], () => g.paste({ offset: 32 }))
  bind(['meta+a', 'ctrl+a'], (e) => {
    e.preventDefault()
    g.select(g.getCells())
  })
  bind(['delete', 'backspace'], () => {
    const cells = g.getSelectedCells()
    if (cells.length) g.removeCells(cells)
  })
  bind(['esc'], () => g.cleanSelection())
}

function doCopy() {
  const g = graphRef.value
  if (!g) return
  const cells = g.getSelectedCells()
  if (cells.length) g.copy(cells)
}

function doCut() {
  const g = graphRef.value
  if (!g) return
  const cells = g.getSelectedCells()
  if (cells.length) g.cut(cells)
}

function doPaste() {
  graphRef.value?.paste({ offset: 32 })
}

function doDelete() {
  const g = graphRef.value
  if (!g) return
  const cells = g.getSelectedCells()
  if (cells.length) g.removeCells(cells)
}

function zoom(delta: number) {
  graphRef.value?.zoom(delta)
}

function addPresetNode(shape: string, w: number, h: number) {
  const g = graphRef.value
  const el = containerRef.value
  if (!g || !el) return
  const r = el.getBoundingClientRect()
  const center = g.clientToGraph(r.left + r.width / 2, r.top + r.height / 2)
  const node = g.addNode({ shape, x: center.x - w / 2, y: center.y - h / 2 })
  g.cleanSelection()
  g.select(node)
}

function openEditor(cell: Node | Edge, ev: MouseEvent) {
  const wrap = wrapRef.value
  if (!wrap) return
  editorCell = cell
  editorValue.value = cell.isNode() ? String(cell.attr('label/text') ?? '') : edgeLabelText(cell)
  const r = wrap.getBoundingClientRect()
  editorPos.x = ev.clientX - r.left
  editorPos.y = ev.clientY - r.top
  editorVisible.value = true
  nextTick(() => editorInputRef.value?.focus())
}

function commitEditor() {
  const cell = editorCell
  if (cell) {
    const v = editorValue.value
    if (cell.isNode()) {
      cell.setAttrByPath('label/text', v)
      cell.setData({ ...(cell.getData() || {}), label: v })
    } else {
      if (v) {
        cell.setLabels([{ position: 0.5, attrs: { label: { text: v, fontSize: 12, fill: '#4b5563' } } }])
      } else {
        cell.setLabels([])
      }
    }
    updateSel()
  }
  editorVisible.value = false
  editorCell = null
}

function saveJson() {
  const g = graphRef.value
  if (!g) return
  const data = { app: 'topo-demo', schemaVersion: 1, savedAt: new Date().toISOString(), graph: g.toJSON() }
  downloadText(timestampName('topology', 'json'), JSON.stringify(data, null, 2))
}

function openJson(e: Event) {
  const g = graphRef.value
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!g || !file) return
  const reader = new FileReader()
  reader.onload = () => {
    try {
      const obj = JSON.parse(String(reader.result))
      g.cleanSelection()
      g.fromJSON(obj.graph ?? obj)
    } catch {
      alert('文件格式无法识别，请选择本工具导出的 .json 文件')
    }
  }
  reader.readAsText(file)
  input.value = ''
}

function exportPNG() {
  graphRef.value?.exportPNG(timestampName('topology', 'png'), {
    backgroundColor: '#ffffff',
    copyStyles: true,
    padding: 20
  })
}

function exportSVG() {
  graphRef.value?.exportSVG(timestampName('topology', 'svg'), {
    copyStyles: true,
    serializeImages: true,
    preserveDimensions: true
  })
}

function clearAll() {
  const g = graphRef.value
  if (!g) return
  if (g.getCells().length && !confirm('确定清空画布？（可撤销）')) return
  g.clearCells()
}
</script>

<template>
  <div class="shell">
    <header class="toolbar">
      <span class="brand"><b>拓扑编辑器</b><em>Demo</em></span>
      <i class="sep" />
      <button :disabled="!canUndo" @click="graphRef?.undo()">撤销</button>
      <button :disabled="!canRedo" @click="graphRef?.redo()">重做</button>
      <i class="sep" />
      <button @click="doCopy">复制</button>
      <button @click="doCut">剪切</button>
      <button @click="doPaste">粘贴</button>
      <button @click="doDelete">删除</button>
      <i class="sep" />
      <button @click="addPresetNode(TOPO_REGION, 200, 120)">区域</button>
      <button @click="addPresetNode(TOPO_TEXT, 120, 32)">文本</button>
      <i class="sep" />
      <button @click="zoom(-0.2)">−</button>
      <span class="zoomtext">{{ Math.round(scale * 100) }}%</span>
      <button @click="zoom(0.2)">＋</button>
      <button @click="graphRef?.zoomTo(1)">1:1</button>
      <button @click="graphRef?.zoomToFit({ padding: 40, maxScale: 1 })">适应</button>
      <span class="spacer" />
      <button class="primary" @click="saveJson">保存</button>
      <button @click="fileInput?.click()">打开</button>
      <input ref="fileInput" type="file" accept=".json,application/json" hidden @change="openJson" />
      <button @click="exportPNG">导出PNG</button>
      <button @click="exportSVG">导出SVG</button>
      <button @click="clearAll">清空</button>
    </header>

    <div class="body">
      <StencilPanel :graph="graphRef" />
      <div class="canvas-wrap" ref="wrapRef">
        <div class="canvas" ref="containerRef" />
        <div class="minimap" ref="minimapRef" />
        <input
          v-if="editorVisible"
          ref="editorInputRef"
          v-model="editorValue"
          class="label-editor"
          :style="{ left: editorPos.x + 'px', top: editorPos.y + 'px' }"
          @keyup.enter="commitEditor"
          @keyup.esc="editorVisible = false"
          @blur="commitEditor"
        />
        <div class="hint">拖拽左侧图标到画布 · 悬停节点显示连接点，按住拖出连线 · Space/右键拖动平移 · 滚轮缩放 · 双击改名</div>
      </div>
      <Inspector :graph="graphRef" :sel="sel" @changed="updateSel" />
    </div>
  </div>
</template>

<style scoped>
.shell {
  height: 100%;
  display: flex;
  flex-direction: column;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  background: #fff;
  border-bottom: 1px solid #e5e8ee;
  flex-wrap: wrap;
}

.brand {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin-right: 6px;
}

.brand b {
  font-size: 14px;
}

.brand em {
  font-style: normal;
  font-size: 11px;
  color: #a8b0bd;
}

.sep {
  width: 1px;
  height: 18px;
  background: #e5e8ee;
  margin: 0 6px;
}

.spacer {
  flex: 1;
}

.zoomtext {
  min-width: 44px;
  text-align: center;
  color: #5a6472;
}

.body {
  flex: 1;
  display: flex;
  min-height: 0;
}

.canvas-wrap {
  flex: 1;
  position: relative;
  min-width: 0;
}

.canvas {
  width: 100%;
  height: 100%;
}

.minimap {
  position: absolute;
  right: 12px;
  bottom: 12px;
  background: #fff;
  border: 1px solid #d5dae2;
  border-radius: 8px;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.08);
  overflow: hidden;
  z-index: 10;
}

.hint {
  position: absolute;
  left: 12px;
  bottom: 10px;
  font-size: 11px;
  color: #a8b0bd;
  pointer-events: none;
  z-index: 5;
}
</style>
