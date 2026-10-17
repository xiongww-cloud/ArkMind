import { Graph, Node } from '@antv/x6'
import { getTheme, themeTokens } from './theme'
import { tint } from './linkTypes'

const portAttrs = {
  circle: {
    r: 4,
    magnet: true,
    stroke: '#1677ff',
    strokeWidth: 1.5,
    fill: '#ffffff'
  }
}

export const TOPO_NODE = 'topo-node'
export const TOPO_REGION = 'topo-region'
export const TOPO_TEXT = 'topo-text'

/* 区域容器几何：左右/下内边距、顶部标题留白、最小尺寸（编组、refit、自动布局共用） */
export const REGION_PAD_X = 28
export const REGION_PAD_Y = 6
export const REGION_TITLE_BAR = 26
export const REGION_MIN_W = 180
export const REGION_MIN_H = 120

export type NodeStyleMode = 'plain' | 'card'

export const NODE_STYLE_MODES: { key: NodeStyleMode; label: string }[] = [
  { key: 'plain', label: '纯图标' },
  { key: 'card', label: '卡片' }
]

export type NodeStatus = 'none' | 'ok' | 'warn' | 'down' | 'off'

export const NODE_STATUSES: { key: NodeStatus; label: string }[] = [
  { key: 'none', label: '无' },
  { key: 'ok', label: '正常' },
  { key: 'warn', label: '告警' },
  { key: 'down', label: '故障' },
  { key: 'off', label: '离线' }
]

const STATUS_COLORS: Record<Exclude<NodeStatus, 'none'>, string> = {
  ok: '#16a34a',
  warn: '#f59e0b',
  down: '#ef4444',
  off: '#94a3b8'
}

export const LABEL_FONT_SIZES = [11, 12, 14, 16]
export const LABEL_COLORS = ['#26282c', '#4b5563', '#1677ff', '#d4380d', '#389e0d', '#722ed1']

let registered = false

export function registerTopoShapes() {
  if (registered) return
  registered = true

  Graph.registerNode(
    TOPO_NODE,
    {
      inherit: 'rect',
      width: 76,
      height: 76,
      markup: [
        { tagName: 'rect', selector: 'body' },
        { tagName: 'image', selector: 'icon' },
        { tagName: 'circle', selector: 'status' },
        { tagName: 'text', selector: 'label' }
      ],
      attrs: {
        body: {
          fill: 'transparent',
          stroke: 'transparent',
          strokeWidth: 1,
          rx: 12,
          ry: 12
        },
        icon: {
          refWidth: '100%',
          refHeight: '100%',
          refX: 0,
          refY: 0,
          preserveAspectRatio: 'xMidYMid meet',
          xlinkHref: ''
        },
        status: {
          refX: '88%',
          refY: '8%',
          r: 5,
          fill: STATUS_COLORS.ok,
          stroke: '#ffffff',
          strokeWidth: 1.5,
          visibility: 'hidden',
          eventProbability: 0,
          style: { pointerEvents: 'none' }
        },
        label: {
          refX: '50%',
          refY: '120%',
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 12,
          fontFamily:
            "-apple-system, 'PingFang SC', 'Microsoft YaHei', 'Helvetica Neue', Arial, sans-serif",
          fill: '#26282c',
          pointerEvents: 'none'
        }
      },
      ports: {
        groups: {
          top: { position: 'top', attrs: portAttrs },
          right: { position: 'right', attrs: portAttrs },
          bottom: { position: 'bottom', attrs: portAttrs },
          left: { position: 'left', attrs: portAttrs }
        },
        /* 显式 id：snapEdgePorts 按 id 绑定四端点（side 中点） */
        items: [
          { id: 'top', group: 'top' },
          { id: 'right', group: 'right' },
          { id: 'bottom', group: 'bottom' },
          { id: 'left', group: 'left' }
        ]
      }
    },
    true
  )

  Graph.registerNode(
    TOPO_REGION,
    {
      inherit: 'rect',
      width: 220,
      height: 140,
      attrs: {
        body: {
          fill: '#f4f6fa',
          stroke: '#c9d2df',
          strokeWidth: 1,
          strokeDasharray: '6 4',
          rx: 12,
          ry: 12
        },
        label: {
          refX: '50%',
          refY: 14,
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 15,
          fontWeight: 600,
          fontFamily:
            "-apple-system, 'PingFang SC', 'Microsoft YaHei', 'Helvetica Neue', Arial, sans-serif",
          fill: '#8a94a6'
        }
      },
      label: '区域'
    },
    true
  )

  /* 图例容器：实例级 markup（行数动态）。
     注意：不能 inherit 'rect'——矩形默认 label 定位（refX50/refY50/textAnchor middle）
     会污染实例标题，产生中心偏移；空定义 = 只用实例 markup/attrs。 */
  Graph.registerNode('legend', {}, true)

  Graph.registerNode(
    TOPO_TEXT,
    {
      inherit: 'rect',
      width: 120,
      height: 32,
      markup: [{ tagName: 'text', selector: 'label' }],
      attrs: {
        label: {
          refX: '50%',
          refY: '50%',
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 14,
          fontFamily:
            "-apple-system, 'PingFang SC', 'Microsoft YaHei', 'Helvetica Neue', Arial, sans-serif",
          fill: '#26282c'
        }
      },
      label: '双击编辑文本'
    },
    true
  )
}

/* ---------------- 节点样式 ---------------- */

/* 区域色着：同色系浅填充 + 同色边框（rgba 合成与主题无关，深浅画布均成立）。
   不传 color → 恢复当前主题默认区域色。 */
export function applyRegionColor(node: Node, color?: string | null) {
  if (!color) {
    const t = themeTokens()
    node.attr('body/fill', t.regionFill)
    node.attr('body/stroke', t.regionStroke)
    node.setData({ ...(node.getData() || {}), color: undefined })
    return
  }
  const fill = tint(color, 0.12)
  const stroke = tint(color, 0.55)
  if (fill && stroke) {
    node.attr('body/fill', fill)
    node.attr('body/stroke', stroke)
    node.attr('body/strokeWidth', 1.5)
    node.setData({ ...(node.getData() || {}), color })
  }
}

export function applyNodeStyleMode(node: Node, mode: NodeStyleMode) {
  if (mode === 'card') {
    node.attr('body/fill', getTheme() === 'dark' ? '#2b3648' : '#f8fafc')
    node.attr('body/stroke', getTheme() === 'dark' ? '#46536a' : '#dde3eb')
    node.attr('body/strokeWidth', 1)
  } else {
    node.attr('body/fill', 'transparent')
    node.attr('body/stroke', 'transparent')
  }
  node.setData({ ...(node.getData() || {}), styleMode: mode })
}

export function readNodeStyleMode(node: Node): NodeStyleMode {
  return (node.getData() as { styleMode?: NodeStyleMode })?.styleMode || 'plain'
}

export function setNodeStatus(node: Node, status: NodeStatus) {
  if (status === 'none') {
    node.attr('status/visibility', 'hidden')
  } else {
    node.attr('status/fill', STATUS_COLORS[status])
    node.attr('status/visibility', 'visible')
  }
  node.setData({ ...(node.getData() || {}), status })
}

export function readNodeStatus(node: Node): NodeStatus {
  return (node.getData() as { status?: NodeStatus })?.status || 'none'
}

export function applyNodeLabelStyle(
  node: Node,
  patch: { fontSize?: number; fill?: string; fontWeight?: string | number }
) {
  if (patch.fontSize !== undefined) node.attr('label/fontSize', patch.fontSize)
  if (patch.fill !== undefined) node.attr('label/fill', patch.fill)
  if (patch.fontWeight !== undefined) node.attr('label/fontWeight', patch.fontWeight)
}

export function readNodeLabelStyle(node: Node): { fontSize: number; fill: string; fontWeight: string | number } {
  return {
    fontSize: (node.attr('label/fontSize') as number) || 12,
    fill: String(node.attr('label/fill') || themeTokens().labelFg),
    fontWeight: String(node.attr('label/fontWeight') || 'normal')
  }
}

/* ---------------- 节点创建 ---------------- */

export interface TopoNodeData {
  icon?: string
  label?: string
  styleMode?: NodeStyleMode
  status?: NodeStatus
}

export function createTopoNodeMeta(
  iconSrc: string,
  label: string,
  w?: number,
  h?: number
): Node.Metadata {
  return {
    shape: TOPO_NODE,
    width: w || 76,
    height: h || 76,
    zIndex: 0,
    attrs: {
      icon: { xlinkHref: iconSrc },
      label: { text: label, fill: themeTokens().labelFg }
    },
    data: { icon: iconSrc, label, styleMode: 'plain', status: 'none' }
  } as Node.Metadata
}

export function createRegionMeta(): Node.Metadata {
  const t = themeTokens()
  return {
    shape: TOPO_REGION,
    zIndex: -2,
    attrs: {
      body: { fill: t.regionFill, stroke: t.regionStroke },
      label: { fill: t.regionLabel, text: '区域' }
    },
    data: { label: '区域' }
  }
}

export function createTextNodeMeta(): Node.Metadata {
  return {
    shape: TOPO_TEXT,
    zIndex: 0,
    attrs: { label: { fill: themeTokens().labelFg } },
    data: { label: '双击编辑文本' }
  }
}
