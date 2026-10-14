import { Graph, Shape, Edge } from '@antv/x6'
import { Selection } from '@antv/x6-plugin-selection'
import { Snapline } from '@antv/x6-plugin-snapline'
import { History } from '@antv/x6-plugin-history'
import { Keyboard } from '@antv/x6-plugin-keyboard'
import { Clipboard } from '@antv/x6-plugin-clipboard'
import { Transform } from '@antv/x6-plugin-transform'
import { Export } from '@antv/x6-plugin-export'
import { MiniMap } from '@antv/x6-plugin-minimap'

export const EDGE_COLORS = ['#4b5563', '#1677ff', '#d4380d', '#389e0d', '#7cb305', '#722ed1']

const lineMarker = (color: string) => ({
  line: {
    stroke: color,
    strokeWidth: 2,
    strokeLinecap: 'butt',
    strokeDasharray: '',
    targetMarker: { name: 'block', width: 10, height: 8 }
  }
})

export function createGraph(container: HTMLElement, minimapContainer: HTMLElement): Graph {
  const graph = new Graph({
    container,
    autoResize: true,
    background: { color: '#ffffff' },
    grid: {
      size: 10,
      visible: true,
      type: 'doubleMesh',
      args: [
        { color: '#eef0f4', thickness: 1 },
        { color: '#dfe3ea', thickness: 1, factor: 5 }
      ]
    },
    panning: {
      enabled: true,
      modifiers: ['space'],
      eventTypes: ['leftMouseDown', 'rightMouseDown']
    },
    mousewheel: {
      enabled: true,
      zoomAtMousePosition: true,
      modifiers: null,
      minScale: 0.2,
      maxScale: 4
    },
    connecting: {
      snap: { radius: 24 },
      allowBlank: false,
      allowLoop: false,
      allowEdge: false,
      allowMulti: false,
      highlight: true,
      router: { name: 'manhattan', args: { padding: 16 } },
      connector: { name: 'rounded', args: { radius: 8 } },
      anchor: 'center',
      connectionPoint: 'boundary',
      createEdge() {
        return new Shape.Edge({
          attrs: lineMarker('#4b5563'),
          zIndex: -1
        })
      }
    },
    highlighting: {
      magnetAvailable: {
        name: 'stroke',
        args: { padding: 2, attrs: { strokeWidth: 2, stroke: '#1677ff' } }
      }
    }
  })

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
  graph.use(
    new MiniMap({
      container: minimapContainer,
      width: 210,
      height: 140,
      padding: 8,
      graphOptions: { async: true }
    })
  )

  return graph
}

export type EdgeLineStyle = 'solid' | 'dashed' | 'dotted'
export type EdgeMarker = 'none' | 'target' | 'both'

export function applyLineStyle(edges: Edge[], style: EdgeLineStyle) {
  edges.forEach((edge) => {
    const dash = style === 'dashed' ? '7 5' : style === 'dotted' ? '1 7' : ''
    const cap = style === 'dotted' ? 'round' : 'butt'
    edge.attr('line/strokeDasharray', dash)
    edge.attr('line/strokeLinecap', cap)
  })
}

export function applyMarker(edges: Edge[], marker: EdgeMarker) {
  edges.forEach((edge) => {
    if (marker === 'none') {
      edge.attr('line/targetMarker', null)
      edge.attr('line/sourceMarker', null)
    } else if (marker === 'target') {
      edge.attr('line/targetMarker', { name: 'block', width: 10, height: 8 })
      edge.attr('line/sourceMarker', null)
    } else {
      edge.attr('line/targetMarker', { name: 'block', width: 10, height: 8 })
      edge.attr('line/sourceMarker', { name: 'block', width: 10, height: 8 })
    }
  })
}

export function applyEdgeColor(edges: Edge[], color: string) {
  edges.forEach((edge) => edge.attr('line/stroke', color))
}

export function edgeLineStyle(edge: Edge): EdgeLineStyle {
  const dash = edge.attr('line/strokeDasharray') as string
  if (dash === '1 7') return 'dotted'
  if (dash) return 'dashed'
  return 'solid'
}

export function edgeMarker(edge: Edge): EdgeMarker {
  const t = edge.attr('line/targetMarker')
  const s = edge.attr('line/sourceMarker')
  if (t && s) return 'both'
  if (t) return 'target'
  return 'none'
}
