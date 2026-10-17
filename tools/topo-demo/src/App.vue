<script setup lang="ts">
import { nextTick, onMounted, reactive, ref, shallowRef, shallowReactive } from 'vue'
import { Graph, Node, Edge } from '@antv/x6'
import {
  createGraph,
  applyGrid,
  applyBackground,
  applyThemeToCells,
  setEdgeDefaults,
  getEdgeDefaults,
  readEdgeShape,
  readLineStyle,
  readEndMarkers,
  readEdgeColor,
  readEdgeWidth,
  readEdgeLabelState,
  applyEdgeLabel,
  contentSize,
  GRID_MODES,
  EDGE_SHAPES,
  MARKER_NAMES,
  type EdgeShape,
  type GridMode,
  type MarkerName
} from './graph/setup'
import { TOPO_REGION, TOPO_TEXT, createRegionMeta, createTextNodeMeta, readNodeStyleMode, readNodeStatus, readNodeLabelStyle } from './graph/node'
import { createSelInfo } from './graph/sel'
import { isEditableTarget, downloadText, timestampName } from './graph/utils'
import { parseTopoYaml, applyTopoYaml, applyLayoutOverride, type YamlError } from './graph/yamlImport'
import { graphToYaml, graphToLayout, graphToCombined } from './graph/yamlExport'
import { syncLegend } from './graph/legend'
import { initThemeFromStorage, setTheme, type ThemeName } from './graph/theme'
import {
  alignSelection,
  distributeSelection,
  nudgeSelection,
  duplicateSelection,
  zOrder,
  groupSelection,
  ungroupSelection,
  zoomToSelection,
  autoLayout,
  attachRegionRefit,
  type AlignMode,
  type LayoutDir
} from './graph/editing'
import StencilPanel from './components/StencilPanel.vue'
import Inspector from './components/Inspector.vue'
import EdgeVertexHandles from './components/EdgeVertexHandles.vue'

const containerRef = ref<HTMLDivElement | null>(null)
const wrapRef = ref<HTMLDivElement | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)
const editorInputRef = ref<HTMLInputElement | null>(null)

const graphRef = shallowRef<Graph | null>(null)
const scale = ref(1)
const canUndo = ref(false)
const canRedo = ref(false)
const sel = shallowReactive(createSelInfo())

const theme = ref<ThemeName>(initThemeFromStorage())
const gridMode = ref<GridMode>((() => {
  try {
    const v = localStorage.getItem('topo.gridMode')
    if (v === 'fine' || v === 'dots' || v === 'none') return v
  } catch {
    /* ignore */
  }
  return 'fine'
})())
const defShape = ref<EdgeShape>(getEdgeDefaults().shape)
const defTail = ref<MarkerName>(getEdgeDefaults().tail)

const editorVisible = ref(false)
const editorValue = ref('')
const editorPos = reactive({ x: 0, y: 0 })
let editorCell: Node | Edge | null = null

/* ---------------- 导出对话框 ---------------- */

const exportOpen = ref(false)
const exportScale = ref(2)
const exportBg = ref<'canvas' | 'white' | 'transparent'>('canvas')
const exportPadding = ref(24)

onMounted(() => {
  if (!containerRef.value) return
  const graph = createGraph(containerRef.value)
  graphRef.value = graph
  ;(window as unknown as Record<string, unknown>).__graph = graph

  graph.on('scale', ({ sx }: { sx: number }) => (scale.value = sx))
  graph.on('history:change', refreshHistory)
  graph.on('history:undo', refreshHistory)
  graph.on('history:redo', refreshHistory)
  graph.on('selection:changed', updateSel)
  graph.on('node:dblclick', ({ node, e }: { node: Node; e: MouseEvent }) => openEditor(node, e))
  graph.on('edge:dblclick', ({ edge, e }: { edge: Edge; e: MouseEvent }) => openEditor(edge, e))

  bindKeys(graph)
  attachRegionRefit(graph)
  updateSel()
})

function refreshHistory() {
  const g = graphRef.value
  if (!g) return
  canUndo.value = g.canUndo()
  canRedo.value = g.canRedo()
}

function updateSel() {
  const g = graphRef.value
  if (!g) return
  const cells = g.getSelectedCells()
  sel.nodes = cells.filter((c): c is Node => c.isNode())
  sel.edges = cells.filter((c): c is Edge => c.isEdge())
  if (sel.nodes.length === 1 && sel.edges.length === 0) {
    const n = sel.nodes[0]
    sel.kind = 'node'
    sel.name = String(n.attr('label/text') ?? '')
    sel.styleMode = readNodeStyleMode(n)
    sel.status = readNodeStatus(n)
    const ls = readNodeLabelStyle(n)
    sel.labelFontSize = ls.fontSize
    sel.labelColor = ls.fill
    sel.labelBold = ls.fontWeight === 'bold' || Number(ls.fontWeight) >= 600
  } else if (sel.edges.length === 1 && sel.nodes.length === 0) {
    const e = sel.edges[0]
    sel.kind = 'edge'
    const st = readEdgeLabelState(e)
    sel.labelText = st.text
    sel.linkType = String((e.getData() as { linkType?: string } | undefined)?.linkType ?? '')
    sel.labelPos = st.position
    sel.labelPilled = st.pilled
    sel.labelFontSizeE = st.fontSize
    sel.labelColorE = st.color
    sel.edgeShape = readEdgeShape(e)
    sel.lineStyle = readLineStyle(e)
    const markers = readEndMarkers(e)
    sel.headMarker = markers.head
    sel.tailMarker = markers.tail
    sel.color = readEdgeColor(e)
    sel.strokeWidth = readEdgeWidth(e)
  } else if (cells.length > 1) {
    sel.kind = 'multi'
  } else {
    sel.kind = 'none'
  }
  syncEdgeTools(g, cells)
}

/* 选中连线 → 挂首尾拖拽手柄（端点重连）；取消 → 摘除 */
function syncEdgeTools(g: Graph, cells: ReturnType<Graph['getSelectedCells']>) {
  const selIds = new Set(cells.filter((c) => c.isEdge()).map((c) => c.id))
  g.getEdges().forEach((e) => {
    const on = selIds.has(e.id)
    const has = !!e.getTools()
    if (on && !has) {
      e.addTools([
        {
          name: 'source-arrowhead',
          args: {
            attrs: { fill: '#1677ff', stroke: '#ffffff', 'stroke-width': 2, cursor: 'grab', d: 'M 13 -10 -13 0 13 10 Z' }
          }
        },
        {
          name: 'target-arrowhead',
          args: {
            attrs: { fill: '#1677ff', stroke: '#ffffff', 'stroke-width': 2, cursor: 'grab', d: 'M -13 -10 13 0 -13 10 Z' }
          }
        }
      ])
    } else if (!on && has) {
      e.removeTools()
    }
  })
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
  bind(['meta+d', 'ctrl+d'], (e) => {
    e.preventDefault()
    duplicateSelection(g)
    updateSel()
  })
  const step = (e: KeyboardEvent) => (e.shiftKey ? 10 : 1)
  const nudge = (dx: number, dy: number) => (e: KeyboardEvent) => {
    e.preventDefault()
    nudgeSelection(g, dx * step(e), dy * step(e))
  }
  bind(['left', 'shift+left'], nudge(-1, 0))
  bind(['right', 'shift+right'], nudge(1, 0))
  bind(['up', 'shift+up'], nudge(0, -1))
  bind(['down', 'shift+down'], nudge(0, 1))
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

/* ---------------- P1 编辑操作 ---------------- */

function withGraph(fn: (g: Graph) => void) {
  const g = graphRef.value
  if (!g) return
  fn(g)
  updateSel()
}

function doAlign(mode: AlignMode) {
  withGraph((g) => alignSelection(g, mode))
}

function doDistribute(axis: 'h' | 'v') {
  withGraph((g) => distributeSelection(g, axis))
}

function doZ(action: 'front' | 'back') {
  withGraph((g) => zOrder(g, action))
}

function doGroup() {
  withGraph((g) => groupSelection(g))
}

function doUngroup() {
  withGraph((g) => ungroupSelection(g))
}

function doZoomSel() {
  withGraph((g) => zoomToSelection(g))
}

const layoutDir = ref<LayoutDir>('TB')

function doAutoLayout() {
  const g = graphRef.value
  if (!g) return
  void autoLayout(g, layoutDir.value).then(() => {
    g.zoomToFit({ padding: 60, maxScale: 1.2 })
    updateSel()
  })
}

/* 手动重建线路图例（编辑连线/改类型后刷新） */
function doSyncLegend() {
  withGraph((g) => {
    syncLegend(g)
    g.zoomToFit({ padding: 60, maxScale: 1.2 })
  })
}

function doDuplicate() {
  withGraph((g) => duplicateSelection(g))
}

/* ---------------- 画布设置 ---------------- */

function onThemeChange(v: ThemeName) {
  const g = graphRef.value
  if (!g) return
  setTheme(v)
  applyBackground(g, v)
  applyGrid(g, gridMode.value, v)
  applyThemeToCells(g, v)
}

function onGridChange(v: GridMode) {
  const g = graphRef.value
  if (!g) return
  gridMode.value = v
  try {
    localStorage.setItem('topo.gridMode', v)
  } catch {
    /* ignore */
  }
  applyGrid(g, v, theme.value)
}

function onDefShape(v: EdgeShape) {
  defShape.value = v
  setEdgeDefaults({ shape: v })
}

function onDefTail(v: MarkerName) {
  defTail.value = v
  setEdgeDefaults({ tail: v })
}

/* ---------------- 节点添加 ---------------- */

function addPresetNode(shape: string, w: number, h: number) {
  const g = graphRef.value
  const el = containerRef.value
  if (!g || !el) return
  const r = el.getBoundingClientRect()
  const center = g.clientToGraph(r.left + r.width / 2, r.top + r.height / 2)
  const meta =
    shape === TOPO_REGION ? createRegionMeta() : shape === TOPO_TEXT ? createTextNodeMeta() : { shape }
  const node = g.addNode({ ...meta, x: center.x - w / 2, y: center.y - h / 2 } as never)
  g.cleanSelection()
  g.select(node)
}

/* ---------------- 标签编辑 ---------------- */

function openEditor(cell: Node | Edge, ev: MouseEvent) {
  const wrap = wrapRef.value
  if (!wrap) return
  editorCell = cell
  editorValue.value = cell.isNode() ? String(cell.attr('label/text') ?? '') : readEdgeLabelState(cell).text
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
      applyEdgeLabel([cell], { text: v })
    }
    updateSel()
  }
  editorVisible.value = false
  editorCell = null
}

/* ---------------- 文件 ---------------- */

function saveJson() {
  const g = graphRef.value
  if (!g) return
  const data = {
    app: 'topo-demo',
    version: 1,
    schemaVersion: 1,
    savedAt: new Date().toISOString(),
    graph: g.toJSON()
  }
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

/* ---------------- 导出 ---------------- */

function bgColorOption(): string | undefined {
  if (exportBg.value === 'white') return '#ffffff'
  if (exportBg.value === 'canvas') return canvasBg()
  return undefined
}

function canvasBg(): string {
  const map: Record<ThemeName, string> = { white: '#ffffff', paper: '#faf9f5', dark: '#1f2733' }
  return map[theme.value]
}

function exportPNGFile() {
  const g = graphRef.value
  if (!g) return
  const pad = exportPadding.value
  const size = contentSize(g, 0)
  g.exportPNG(timestampName('topology', 'png'), {
    backgroundColor: bgColorOption(),
    copyStyles: true,
    padding: pad,
    width: Math.round((size.width + pad * 2) * exportScale.value),
    height: Math.round((size.height + pad * 2) * exportScale.value)
  } as never)
  exportOpen.value = false
}

function exportSVGFile() {
  const g = graphRef.value
  if (!g) return
  g.exportSVG(timestampName('topology', 'svg'), {
    copyStyles: true,
    serializeImages: true,
    preserveDimensions: true,
    backgroundColor: bgColorOption()
  } as never)
  exportOpen.value = false
}

function clearAll() {
  const g = graphRef.value
  if (!g) return
  if (g.getCells().length && !confirm('确定清空画布？（可撤销）')) return
  g.clearCells()
}

/* ---------------- YAML 导入 ---------------- */

const yamlOpen = ref(false)
const yamlText = ref('')
const yamlReport = ref<{
  ok: boolean
  items: YamlError[]
  warnings?: YamlError[]
  summary?: string
} | null>(null)

const YAML_EXAMPLE = `# 设备中心式拓扑 YAML：
# - 每台设备一条记录，唯一 id 是引用键（字母/数字/_/-）
# - links 每条链路只在一端声明一次（重复声明会报错）；dir 必填（上|下|左|右，对端相对本设备的方位）
#   type=链路类型（风格预设，内置类型不可覆盖）；可加 label/style/color/arrow 覆盖
# - 位置不输出——工具按连线方向自动排列；iconStyle(简约|写实) / axis(上下|左右) / blocks 均可选
#   同层设备自动对齐（上下布局水平对齐，左右布局垂直对齐）
devices:
  - id: cloud
    icon: cloud
    name: 互联网
    links:
      - { to: fw, type: 互联, dir: 下 }
  - id: fw
    icon: firewall
    name: 边界防火墙
    links:
      - { to: core, type: 千兆光纤, dir: 下 }
  - id: core
    icon: router
    name: 核心路由器
    block: 核心区
    links:
      - { to: sw1, type: 堆叠, label: VSL-P1/2, dir: 下 }
  - id: sw1
    icon: switch
    name: 汇聚交换机
    block: 核心区
    links:
      - { to: srv, type: 千兆网线, arrow: one, dir: 下 }
  - id: srv
    icon: server
    name: 应用服务器
blocks:
  - { id: 核心区, color: blue }`

function yamlExample() {
  yamlText.value = YAML_EXAMPLE
  yamlReport.value = null
}

function yamlParse() {
  const r = parseTopoYaml(yamlText.value)
  if (r.ok) {
    yamlReport.value = {
      ok: true,
      items: [],
      warnings: r.warnings,
      summary: `解析通过：${r.devices} 设备 / ${r.edges} 链路 / ${r.blocks} 区域 · 主轴${r.axis}（${r.axisSource}）${
        r.needsLayout ? ' · 存在缺省坐标，应用时自动布局' : ' · 坐标精确定位'
      }${r.warnings.length ? ` · ${r.warnings.length} 条警告` : ''}`
    }
  } else {
    yamlReport.value = { ok: false, items: r.errors, warnings: r.warnings }
  }
}

async function yamlApply() {
  const g = graphRef.value
  if (!g) return
  const r = await applyTopoYaml(g, yamlText.value)
  if (r.ok) {
    yamlReport.value = {
      ok: true,
      items: [],
      warnings: r.warnings,
      summary: `已应用：${r.devices} 设备 / ${r.edges} 链路 / ${r.blocks} 区域 · ${r.axis}${
        r.needsLayout ? '，已自动布局' : '，坐标精确定位'
      }（⌘Z 可撤销）${r.warnings.length ? ` · ${r.warnings.length} 条警告` : ''}`
    }
    updateSel()
  } else {
    yamlReport.value = { ok: false, items: r.errors, warnings: r.warnings }
  }
}

/* ---- 导出：语义 YAML（无坐标）+ 布局 sidecar ---- */

function yamlExport() {
  const g = graphRef.value
  if (!g) return
  const { yaml, skipped } = graphToYaml(g)
  yamlText.value = yaml
  yamlReport.value = {
    ok: true,
    items: [],
    warnings: [],
    summary: `已导出语义 YAML 到编辑框${skipped ? `（${skipped} 个元素无法用 YAML 表达，已跳过）` : ''}。可复制或下载。`
  }
}

function yamlExportLayout() {
  const g = graphRef.value
  if (!g) return
  const layout = graphToLayout(g)
  downloadText(`${timestampName('topology', 'layout.yaml')}`, layout, 'text/yaml')
}

function yamlExportCombined() {
  const g = graphRef.value
  if (!g) return
  const { yaml, skipped } = graphToCombined(g)
  yamlText.value = yaml
  yamlReport.value = {
    ok: true,
    items: [],
    warnings: [],
    summary: `已导出合并 YAML（语义+坐标）到编辑框${skipped ? `（${skipped} 个元素无法表达，已跳过）` : ''}。一次「应用」即可完整还原。`
  }
}

function yamlApplyLayout() {
  const g = graphRef.value
  if (!g) return
  const r = applyLayoutOverride(g, yamlText.value)
  yamlReport.value = {
    ok: r.ok,
    items: r.ok ? [] : [{ line: null, message: r.message }],
    warnings: [],
    summary: r.ok ? `${r.message}（⌘Z 可撤销）` : r.message
  }
  if (r.ok) updateSel()
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
      <button title="原位副本 (Cmd+D)" @click="doDuplicate">副本</button>
      <button @click="doDelete">删除</button>
      <i class="sep" />
      <button @click="addPresetNode(TOPO_REGION, 220, 140)">区域</button>
      <button @click="addPresetNode(TOPO_TEXT, 120, 32)">文本</button>
      <button @click="doGroup">编组</button>
      <button @click="doUngroup">解组</button>
      <i class="sep" />
      <button title="对齐：左/中/右/顶/中/底" @click="doAlign('left')">⇤</button>
      <button title="水平居中" @click="doAlign('hcenter')">↔</button>
      <button title="右对齐" @click="doAlign('right')">⇥</button>
      <button title="顶对齐" @click="doAlign('top')">⤒</button>
      <button title="垂直居中" @click="doAlign('vcenter')">↕</button>
      <button title="底对齐" @click="doAlign('bottom')">⤓</button>
      <button title="水平等距" @click="doDistribute('h')">分布H</button>
      <button title="垂直等距" @click="doDistribute('v')">分布V</button>
      <i class="sep" />
      <button title="置于顶层/底层" @click="doZ('front')">↑层</button>
      <button title="置于底层" @click="doZ('back')">↓层</button>
      <i class="sep" />
      <select v-model="layoutDir" title="布局方向">
        <option value="TB">上下</option>
        <option value="LR">左右</option>
      </select>
      <button title="dagre 分层自动布局" @click="doAutoLayout">自动布局</button>
      <button title="重建线路图例" @click="doSyncLegend">图例</button>
      <i class="sep" />
      <label class="tb-label">新连线</label>
      <select :value="defShape" @change="onDefShape(($event.target as HTMLSelectElement).value as EdgeShape)">
        <option v-for="s in EDGE_SHAPES" :key="s.key" :value="s.key">{{ s.label }}</option>
      </select>
      <select :value="defTail" @change="onDefTail(($event.target as HTMLSelectElement).value as MarkerName)">
        <option v-for="m in MARKER_NAMES" :key="m" :value="m">
          {{ { none: '无', block: '方块', classic: '尖箭', diamond: '菱形', circle: '圆点', ellipse: '椭圆', cross: '十字' }[m] }}
        </option>
      </select>
      <i class="sep" />
      <label class="tb-label">画布</label>
      <select :value="theme" @change="onThemeChange(($event.target as HTMLSelectElement).value as ThemeName)">
        <option value="white">白</option>
        <option value="paper">米白</option>
        <option value="dark">深灰</option>
      </select>
      <select :value="gridMode" @change="onGridChange(($event.target as HTMLSelectElement).value as GridMode)">
        <option v-for="g in GRID_MODES" :key="g.key" :value="g.key">网格·{{ g.label }}</option>
      </select>
      <i class="sep" />
      <button @click="zoom(-0.2)">−</button>
      <span class="zoomtext">{{ Math.round(scale * 100) }}%</span>
      <button @click="zoom(0.2)">＋</button>
      <button @click="graphRef?.zoomTo(1)">1:1</button>
      <button @click="graphRef?.zoomToFit({ padding: 40, maxScale: 1 })">适应</button>
      <button title="缩放到选中" @click="doZoomSel">到选区</button>
      <span class="spacer" />
      <button class="primary" @click="saveJson">保存</button>
      <button @click="fileInput?.click()">打开</button>
      <input ref="fileInput" type="file" accept=".json,application/json" hidden @change="openJson" />
      <button title="导入拓扑 YAML" @click="yamlOpen = true">YAML</button>
      <button @click="exportOpen = true">导出…</button>
      <button @click="clearAll">清空</button>
    </header>

    <div class="body">
      <StencilPanel :graph="graphRef" />
      <div class="canvas-wrap" ref="wrapRef">
        <div class="canvas" ref="containerRef" />
        <EdgeVertexHandles :graph="graphRef" :wrap="wrapRef" />
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
        <div class="hint">拖拽左侧图标到画布 · 悬停节点拖出连线 · 左键拖动框选 · 双指滚动/右键拖动平移 · Ctrl+滚轮缩放 · 双击改名</div>
      </div>
      <Inspector :graph="graphRef" :sel="sel" @changed="updateSel" @action="doZ" />
    </div>

    <div v-if="exportOpen" class="modal-mask" @click.self="exportOpen = false">
      <div class="modal">
        <h3>导出</h3>
        <div class="field">
          <label>PNG 缩放（清晰度）</label>
          <div class="btn-group">
            <button v-for="s in [1, 2, 3]" :key="s" :class="{ active: exportScale === s }" @click="exportScale = s">
              {{ s }}x
            </button>
          </div>
        </div>
        <div class="field">
          <label>背景</label>
          <div class="btn-group">
            <button :class="{ active: exportBg === 'canvas' }" @click="exportBg = 'canvas'">跟随画布</button>
            <button :class="{ active: exportBg === 'white' }" @click="exportBg = 'white'">白色</button>
            <button :class="{ active: exportBg === 'transparent' }" @click="exportBg = 'transparent'">透明</button>
          </div>
        </div>
        <div class="field">
          <label>边距：{{ exportPadding }}px</label>
          <input v-model.number="exportPadding" type="range" min="0" max="80" step="4" />
        </div>
        <div class="modal-actions">
          <button @click="exportOpen = false">取消</button>
          <button class="primary" @click="exportPNGFile">导出 PNG</button>
          <button @click="exportSVGFile">导出 SVG</button>
        </div>
      </div>
    </div>

    <div v-if="yamlOpen" class="modal-mask" @click.self="yamlOpen = false">
      <div class="modal yaml-modal">
        <h3>YAML 拓扑 · 导入 / 导出</h3>
        <textarea
          v-model="yamlText"
          class="yaml-input"
          placeholder="粘贴拓扑 YAML（点「示例」看格式）…"
          spellcheck="false"
        />
        <div v-if="yamlReport" class="yaml-report" :class="yamlReport.ok ? 'ok' : 'err'">
          <div v-if="yamlReport.summary" class="yaml-summary">{{ yamlReport.summary }}</div>
          <div v-for="(it, i) in yamlReport.items" :key="'e' + i" class="yaml-item">
            <template v-if="it.line !== null">第 {{ it.line }} 行：</template>{{ it.message }}
          </div>
          <div v-if="yamlReport.warnings && yamlReport.warnings.length" class="yaml-warn-title">警告：</div>
          <div v-for="(w, i) in yamlReport.warnings || []" :key="'w' + i" class="yaml-warn">
            <template v-if="w.line !== null">第 {{ w.line }} 行：</template>{{ w.message }}
          </div>
        </div>
        <div class="modal-actions">
          <button @click="yamlExample">示例</button>
          <button @click="yamlParse">解析</button>
          <button class="primary" @click="yamlApply">应用</button>
          <i class="sep" />
          <button title="导出当前图为语义 YAML（无坐标）到编辑框" @click="yamlExport">导出</button>
          <button title="导出合并 YAML（语义+坐标），一次应用完整还原" @click="yamlExportCombined">导出(含坐标)</button>
          <button title="下载布局坐标 sidecar（yid→坐标）" @click="yamlExportLayout">导出布局</button>
          <button title="把编辑框里的布局 sidecar 套到当前图（按 yid）" @click="yamlApplyLayout">应用布局</button>
          <i class="sep" />
          <button @click="yamlOpen = false">关闭</button>
        </div>
      </div>
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

.tb-label {
  font-size: 12px;
  color: #5a6472;
}

.toolbar select {
  font: inherit;
  border: 1px solid #d5dae2;
  border-radius: 6px;
  padding: 3px 6px;
  background: #fff;
  outline: none;
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

.hint {
  position: absolute;
  left: 12px;
  bottom: 10px;
  font-size: 11px;
  color: #a8b0bd;
  pointer-events: none;
  z-index: 5;
}

.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(15, 23, 42, 0.35);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}

.modal {
  width: 340px;
  background: #fff;
  border-radius: 12px;
  padding: 18px;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.2);
}

.modal h3 {
  font-size: 15px;
  margin-bottom: 14px;
}

.modal .field {
  margin-bottom: 14px;
}

.modal .field label {
  display: block;
  font-size: 12px;
  color: #5a6472;
  margin-bottom: 6px;
}

.modal .btn-group {
  display: flex;
  gap: 6px;
}

.modal .btn-group button {
  flex: 1;
}

.modal button.active {
  border-color: #1677ff;
  color: #1677ff;
  background: #f0f7ff;
}

.modal input[type='range'] {
  width: 100%;
}

.modal-actions {
  display: flex;
  gap: 8px;
  align-items: center;
  justify-content: flex-end;
  margin-top: 18px;
}

.modal-actions .sep {
  width: 1px;
  height: 18px;
  background: #e5e8ee;
  margin: 0 2px;
}

.yaml-modal {
  width: 640px;
}

.yaml-input {
  width: 100%;
  height: 300px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.6;
  border: 1px solid #d5dae2;
  border-radius: 8px;
  padding: 10px;
  resize: vertical;
  outline: none;
  box-sizing: border-box;
}

.yaml-input:focus {
  border-color: #1677ff;
}

.yaml-report {
  margin-top: 10px;
  max-height: 150px;
  overflow-y: auto;
  font-size: 12px;
  line-height: 1.7;
  padding: 8px 10px;
  border-radius: 8px;
}

.yaml-report.ok {
  background: #f0fdf4;
  color: #166534;
}

.yaml-report.err {
  background: #fef2f2;
  color: #b91c1c;
}

.yaml-summary {
  font-weight: 600;
}

.yaml-warn-title {
  margin-top: 6px;
  font-weight: 600;
}

.yaml-warn {
  color: #b45309;
}
</style>
