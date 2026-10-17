import { Graph, Shape, Edge, Node, Registry } from '@antv/x6'
import { Selection } from '@antv/x6-plugin-selection'
import { Snapline } from '@antv/x6-plugin-snapline'
import { History } from '@antv/x6-plugin-history'
import { Keyboard } from '@antv/x6-plugin-keyboard'
import { Clipboard } from '@antv/x6-plugin-clipboard'
import { Transform } from '@antv/x6-plugin-transform'
import { Export } from '@antv/x6-plugin-export'
import { TOPO_NODE, TOPO_REGION } from './node'
import { resolveColor, type LinkTypePreset } from './linkTypes'
import { getTheme, themeTokens, THEMES as THEMES_LOOKUP, type ThemeName } from './theme'

/* ---------------- 颜色 ---------------- */

export const EDGE_COLORS = [
  '#4b5563',
  '#1677ff',
  '#0891b2',
  '#389e0d',
  '#7cb305',
  '#ca8a04',
  '#ea580c',
  '#d4380d',
  '#db2777',
  '#722ed1',
  '#64748b',
  '#111827'
]

let recentColors: string[] = []
try {
  recentColors = JSON.parse(localStorage.getItem('topo.recentColors') || '[]')
} catch {
  recentColors = []
}

export function getRecentColors(): string[] {
  return recentColors.slice(0, 5)
}

export function pushRecentColor(color: string) {
  recentColors = [color, ...recentColors.filter((c) => c !== color)].slice(0, 5)
  try {
    localStorage.setItem('topo.recentColors', JSON.stringify(recentColors))
  } catch {
    /* ignore */
  }
}

/* ---------------- 箭头 ---------------- */

export type MarkerName = 'none' | 'block' | 'classic' | 'diamond' | 'cross'

export const MARKER_NAMES: MarkerName[] = ['none', 'block', 'classic', 'diamond', 'cross']

const MARKER_DEFS: Record<Exclude<MarkerName, 'none'>, { width: number; height: number }> = {
  block: { width: 11, height: 9 },
  classic: { width: 11, height: 9 },
  diamond: { width: 10, height: 10 },
  cross: { width: 9, height: 9 }
}

/* 'none' 返回 false：null 会被构造期 merge 丢弃而回退到 X6 形状默认箭头(classic)，
   false 能存进 attrs 且 qualifier 判否 → 不渲染箭头 */
export function markerAttr(
  name: MarkerName | undefined | null
): { name: string; width: number; height: number } | false {
  if (!name || name === 'none') return false
  const def = MARKER_DEFS[name as Exclude<MarkerName, 'none'>]
  return { name, width: def.width, height: def.height }
}

/* ---------------- 连线形状 ---------------- */

export type EdgeShape = 'rounded' | 'smooth' | 'straight' | 'jump'

export const EDGE_SHAPES: { key: EdgeShape; label: string }[] = [
  { key: 'rounded', label: '折线' },
  { key: 'straight', label: '直线' }
]

const SHAPE_DEFS: Record<EdgeShape, { router?: any; connector: any }> = {
  rounded: {
    router: { name: 'manhattan', args: { padding: 16 } },
    connector: { name: 'rounded', args: { radius: 8 } }
  },
  smooth: {
    connector: { name: 'smooth' }
  },
  straight: {
    connector: { name: 'normal' }
  },
  jump: {
    router: { name: 'manhattan', args: { padding: 16 } },
    connector: { name: 'jumpover', args: { size: 8 } }
  }
}

/* ---------------- 连线默认样式状态 ---------------- */

export type LineStyle = 'solid' | 'dashed' | 'dotted'

export interface EdgeStyleDefaults {
  shape: EdgeShape
  lineStyle: LineStyle
  head: MarkerName
  tail: MarkerName
  color: string
  strokeWidth: number
}

let edgeDefaults: EdgeStyleDefaults | null = null

export function getEdgeDefaults(): EdgeStyleDefaults {
  if (!edgeDefaults) {
    edgeDefaults = {
      shape: 'straight',
      lineStyle: 'solid',
      head: 'none',
      tail: 'none',
      color: themeTokens().edgeDefault,
      strokeWidth: 2
    }
  }
  return edgeDefaults
}

export function setEdgeDefaults(patch: Partial<EdgeStyleDefaults>) {
  edgeDefaults = { ...getEdgeDefaults(), ...patch }
}

/* ---------------- 创建连线（新边走默认样式） ---------------- */

function baseLineAttrs(color: string, strokeWidth: number, lineStyle: LineStyle, tail: MarkerName) {
  const dash = lineStyle === 'dashed' ? '7 5' : lineStyle === 'dotted' ? '1 7' : ''
  const cap = lineStyle === 'dotted' ? 'round' : 'butt'
  return {
    line: {
      stroke: color,
      strokeWidth,
      strokeLinecap: cap,
      strokeDasharray: dash,
      targetMarker: markerAttr(tail)
    }
  }
}

/* ---------------- 端点连接点 ----------------
   端口（四端点圆点）：线终止于端口圆心——连线直接渲染到端点上；
   节点主体/区域：沿用 boundary 内探贴合图标（写实 flush 校准不变）。 */
Graph.registerConnectionPoint(
  'edgeEnd',
  function (
    this: unknown,
    line: { end: unknown },
    view: unknown,
    magnet: Element | null,
    options: Record<string, unknown>,
    type: unknown
  ) {
    if (magnet && typeof magnet.closest === 'function' && magnet.closest('.x6-port-body')) {
      return line.end
    }
    return Registry.ConnectionPoint.presets.boundary.call(this as never, line as never, view as never, magnet as never, {
      ...options,
      offset: -6
    }, type as never)
  } as never,
  true
)

/* ---------------- 端点绑定（声明驱动，无几何） ----------------
   YAML 边：dir 声明端口（下→[bottom,top] 等），真相源；
   手动边：预览期吸附最近端口（手势），落点定型，用户选的端口一律保留。 */

const DIR_PORTS: Record<string, [string, string]> = {
  下: ['bottom', 'top'],
  上: ['top', 'bottom'],
  右: ['right', 'left'],
  左: ['left', 'right']
}

const PREVIEW_SNAP_RADIUS = 50

/** 四端点中距 point 最近者 */
function nearestPortSide(node: Node, point: { x: number; y: number }): string {
  const b = node.getBBox()
  const sides: [string, number][] = [
    ['top', Math.hypot(point.x - (b.x + b.width / 2), point.y - b.y)],
    ['bottom', Math.hypot(point.x - (b.x + b.width / 2), point.y - (b.y + b.height))],
    ['left', Math.hypot(point.x - b.x, point.y - (b.y + b.height / 2))],
    ['right', Math.hypot(point.x - (b.x + b.width), point.y - (b.y + b.height / 2))]
  ]
  sides.sort((a, c) => a[1] - c[1])
  return sides[0][0]
}

/** YAML 声明驱动绑定：dir → 固定端口；无 dir（手动边/legacy）→ 原样保留不动 */
export function bindEdgePorts(edge: Edge): boolean {
  const src = edge.getSourceCell()
  const tgt = edge.getTargetCell()
  if (!src || !tgt || !src.isNode() || !tgt.isNode()) return false
  if (src.id === tgt.id) return false
  if (src.shape !== TOPO_NODE || tgt.shape !== TOPO_NODE) return false
  const pair = DIR_PORTS[String((edge.getData() as { dir?: string } | undefined)?.dir ?? '')]
  if (!pair) return false
  edge.setSource({ cell: src.id, port: pair[0] })
  edge.setTarget({ cell: tgt.id, port: pair[1] })
  return true
}

/** 全图设备边按 dir 绑定（YAML 导入 / 自动布局收尾调用，须在批次内） */
export function bindAllEdgePorts(graph: Graph): void {
  graph.getEdges().forEach((e) => bindEdgePorts(e))
}

/** 预览吸附：拖拽中终端是 {x,y} 点时，吸附50px内最近的 topo-node 端口。
    source/target 对称；排除另一端所在节点（避免自环）。 */
function previewSnapTerminal(graph: Graph, edge: Edge, type: 'source' | 'target'): void {
  const t = (type === 'source' ? edge.getSource() : edge.getTarget()) as {
    cell?: string
    port?: string
    x?: number
    y?: number
  }
  if (!t || t.cell != null || t.x == null || t.y == null) return
  const point = { x: t.x, y: t.y }
  const other = (type === 'source' ? edge.getTargetCell() : edge.getSourceCell())?.id
  let best: { node: Node; dist: number } | null = null
  for (const n of graph.getNodes()) {
    if (n.shape !== TOPO_NODE || n.id === other) continue
    const b = n.getBBox()
    const cx = Math.max(b.x, Math.min(point.x, b.x + b.width))
    const cy = Math.max(b.y, Math.min(point.y, b.y + b.height))
    const dist = Math.hypot(point.x - cx, point.y - cy)
    if (dist <= PREVIEW_SNAP_RADIUS && (!best || dist < best.dist)) best = { node: n, dist }
  }
  if (!best) return
  const term = { cell: best.node.id, port: nearestPortSide(best.node, point) }
  if (type === 'source') edge.setSource(term, { ui: true })
  else edge.setTarget(term, { ui: true })
}

/** 落点定型：终端是 {cell} 无端口（落在 body 上）→ 绑释放点最近端口。
    已是端口终端（用户显式选择）→ 保留不动；区域/文本端点不动。 */
export function finalizeEdgePorts(graph: Graph, edge: Edge, clientPoint?: { x: number; y: number }): void {
  for (const type of ['source', 'target'] as const) {
    const t = (type === 'source' ? edge.getSource() : edge.getTarget()) as {
      cell?: string
      port?: string
    }
    if (!t?.cell || t.port) continue
    const node = graph.getCellById(t.cell)
    if (!node || !node.isNode() || node.shape !== TOPO_NODE) continue
    let local: { x: number; y: number }
    if (clientPoint) {
      local = graph.clientToLocal(clientPoint.x, clientPoint.y)
    } else {
      const b = (node as Node).getBBox()
      local = { x: b.x + b.width / 2, y: b.y + b.height / 2 }
    }
    const port = nearestPortSide(node as Node, local)
    if (type === 'source') edge.setSource({ cell: t.cell, port })
    else edge.setTarget({ cell: t.cell, port })
  }
}

/* ---------------- 堆叠椭圆 label（双线中点） ---------------- */

const OVAL_LABEL_MARKUP = [{ tagName: 'ellipse', selector: 'oval' }]

function isOvalLabel(label: unknown): boolean {
  return !!(label as { attrs?: { oval?: unknown } })?.attrs?.oval
}

function ovalLabel(color: string) {
  return {
    markup: OVAL_LABEL_MARKUP,
    position: { distance: 0.5 },
    attrs: {
      oval: {
        rx: 7.5,
        ry: 11.5,
        fill: 'none',
        stroke: color,
        strokeWidth: 1.6,
        strokeDasharray: '4 3',
        eventProbability: 0,
        style: { pointerEvents: 'none' }
      }
    }
  }
}

function labelDistance(label: { position?: unknown }): number {
  const p = label.position
  return typeof p === 'number' ? p : ((p as { distance?: number })?.distance ?? 0.5)
}

/** 同步堆叠椭圆：want=有则补/换色并上移文字，无则移除椭圆并复位文字位置 */
function syncOvalLabel(edge: Edge, want: boolean, color?: string): void {
  const labels = (edge.getLabels() as Record<string, unknown>[]) ?? []
  const ovals = labels.filter(isOvalLabel)
  const texts = labels.filter((l) => !isOvalLabel(l))
  if (want) {
    const hex = color || (ovals[0] as { attrs: { oval: { stroke: string } } })?.attrs?.oval?.stroke || '#722ed1'
    const lifted = texts.map((l) => ({
      ...l,
      position: { distance: labelDistance(l), offset: { x: 0, y: -14 } }
    }))
    edge.setLabels([ovalLabel(hex), ...lifted] as never)
  } else if (ovals.length) {
    const flat = texts.map((l) => ({ ...l, position: labelDistance(l) }))
    edge.setLabels(flat as never)
  }
}

function setOvalColor(edge: Edge, hex: string): void {
  const labels = (edge.getLabels() as Record<string, unknown>[]) ?? []
  if (!labels.some(isOvalLabel)) return
  edge.setLabels(
    labels.map((l) =>
      isOvalLabel(l)
        ? { ...l, attrs: { ...(l.attrs as object), oval: { ...(l.attrs as { oval: object }).oval, stroke: hex } } }
        : l
    ) as never
  )
}

/* 双线（级联）markup：wrap 命中 + line 外轨 + core 内芯，三者同挂 lines 组共享连接几何 */
const DBL_MARKUP = [
  { tagName: 'path', selector: 'wrap', groupSelector: 'lines', attrs: { fill: 'none', cursor: 'pointer', stroke: 'transparent', strokeLinecap: 'round' } },
  { tagName: 'path', selector: 'line', groupSelector: 'lines', attrs: { fill: 'none', pointerEvents: 'none' } },
  { tagName: 'path', selector: 'core', groupSelector: 'lines', attrs: { fill: 'none', pointerEvents: 'none', strokeLinecap: 'butt' } }
]

/** 双线边：外轨染链路色，内芯用画布底色割出双轨；构造后 setAttrs（绕开形状默认箭头） */
export function createDoubleEdge(color: string, outerWidth = 4): Edge {
  const d = getEdgeDefaults()
  const shapeDef = SHAPE_DEFS[d.shape]
  const edge = new Shape.Edge({
    markup: DBL_MARKUP,
    router: shapeDef.router,
    connector: shapeDef.connector,
    zIndex: -1
  })
  edge.setAttrs({
    line: {
      stroke: color,
      strokeWidth: outerWidth,
      strokeLinecap: 'butt',
      strokeDasharray: '',
      targetMarker: false
    },
    core: {
      stroke: themeTokens().bg,
      strokeWidth: 1.6
    }
  })
  return edge
}

export function isDoubleEdge(edge: Edge): boolean {
  return edge.attr('core/stroke') != null
}

export function createEdgeCell(): Edge {
  const d = getEdgeDefaults()
  const shapeDef = SHAPE_DEFS[d.shape]
  const edge = new Shape.Edge({
    router: shapeDef.router,
    connector: shapeDef.connector,
    zIndex: -1
  })
  // 注意：不要走构造器 attrs 合并——X6 构造 merge 会丢弃 false 等“无箭头”显式值而回退形状默认(经典箭头)；
  // 构造后 setAttrs 可靠写入（实测）。
  edge.setAttrs({ line: baseLineAttrs(d.color, d.strokeWidth, d.lineStyle, d.tail).line })
  return edge
}

/* ---------------- 属性应用 / 读取 ---------------- */

export function applyEdgeShape(edges: Edge[], shape: EdgeShape) {
  const def = SHAPE_DEFS[shape]
  edges.forEach((edge) => {
    if (def.router) edge.setRouter(def.router)
    else edge.setRouter(undefined as never)
    edge.setConnector(def.connector)
  })
}

export function readEdgeShape(edge: Edge): EdgeShape {
  const c = edge.getConnector()
  const name = typeof c === 'string' ? c : (c as { name?: string })?.name
  if (name === 'smooth') return 'smooth'
  if (name === 'normal') return 'straight'
  if (name === 'jumpover') return 'jump'
  return 'rounded'
}

export function applyLineStyle(edges: Edge[], style: LineStyle) {
  edges.forEach((edge) => {
    const dash = style === 'dashed' ? '7 5' : style === 'dotted' ? '1 7' : ''
    const cap = style === 'dotted' ? 'round' : 'butt'
    edge.attr('line/strokeDasharray', dash)
    edge.attr('line/strokeLinecap', cap)
  })
}

export function readLineStyle(edge: Edge): LineStyle {
  const dash = edge.attr('line/strokeDasharray') as string
  if (dash === '1 7') return 'dotted'
  if (dash) return 'dashed'
  return 'solid'
}

function readMarkerName(value: unknown): MarkerName {
  if (!value) return 'none'
  if (typeof value === 'string') return value as MarkerName
  const name = (value as { name?: string }).name
  return (name as MarkerName) || 'none'
}

export function applyEndMarkers(edges: Edge[], head: MarkerName, tail: MarkerName) {
  edges.forEach((edge) => {
    edge.attr('line/sourceMarker', markerAttr(head))
    edge.attr('line/targetMarker', markerAttr(tail))
  })
}

export function readEndMarkers(edge: Edge): { head: MarkerName; tail: MarkerName } {
  return {
    head: readMarkerName(edge.attr('line/sourceMarker')),
    tail: readMarkerName(edge.attr('line/targetMarker'))
  }
}

export function applyEdgeColor(edges: Edge[], color: string) {
  edges.forEach((edge) => {
    edge.attr('line/stroke', color)
    setOvalColor(edge, color)
  })
}

export function applyEdgeWidth(edges: Edge[], width: number) {
  edges.forEach((edge) => edge.attr('line/strokeWidth', width))
}

export function readEdgeColor(edge: Edge): string {
  return String(edge.attr('line/stroke') ?? themeTokens().edgeDefault)
}

export function readEdgeWidth(edge: Edge): number {
  const w = edge.attr('line/strokeWidth')
  return typeof w === 'number' ? w : 2
}

/* ---------------- 边标签（药丸底 + 位置） ---------------- */

export interface EdgeLabelState {
  text: string
  position: number
  pilled: boolean
  fontSize: number
  color: string
}

export function pillBody() {
  const t = themeTokens()
  return {
    fill: t.pillBg,
    stroke: t.pillStroke,
    strokeWidth: 1,
    rx: 6,
    ry: 6
  }
}

function estimateLabelSize(text: string, fontSize: number) {
  let w = 0
  for (const ch of text) {
    w += ch.charCodeAt(0) > 255 ? fontSize : fontSize * 0.58
  }
  return { width: Math.round(w + 18), height: Math.round(fontSize + 12) }
}

export function makeEdgeLabelAttrs(state: EdgeLabelState) {
  const size = estimateLabelSize(state.text, state.fontSize)
  return {
    position: state.position,
    size,
    attrs: {
      body: state.pilled
        ? { ...pillBody(), width: size.width, height: size.height }
        : { fill: 'transparent', stroke: 'transparent' },
      label: {
        text: state.text,
        fontSize: state.fontSize,
        fill: state.color,
        textVerticalAnchor: 'middle'
      }
    }
  }
}

export function applyEdgeLabel(edges: Edge[], patch: Partial<EdgeLabelState>) {
  edges.forEach((edge) => {
    const labels = (edge.getLabels() as Record<string, unknown>[]) ?? []
    const ovals = labels.filter(isOvalLabel)
    const cur = readEdgeLabelState(edge)
    const next = { ...cur, ...patch }
    if (!next.text) {
      edge.setLabels(ovals as never)
      return
    }
    const textLabel = makeEdgeLabelAttrs(next)
    if (ovals.length) {
      ;(textLabel as { position: unknown }).position = {
        distance: next.position,
        offset: { x: 0, y: -14 }
      }
    }
    edge.setLabels([...ovals, textLabel] as never)
  })
}

export function readEdgeLabelState(edge: Edge): EdgeLabelState {
  const labels = edge.getLabels() as Record<string, unknown>[]
  const first = (labels ?? []).find((l) => !isOvalLabel(l)) ?? null
  if (!first) {
    return {
      text: '',
      position: 0.5,
      pilled: true,
      fontSize: 12,
      color: themeTokens().edgeLabelFg
    }
  }
  const labelAttrs = (first.attrs as { label?: Record<string, unknown> })?.label || {}
  const pos = first.position
  return {
    text: (labelAttrs.text as string) ?? '',
    position: typeof pos === 'number' ? pos : ((pos as { distance?: number })?.distance ?? 0.5),
    pilled:
      !!(first.attrs as Record<string, unknown>)?.body &&
      ((first.attrs as { body: { fill?: string } }).body.fill !== 'transparent'),
    fontSize: typeof labelAttrs.fontSize === 'number' ? labelAttrs.fontSize : 12,
    color: (labelAttrs.color as string) || (labelAttrs.fill as string) || themeTokens().edgeLabelFg
  }
}

/* ---------- 链路类型应用（Inspector 手动切换 / 解析器共用） ---------- */

function presetLineAttrs(preset: LinkTypePreset, isDbl: boolean) {
  const hex = resolveColor(preset.color) || '#4b5563'
  const dash = preset.style === 'dashed' ? '7 5' : preset.style === 'dotted' ? '1 7' : ''
  const cap = preset.style === 'dotted' ? 'round' : 'butt'
  return isDbl
    ? {
        line: {
          stroke: hex,
          strokeWidth: preset.width,
          strokeDasharray: dash,
          strokeLinecap: cap,
          targetMarker: false,
          sourceMarker: false
        },
        core: { stroke: themeTokens().bg, strokeWidth: 1.6 }
      }
    : {
        line: {
          stroke: hex,
          strokeWidth: preset.width,
          strokeDasharray: dash,
          strokeLinecap: cap,
          targetMarker: false,
          sourceMarker: false
        }
      }
}

/** 把预设外观落到边上（按单双线自适应 selectors + 堆叠椭圆同步） */
export function applyPresetBase(edge: Edge, preset: LinkTypePreset): void {
  edge.setAttrs(presetLineAttrs(preset, isDoubleEdge(edge)) as never)
  syncOvalLabel(edge, !!preset.oval, resolveColor(preset.color) || undefined)
}

/** 对既有边应用预设（双线↔单线形态变化时重建边，一次批次可撤销） */
export function applyLinkType(graph: Graph, edges: Edge[], name: string, preset: LinkTypePreset | null): void {
  if (!preset) return
  edges.forEach((edge) => {
    const needDouble = !!preset.double
    const isDbl = isDoubleEdge(edge)
    if (needDouble !== isDbl) {
      graph.startBatch('change-link-type')
      try {
        const src = edge.getSource()
        const tgt = edge.getTarget()
        const labels = edge.getLabels()
        const data = { ...(edge.getData() || {}), linkType: name }
        edge.remove()
        const next = needDouble
          ? createDoubleEdge(resolveColor(preset.color) || '#4b5563', preset.width)
          : createEdgeCell()
        next.setSource(src)
        next.setTarget(tgt)
        next.setLabels(labels)
        next.setData(data)
        next.setAttrs(presetLineAttrs(preset, needDouble) as never)
        syncOvalLabel(next, !!preset.oval, resolveColor(preset.color) || undefined)
        graph.addEdge(next)
        graph.cleanSelection()
        graph.select(next)
      } finally {
        graph.stopBatch('change-link-type')
      }
    } else {
      edge.setAttrs(presetLineAttrs(preset, isDbl) as never)
      edge.setData({ ...(edge.getData() || {}), linkType: name })
      syncOvalLabel(edge, !!preset.oval, resolveColor(preset.color) || undefined)
    }
  })
}

export function edgeLabelText(edge: Edge): string {
  return readEdgeLabelState(edge).text
}

/* ---------------- 网格 / 背景 / 主题 ---------------- */

export type GridMode = 'fine' | 'dots' | 'none'

export const GRID_MODES: { key: GridMode; label: string }[] = [
  { key: 'fine', label: '细线' },
  { key: 'dots', label: '点阵' },
  { key: 'none', label: '无' }
]

export function applyGrid(graph: Graph, mode: GridMode, theme: ThemeName) {
  const t = THEMES_LOOKUP[theme]
  if (mode === 'none') {
    graph.hideGrid()
    return
  }
  graph.showGrid()
  if (mode === 'fine') {
    graph.grid.draw({
      size: 10,
      visible: true,
      type: 'doubleMesh',
      args: [
        { color: t.gridFine[0], thickness: 1 },
        { color: t.gridFine[1], thickness: 1, factor: 5 }
      ]
    } as never)
  } else {
    graph.grid.draw({
      size: 14,
      visible: true,
      type: 'dot',
      args: [{ color: t.gridDot, thickness: 2 }]
    } as never)
  }
}

export function applyBackground(graph: Graph, theme: ThemeName) {
  graph.background.draw({ color: THEMES_LOOKUP[theme].bg })
}

/* ---------------- 主题切换（迁移默认配色的元素） ---------------- */

export function applyThemeToCells(graph: Graph, theme: ThemeName) {
  const t = THEMES_LOOKUP[theme]
  graph.getNodes().forEach((node: Node) => {
    const shape = node.shape
    if (shape === 'topo-node' || shape === 'topo-text') {
      const fill = String(node.attr('label/fill') ?? '')
      if (THEMES_LOOKUP.white.labelCandidates.includes(fill) || THEMES_LOOKUP.dark.labelCandidates.includes(fill)) {
        node.attr('label/fill', t.labelFg)
      }
    } else if (shape === TOPO_REGION) {
      const bodyFill = String(node.attr('body/fill') ?? '')
      const knownFills = Object.values(THEMES_LOOKUP).map((x) => x.regionFill)
      if (knownFills.includes(bodyFill)) {
        node.attr('body/fill', t.regionFill)
        node.attr('body/stroke', t.regionStroke)
        const labelFill = String(node.attr('label/fill') ?? '')
        const knownLabels = Object.values(THEMES_LOOKUP).map((x) => x.regionLabel)
        if (knownLabels.includes(labelFill)) node.attr('label/fill', t.regionLabel)
      }
    }
  })
  graph.getEdges().forEach((edge) => {
    const state = readEdgeLabelState(edge)
    if (state.text) {
      const known = Object.values(THEMES_LOOKUP).map((x) => x.edgeLabelFg)
      if (known.includes(state.color)) applyEdgeLabel([edge], { color: t.edgeLabelFg })
    }
    // 双线内芯随主题换底色（芯色不属于用户样式，直接跟随）
    if (edge.attr('core/stroke') != null) {
      edge.attr('core/stroke', t.bg)
    }
  })
}

/* ---------------- Graph 构建 ---------------- */

export function createGraph(container: HTMLElement): Graph {
  const theme = getTheme()
  const t = THEMES_LOOKUP[theme]

  let graph: Graph

  graph = new Graph({
    container,
    autoResize: true,
    background: { color: t.bg },
    grid: {
      size: 10,
      visible: true,
      type: 'doubleMesh',
      args: [
        { color: t.gridFine[0], thickness: 1 },
        { color: t.gridFine[1], thickness: 1, factor: 5 }
      ]
    },
    panning: {
      enabled: true,
      eventTypes: ['rightMouseDown', 'mouseWheel']
    },
    mousewheel: {
      // 关掉 X6 内建缩放：它只看 deltaY 正负、忽略大小，触控板捏合连发小事件时每发一步 → 跳变。
      // 改用下方自定义比例缩放（rAF 节流 + exp 映射），plain-wheel 平移仍由 panning 接管。
      enabled: false,
      zoomAtMousePosition: true,
      modifiers: ['ctrl'],
      minScale: 0.2,
      maxScale: 4
    },
    embedding: {
      enabled: true,
      findParent: ({ node }: { node: Node }) => {
        const center = node.getBBox().getCenter()
        return graph
          .getNodes()
          .filter((n) => n.shape === TOPO_REGION && n.id !== node.id && n.getBBox().containsPoint(center))
      },
      validate: ({ parent, child }: { parent: Node; child: Node }) =>
        parent.shape === TOPO_REGION && parent.id !== child.id,
      frontOnly: true
    },
    connecting: {
      allowBlank: false,
      allowLoop: false,
      allowEdge: false,
      allowMulti: true,
      highlight: true,
      anchor: 'center',
      // edgeEnd：端口终端终止于端口圆心（固定四端点）；主体/区域沿用 boundary 内探贴合
      connectionPoint: { name: 'edgeEnd' },
      createEdge() {
        // 连线拖拽状态标记（仅交互建边触发；load/paste 不走此路径）
        container.classList.add('is-connecting')
        return createEdgeCell()
      }
    },
    highlighting: {
      magnetAvailable: {
        name: 'stroke',
        args: { padding: 2, attrs: { strokeWidth: 2, stroke: '#1677ff' } }
      }
    }
  })

  const clearConnecting = () => container.classList.remove('is-connecting')
  graph.on('edge:connected', (args: { edge: Edge; e?: MouseEvent }) => {
    clearConnecting()
    // 交互建边/重连：落点定型 + 清除过时 dir（手动操作优先于声明，之后布局不再改写）
    const edge = args?.edge
    if (!edge) return
    const was = graph.isHistoryEnabled()
    if (was) graph.disableHistory()
    try {
      const ev = args.e
      finalizeEdgePorts(graph, edge, ev ? { x: ev.clientX, y: ev.clientY } : undefined)
      const data = edge.getData() as Record<string, unknown> | undefined
      if (data && data.dir) edge.setData({ ...data, dir: '' })
    } finally {
      if (was) graph.enableHistory()
    }
  })

  /* 预览吸附：拖拽中把跟随光标的终端点实时吸附到最近端口（ui 变更，不入历史） */
  graph.on('edge:change:target', (args: { edge?: Edge; options?: { ui?: boolean } }) => {
    if (!args?.edge || !args.options?.ui) return
    previewSnapTerminal(graph, args.edge, 'target')
  })
  graph.on('edge:change:source', (args: { edge?: Edge; options?: { ui?: boolean } }) => {
    if (!args?.edge || !args.options?.ui) return
    previewSnapTerminal(graph, args.edge, 'source')
  })
  graph.on('cell:removed', clearConnecting)
  graph.on('blank:mousedown', clearConnecting) // 兜底：任何新交互前复位

  graph.use(new History())
  graph.use(new Keyboard({ global: true }))
  graph.use(new Clipboard())
  graph.use(
    new Selection({
      enabled: true,
      rubberband: true,
      showNodeSelectionBox: true,
      showEdgeSelectionBox: false,
      pointerEvents: 'none'
    })
  )
  graph.use(new Snapline({ enabled: true }))
  graph.use(
    new Transform({
      resizing: { enabled: true, minWidth: 40, minHeight: 32, preserveAspectRatio: true },
      rotating: false
    })
  )
  graph.use(new Export())

  /* 自定义比例缩放（替代 X6 内建 mousewheel）：ctrl+wheel（Mac pinch 合成 ctrl+wheel）时，
     按 deltaY 累积走 exp 映射 + rAF 节流 → 触控板捏合平滑连续，不再每事件跳一步。
     非 ctrl 放行给 panning 的 plain-wheel 平移。 */
  const MIN_SCALE = 0.2
  const MAX_SCALE = 4
  const SENS = 0.0025
  let wheelAcc = 0
  let wheelRaf = 0
  const flushWheel = () => {
    wheelRaf = 0
    const dy = wheelAcc
    wheelAcc = 0
    if (!dy) return
    const cur = graph.transform.getScale().sx
    const next = Math.max(MIN_SCALE, Math.min(MAX_SCALE, cur * Math.exp(-dy * SENS)))
    if (next === cur) return
    // 锚定光标：取容器内最后已知指针位置
    const rect = container.getBoundingClientRect()
    const origin = graph.clientToGraph({ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 } as never)
    graph.zoom(next, { absolute: true, center: origin as never })
  }
  const onWheel = (e: WheelEvent) => {
    if (!e.ctrlKey) return // 非 ctrl：放行（平移）
    e.preventDefault()
    wheelAcc += e.deltaY
    if (!wheelRaf) wheelRaf = requestAnimationFrame(flushWheel)
  }
  container.addEventListener('wheel', onWheel, { passive: false })
  // graph 销毁时解绑（X6 graph.dispose 不会自动清我们挂的原生监听）
  const origDispose = graph.dispose.bind(graph)
  ;(graph as unknown as { dispose: () => void }).dispose = () => {
    container.removeEventListener('wheel', onWheel)
    if (wheelRaf) cancelAnimationFrame(wheelRaf)
    origDispose()
  }

  return graph
}

/* ---------------- 导出辅助 ---------------- */

export function contentSize(graph: Graph, padding: number) {
  const bbox = graph.getContentBBox()
  return {
    width: Math.max(1, Math.round(bbox.width) + padding * 2),
    height: Math.max(1, Math.round(bbox.height) + padding * 2)
  }
}
