import { Graph, Node } from '@antv/x6'

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

let registered = false

export function registerTopoShapes() {
  if (registered) return
  registered = true

  Graph.registerNode(
    TOPO_NODE,
    {
      inherit: 'rect',
      width: 100,
      height: 84,
      markup: [
        { tagName: 'image', selector: 'icon' },
        { tagName: 'text', selector: 'label' }
      ],
      attrs: {
        icon: {
          refWidth: '100%',
          refHeight: '64%',
          refX: 0,
          refY: 2,
          preserveAspectRatio: 'xMidYMid meet',
          xlinkHref: ''
        },
        label: {
          refX: '50%',
          refY: '82%',
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 12,
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
        items: [{ group: 'top' }, { group: 'right' }, { group: 'bottom' }, { group: 'left' }]
      }
    },
    true
  )

  Graph.registerNode(
    TOPO_REGION,
    {
      inherit: 'rect',
      width: 200,
      height: 120,
      attrs: {
        body: {
          fill: '#f6f8fb',
          stroke: '#c3cddb',
          strokeWidth: 1,
          strokeDasharray: '5 4',
          rx: 10,
          ry: 10
        },
        label: {
          refX: '50%',
          refY: 16,
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 13,
          fill: '#8a94a6'
        }
      },
      label: '区域'
    },
    true
  )

  Graph.registerNode(
    TOPO_TEXT,
    {
      inherit: 'rect',
      width: 120,
      height: 32,
      markup: [{ tagName: 'text', selector: 'label' }],
      attrs: {
        body: { fill: 'transparent', stroke: 'transparent' },
        label: {
          refX: '50%',
          refY: '50%',
          textAnchor: 'middle',
          textVerticalAnchor: 'middle',
          fontSize: 14,
          fill: '#26282c'
        }
      },
      label: '双击编辑文本'
    },
    true
  )
}

export interface TopoNodeData {
  icon?: string
  label?: string
}

export function createTopoNodeMeta(iconSrc: string, label: string): Node.Metadata {
  return {
    shape: TOPO_NODE,
    attrs: {
      icon: { xlinkHref: iconSrc },
      label: { text: label }
    },
    data: { icon: iconSrc, label }
  }
}
