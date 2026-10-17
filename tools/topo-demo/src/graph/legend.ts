import { Graph, Node } from '@antv/x6'
import { getActivePreset, resolveColor } from './linkTypes'
import { themeTokens } from './theme'

/* 线路图例：扫描边的 linkType → 图内可拖动 legend 节点（随 PNG/SVG 导出）。
   触发：YAML 应用自动生成 / 工具栏「图例」手动重建 / 自动布局后重定位。
   位置：固定拓扑左上角（与最左设备左对齐、最上设备上对齐）；
   若与设备重叠则改放右上角（与最右设备右对齐）。 */

interface LegendRow {
  type: string
  desc: string
  double?: boolean
  oval?: boolean
  color: string
  style: 'solid' | 'dashed' | 'dotted'
  width: number
}

const FAMILY =
  "-apple-system, 'PingFang SC', 'Microsoft YaHei', 'Helvetica Neue', Arial, sans-serif"
const ROW_H = 24
const TITLE_H = 28
const PAD = 14
const SAMPLE_W = 44

/** 非 legend 内容包围盒 */
function contentBBox(graph: Graph): { minX: number; minY: number; maxX: number } {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  graph.getNodes().forEach((n) => {
    if (n.getData()?.legend === true) return
    const bb = n.getBBox()
    minX = Math.min(minX, bb.x)
    minY = Math.min(minY, bb.y)
    maxX = Math.max(maxX, bb.x + bb.width)
  })
  if (!isFinite(minX)) return { minX: 40, minY: 40, maxX: 40 }
  return { minX, minY, maxX }
}

/** 图例锚点：左上优先；与任一设备 bbox 相交则改右上 */
export function legendAnchor(
  graph: Graph,
  legendW: number,
  legendH: number
): { x: number; y: number } {
  const { minX, minY, maxX } = contentBBox(graph)
  const topY = Math.round(minY)
  const leftX = Math.round(minX)
  const rightX = Math.round(maxX - legendW)
  const devices = graph
    .getNodes()
    .filter((n) => n.shape !== 'legend' && !n.getData()?.legend)
    .map((n) => n.getBBox())
  const overlaps = (x: number, y: number) =>
    devices.some(
      (b) => x < b.x + b.width && x + legendW > b.x && y < b.y + b.height && y + legendH > b.y
    )
  if (!overlaps(leftX, topY)) return { x: leftX, y: topY }
  return { x: rightX, y: topY }
}

/** 把已有 legend 节点重定位到当前内容锚点（不重建行内容） */
export function repositionLegend(graph: Graph): void {
  const legend = graph.getNodes().find((n) => n.getData()?.legend === true)
  if (!legend) return
  const sz = legend.getSize()
  const pos = legendAnchor(graph, sz.width, sz.height)
  legend.setPosition(pos.x, pos.y)
}

export function syncLegend(graph: Graph): boolean {
  // 清旧
  graph
    .getNodes()
    .filter((n) => n.getData()?.legend === true)
    .forEach((n) => graph.removeCell(n))

  // 收集在用类型（首现顺序）
  const rows: LegendRow[] = []
  const seen = new Set<string>()
  graph.getEdges().forEach((e) => {
    const t = String((e.getData() as { linkType?: string } | undefined)?.linkType ?? '')
    if (!t || seen.has(t)) return
    const p = getActivePreset(t)
    if (!p) return
    seen.add(t)
    rows.push({
      type: t,
      desc: p.desc,
      double: !!p.double,
      oval: !!p.oval,
      color: resolveColor(p.color) || '#4b5563',
      style: p.style,
      width: p.width
    })
  })
  if (!rows.length) return false

  const t = themeTokens()
  const estDescWidth = (text: string): number => {
    let w = 0
    for (const ch of text) w += ch.charCodeAt(0) > 255 ? 11 : 6.4
    return w
  }
  const descX = PAD + 4 + SAMPLE_W + 12
  const maxDesc = Math.max(...rows.map((r) => estDescWidth(r.desc)))
  const width = Math.round(descX + maxDesc + PAD)
  const height = TITLE_H + rows.length * ROW_H + PAD * 2

  const markup: unknown[] = [
    { tagName: 'rect', selector: 'body' },
    { tagName: 'text', selector: 'title' }
  ]
  const attrs: Record<string, Record<string, unknown>> = {
    body: { fill: t.bg, stroke: t.regionStroke, strokeWidth: 1, rx: 10, ry: 10 },
    title: {
      x: PAD + 2,
      y: 20,
      fontSize: 14,
      fontWeight: 600,
      textAnchor: 'start',
      fill: t.labelFg,
      text: '图例',
      fontFamily: FAMILY
    }
  }

  rows.forEach((r, i) => {
    const y = TITLE_H + PAD + i * ROW_H + 12
    const d = `M${PAD + 4} ${y} H${PAD + 4 + SAMPLE_W}`
    const si = `s${i}`
    markup.push({ tagName: 'path', selector: si })
    if (r.double) markup.push({ tagName: 'path', selector: `${si}c` })
    if (r.oval) markup.push({ tagName: 'ellipse', selector: `${si}o` })
    markup.push({ tagName: 'text', selector: `d${i}` })
    const dash = r.style === 'dashed' ? '7 5' : r.style === 'dotted' ? '1 7' : ''
    attrs[si] = {
      d,
      fill: 'none',
      stroke: r.color,
      strokeWidth: r.double ? 4 : Math.min(r.width, 3.5),
      strokeDasharray: dash,
      strokeLinecap: r.style === 'dotted' ? 'round' : 'butt',
      sourceMarker: false,
      targetMarker: false
    }
    if (r.double) {
      attrs[`${si}c`] = { d, fill: 'none', stroke: t.bg, strokeWidth: 1.6, strokeLinecap: 'butt' }
    }
    if (r.oval) {
      // 堆叠双线中点椭圆（与边上 label 同款：虚线空心轮廓）
      attrs[`${si}o`] = {
        cx: PAD + 4 + SAMPLE_W / 2,
        cy: y,
        rx: 6,
        ry: 9,
        fill: 'none',
        stroke: r.color,
        strokeWidth: 1.4,
        strokeDasharray: '3 2.2'
      }
    }
    /* 单列说明：只写速率/材质/用途，不显示内置类型名 */
    attrs[`d${i}`] = {
      x: descX,
      y: y + 4,
      fontSize: 11.5,
      textAnchor: 'start',
      fill: t.labelFg,
      opacity: 0.85,
      text: r.desc,
      fontFamily: FAMILY
    }
  })

  const pos = legendAnchor(graph, width, height)
  graph.addNode({
    shape: 'legend',
    x: pos.x,
    y: pos.y,
    width,
    height,
    zIndex: 3,
    markup,
    attrs,
    data: { legend: true, rows: rows.map((r) => r.type) }
  } as never)
  return true
}
