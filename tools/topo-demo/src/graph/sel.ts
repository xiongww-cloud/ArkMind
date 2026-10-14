import type { Node, Edge } from '@antv/x6'
import type { EdgeLineStyle, EdgeMarker } from './setup'

export interface SelInfo {
  kind: 'none' | 'node' | 'edge' | 'multi'
  nodes: Node[]
  edges: Edge[]
  name: string
  labelText: string
  lineStyle: EdgeLineStyle
  marker: EdgeMarker
  color: string
}

export function createSelInfo(): SelInfo {
  return {
    kind: 'none',
    nodes: [],
    edges: [],
    name: '',
    labelText: '',
    lineStyle: 'solid',
    marker: 'target',
    color: '#4b5563'
  }
}
