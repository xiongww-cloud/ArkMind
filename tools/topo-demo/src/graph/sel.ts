import type { Node, Edge } from '@antv/x6'
import type { EdgeShape, LineStyle, MarkerName } from './setup'
import type { NodeStyleMode, NodeStatus } from './node'

export interface SelInfo {
  kind: 'none' | 'node' | 'edge' | 'multi'
  nodes: Node[]
  edges: Edge[]
  /* node */
  name: string
  styleMode: NodeStyleMode
  status: NodeStatus
  labelFontSize: number
  labelColor: string
  labelBold: boolean
  /* edge */
  linkType: string
  labelText: string
  labelPos: number
  labelPilled: boolean
  labelFontSizeE: number
  labelColorE: string
  edgeShape: EdgeShape
  lineStyle: LineStyle
  headMarker: MarkerName
  tailMarker: MarkerName
  color: string
  strokeWidth: number
}

export function createSelInfo(): SelInfo {
  return {
    kind: 'none',
    nodes: [],
    edges: [],
    name: '',
    styleMode: 'plain',
    status: 'none',
    labelFontSize: 12,
    labelColor: '#26282c',
    labelBold: false,
    labelText: '',
    labelPos: 0.5,
    labelPilled: true,
    labelFontSizeE: 12,
    labelColorE: '#374151',
    linkType: '',
    edgeShape: 'rounded',
    lineStyle: 'solid',
    headMarker: 'none',
    tailMarker: 'none',
    color: '#4b5563',
    strokeWidth: 2
  }
}
