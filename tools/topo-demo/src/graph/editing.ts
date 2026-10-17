import { Graph, Node } from '@antv/x6'
import {
  createRegionMeta,
  REGION_PAD_X,
  REGION_PAD_Y,
  REGION_TITLE_BAR,
  REGION_MIN_W,
  REGION_MIN_H,
  TOPO_REGION
} from './node'
import { bindAllEdgePorts } from './setup'
import { repositionLegend } from './legend'

/* 只取顶层节点（父不在选区内的），移动父节点时子节点自动跟随，避免双重位移 */
function topLevelNodes(graph: Graph): Node[] {
  const selected = new Set(graph.getSelectedCells())
  return graph
    .getSelectedCells()
    .filter((c): c is Node => c.isNode())
    .filter((n) => {
      const p = n.getParent()
      return !p || !selected.has(p)
    })
}

export type AlignMode = 'left' | 'hcenter' | 'right' | 'top' | 'vcenter' | 'bottom'

export function alignSelection(graph: Graph, mode: AlignMode) {
  const nodes = topLevelNodes(graph)
  if (nodes.length < 2) return
  const boxes = nodes.map((n) => n.getBBox())
  const minX = Math.min(...boxes.map((b) => b.x))
  const maxX = Math.max(...boxes.map((b) => b.x + b.width))
  const minY = Math.min(...boxes.map((b) => b.y))
  const maxY = Math.max(...boxes.map((b) => b.y + b.height))
  nodes.forEach((n, i) => {
    const b = boxes[i]
    let { x, y } = b
    if (mode === 'left') x = minX
    else if (mode === 'right') x = maxX - b.width
    else if (mode === 'hcenter') x = (minX + maxX) / 2 - b.width / 2
    else if (mode === 'top') y = minY
    else if (mode === 'bottom') y = maxY - b.height
    else if (mode === 'vcenter') y = (minY + maxY) / 2 - b.height / 2
    n.setPosition(x, y)
  })
}

export function distributeSelection(graph: Graph, axis: 'h' | 'v') {
  const nodes = topLevelNodes(graph)
  if (nodes.length < 3) return
  const boxes = nodes.map((n) => n.getBBox())
  if (axis === 'h') {
    const order = boxes.map((b, i) => i).sort((a, b) => boxes[a].x - boxes[b].x)
    const first = boxes[order[0]]
    const last = boxes[order[order.length - 1]]
    const start = first.x + first.width / 2
    const end = last.x + last.width / 2
    const step = (end - start) / (order.length - 1)
    order.forEach((idx, rank) => {
      if (rank === 0 || rank === order.length - 1) return
      const b = boxes[idx]
      nodes[idx].setPosition(start + step * rank - b.width / 2, b.y)
    })
  } else {
    const order = boxes.map((b, i) => i).sort((a, b) => boxes[a].y - boxes[b].y)
    const first = boxes[order[0]]
    const last = boxes[order[order.length - 1]]
    const start = first.y + first.height / 2
    const end = last.y + last.height / 2
    const step = (end - start) / (order.length - 1)
    order.forEach((idx, rank) => {
      if (rank === 0 || rank === order.length - 1) return
      const b = boxes[idx]
      nodes[idx].setPosition(b.x, start + step * rank - b.height / 2)
    })
  }
}

export function nudgeSelection(graph: Graph, dx: number, dy: number) {
  topLevelNodes(graph).forEach((n) => {
    const p = n.getPosition()
    n.setPosition(p.x + dx, p.y + dy)
  })
}

export function duplicateSelection(graph: Graph): Node[] {
  const cells = graph.getSelectedCells()
  if (!cells.length) return []
  graph.copy(cells)
  const pasted = graph.paste({ offset: { dx: 28, dy: 28 } })
  graph.cleanSelection()
  pasted.forEach((c) => graph.select(c))
  return pasted.filter((c): c is Node => c.isNode())
}

export function zOrder(graph: Graph, action: 'front' | 'back') {
  graph.getSelectedCells().forEach((c) => (action === 'front' ? c.toFront() : c.toBack()))
}

export function groupSelection(graph: Graph) {
  const nodes = topLevelNodes(graph)
  if (nodes.length < 2) return
  const ms = nodes.map(childMetrics)
  const minX = Math.min(...ms.map((m) => m.box.x)) - REGION_PAD_X
  const minY = Math.min(...ms.map((m) => m.box.y)) - REGION_TITLE_BAR
  const maxX = Math.max(...ms.map((m) => m.box.x + m.box.width)) + REGION_PAD_X
  const maxY =
    Math.max(...ms.map((m) => m.box.y + m.box.height)) +
    Math.max(...ms.map((m) => m.overhang)) +
    REGION_PAD_Y
  const region = graph.addNode({
    ...createRegionMeta(),
    x: minX,
    y: minY,
    width: maxX - minX,
    height: maxY - minY
  } as never)
  nodes.forEach((n) => n.addTo(region))
  graph.cleanSelection()
  graph.select(region)
}

export function ungroupSelection(graph: Graph) {
  const cells = graph.getSelectedCells()
  cells.forEach((c) => {
    if (!c.isNode()) return
    const node = c as Node
    if (node.shape === 'topo-region') {
      graph
        .getNodes()
        .filter((n) => n.getParentId() === node.id)
        .forEach((child) => child.setParent(null))
    }
    if (node.getParent()) node.setParent(null)
  })
}

export function zoomToSelection(graph: Graph) {
  const cells = graph.getSelectedCells()
  if (!cells.length) {
    graph.zoomToFit({ padding: 40, maxScale: 1 })
    return
  }
  const boxes = cells.map((c) => c.getBBox())
  const pad = 48
  const rect = {
    x: Math.min(...boxes.map((b) => b.x)) - pad,
    y: Math.min(...boxes.map((b) => b.y)) - pad,
    width: Math.max(...boxes.map((b) => b.x + b.width)) - Math.min(...boxes.map((b) => b.x)) + pad * 2,
    height: Math.max(...boxes.map((b) => b.y + b.height)) - Math.min(...boxes.map((b) => b.y)) + pad * 2
  }
  graph.zoomToRect(rect, { maxScale: 2 })
}

/* ---------------- 区域容器 refit ---------------- */

/* 子节点变化后把区域收缩/扩张到恰好包住内容（含边距与标题留白）。
   只动区域自身位置/尺寸，从不移动子节点；空区域保持现状。 */
/* 设备名称在方框外下方：悬出量 ≈ 0.2×高 + 8px */
export function labelOverhang(h: number): number {
  return Math.round(h * 0.2) + 8
}

function estLabelWidth(node: Node): number {
  const text = String(node.attr('label/text') ?? '')
  if (!text) return 0
  let w = 0
  for (const ch of text) w += ch.charCodeAt(0) > 255 ? 12 : 7
  return w
}

/* 子节点度量：水平扩展到含名称宽度，底部悬出量（区域无外悬名称） */
function childMetrics(n: Node): {
  box: { x: number; y: number; width: number; height: number }
  overhang: number
} {
  const b = n.getBBox()
  const overhang = n.shape === TOPO_REGION ? 0 : labelOverhang(b.height)
  const over = Math.max(0, (estLabelWidth(n) - b.width) / 2)
  return {
    box: { x: b.x - over, y: b.y, width: b.width + over * 2, height: b.height },
    overhang
  }
}

/* 区域尺寸 = 内容(含名称) + 左右边距 + 顶部标题 + 底部(最大名称悬出 + 下边距) */
function regionSizeOf(
  items: { box: { x: number; y: number; width: number; height: number }; overhang: number }[]
): { w: number; h: number } | null {
  if (!items.length) return null
  const minX = Math.min(...items.map((m) => m.box.x))
  const maxX = Math.max(...items.map((m) => m.box.x + m.box.width))
  const minY = Math.min(...items.map((m) => m.box.y))
  const maxY = Math.max(...items.map((m) => m.box.y + m.box.height))
  const maxOv = Math.max(...items.map((m) => m.overhang))
  return {
    w: Math.max(REGION_MIN_W, maxX - minX + REGION_PAD_X * 2),
    h: Math.max(REGION_MIN_H, maxY - minY + REGION_TITLE_BAR + maxOv + REGION_PAD_Y)
  }
}

export function refitRegion(graph: Graph, region: Node): void {
  const children = graph.getNodes().filter((n) => n.getParentId() === region.id)
  if (!children.length) return
  const ms = children.map(childMetrics)
  const fit = regionSizeOf(ms)
  if (!fit) return
  const minX = Math.min(...ms.map((m) => m.box.x))
  const maxX = Math.max(...ms.map((m) => m.box.x + m.box.width))
  const minY = Math.min(...ms.map((m) => m.box.y))
  /* 水平居中（最小尺寸钳制后仍以内容中心对齐），顶部按标题锚定 */
  region.setPosition(Math.round((minX + maxX) / 2 - fit.w / 2), Math.round(minY - REGION_TITLE_BAR))
  region.resize(fit.w, fit.h)
}

export function refitAllRegions(graph: Graph): void {
  const regions = graph.getNodes().filter((n) => n.shape === TOPO_REGION)
  // 由深到浅：先内层后外层
  const depth = (n: Node): number => {
    let d = 0
    let p = n.getParent()
    while (p) {
      d++
      p = p.getParent()
    }
    return d
  }
  regions.sort((a, b) => depth(b) - depth(a))
  regions.forEach((r) => refitRegion(graph, r))
}

/* 挂起中的贴合定时器（模块级，供 YAML 导入等原子操作在收尾时取消） */
let refitTimer: ReturnType<typeof setTimeout> | undefined

export function cancelRegionRefit() {
  if (refitTimer) {
    clearTimeout(refitTimer)
    refitTimer = undefined
  }
}

/* 交互态自动缩放：子节点拖动/进出区域（防抖）时挂接；返回解绑函数。
   布局批量与区域自身拖动产生的级联事件经 debounce+幂等 refit 收敛，无需特判。 */
export function attachRegionRefit(graph: Graph): () => void {
  const schedule = () => {
    if (refitTimer) clearTimeout(refitTimer)
    refitTimer = setTimeout(() => {
      refitTimer = undefined
      if (suppressRefit > 0) return
      refitAllRegions(graph)
    }, 150)
  }
  const onPosition = ({ node }: { node: Node }) => {
    if (node.getParent() && node.getParent()?.shape === TOPO_REGION) schedule()
  }
  const onParent = () => schedule()
  graph.on('node:change:position', onPosition)
  graph.on('node:change:parent', onParent)
  return () => {
    cancelRegionRefit()
    graph.off('node:change:position', onPosition)
    graph.off('node:change:parent', onParent)
  }
}

/* ---------------- dagre 自动布局（v2：compound + 连线方向分层 + 区域容器） ---------------- */

import { DagreLayout } from '@antv/layout'

export type LayoutDir = 'TB' | 'LR'

/* 布局批量期间抑制交互 refit（同一模块内共享） */
let suppressRefit = 0

function nodeDepth(n: Node): number {
  let d = 0
  let p = n.getParent()
  while (p) {
    d++
    p = p.getParent()
  }
  return d
}

export interface AutoLayoutResult {
  ranker: string
  fallback: boolean
}

/* 应急网格排列：BFS 分层（最短路径层）+ 行列打包，区域子网格内排后 refit。
   纯算术实现——dagre 全部失败时保证不堆叠、不抛错。 */
function gridPack(graph: Graph, topNodes: Node[], edges: { source: string; target: string }[], rankdir: LayoutDir): void {
  const outgoing = new Map<string, string[]>()
  const incoming = new Set<string>()
  edges.forEach((e) => {
    outgoing.set(e.source, [...(outgoing.get(e.source) ?? []), e.target])
    incoming.add(e.target)
  })
  const layer = new Map<string, number>()
  const q = topNodes.filter((n) => !incoming.has(n.id)).map((n) => ({ id: n.id, d: 0 }))
  while (q.length) {
    const { id, d } = q.shift() as { id: string; d: number }
    if (layer.has(id)) continue // 首访即最短层（防环）
    layer.set(id, d)
    for (const t of outgoing.get(id) ?? []) q.push({ id: t, d: d + 1 })
  }
  let maxD = 0
  layer.forEach((v) => {
    maxD = Math.max(maxD, v)
  })
  const rowsMap = new Map<number, Node[]>()
  topNodes.forEach((n) => {
    const d = layer.has(n.id) ? (layer.get(n.id) as number) : maxD + 1
    rowsMap.set(d, [...(rowsMap.get(d) ?? []), n])
  })
  const NODESEP = 48
  const RANKSEP = 56
  const sorted = [...rowsMap.keys()].sort((a, b) => a - b)
  if (rankdir === 'TB') {
    let y = 40
    sorted.forEach((d) => {
      let x = 40
      let maxH = 0
      ;(rowsMap.get(d) as Node[]).forEach((n) => {
        const sz = n.getSize()
        n.setPosition(Math.round(x), Math.round(y))
        x += sz.width + NODESEP
        maxH = Math.max(maxH, sz.height)
      })
      y += maxH + RANKSEP
    })
  } else {
    let x = 40
    sorted.forEach((d) => {
      let y = 40
      let maxW = 0
      ;(rowsMap.get(d) as Node[]).forEach((n) => {
        const sz = n.getSize()
        n.setPosition(Math.round(x), Math.round(y))
        y += sz.height + NODESEP
        maxW = Math.max(maxW, sz.width)
      })
      x += maxW + RANKSEP
    })
  }
  // 区域子网格（每行2台）后吸附
  topNodes
    .filter((n) => n.shape === TOPO_REGION)
    .forEach((region) => {
      const kids = graph.getNodes().filter((n) => n.getParentId() === region.id)
      if (!kids.length) return
      const baseX = region.getPosition().x + REGION_PAD_X
      const baseY = region.getPosition().y + REGION_TITLE_BAR
      let cx = baseX
      let cy = baseY
      let rowH = 0
      let i = 0
      kids.forEach((k) => {
        const sz = k.getSize()
        if (i >= 2) {
          cx = baseX
          cy += rowH + 24
          rowH = 0
          i = 0
        }
        k.setPosition(Math.round(cx), Math.round(cy))
        cx += sz.width + 32
        rowH = Math.max(rowH, sz.height)
        i += 1
      })
      refitRegion(graph, region)
    })
}

/* ---------------- 中轴对称层次布局（自研主算法） ----------------
   目标：中轴线 + 两边对称 + 同数量层垂直对齐。整体居中口径（子树不等宽时父居中于子列整体）。
   cross 边（HA/堆叠/左右向）转"同层对"约束（不再丢弃）→ 修缺陷2/3。 */

const PAIRSEP = 56 // 超节点内部间距（HA/堆叠对之间）

/** 在给定节点集上做对称布局，返回左上角坐标（局部，最小约 (0,0)）。
    nodeIds: 参与布局的节点 id；sizeOf: id→尺寸；dirEdges: 定向边（源在上/左）；
    crossPairs: 同层对（HA/堆叠）；rankdir: TB/LR。失败返回 null。 */
function symmetricLayout(
  nodeIds: string[],
  sizeOf: (id: string) => { w: number; h: number },
  dirEdges: { source: string; target: string }[],
  crossPairs: [string, string][],
  rankdir: LayoutDir
): Map<string, { x: number; y: number }> | null {
  if (!nodeIds.length) return null
  const idSet = new Set(nodeIds)
  // 对称轴向尺寸（TB 横向为对称轴，LR 纵向）；层轴向为另一维
  const ext = (id: string) => (rankdir === 'TB' ? sizeOf(id).w : sizeOf(id).h)
  const dep = (id: string) => (rankdir === 'TB' ? sizeOf(id).h : sizeOf(id).w)

  /* --- Pass 1: 分层（最长路径 + cross 归同层 + 孤立回填） --- */
  const rank = new Map<string, number>()
  const incoming = new Map<string, number>()
  const validEdges: { source: string; target: string }[] = []
  dirEdges.forEach((e) => {
    if (!idSet.has(e.source) || !idSet.has(e.target) || e.source === e.target) return
    validEdges.push(e)
    incoming.set(e.target, (incoming.get(e.target) ?? 0) + 1)
  })
  // 根 = 无入边；无根（全环）→ 取首节点
  let roots = nodeIds.filter((id) => !incoming.has(id))
  if (!roots.length) roots = [nodeIds[0]]
  roots.forEach((id) => rank.set(id, 0))
  // 松弛求最长路径（DAG 收敛；含环也只前进不发散）
  for (let iter = 0; iter <= nodeIds.length; iter++) {
    let changed = false
    for (const e of validEdges) {
      const rs = rank.get(e.source)
      if (rs === undefined) continue
      const rt = rank.get(e.target)
      if (rt === undefined || rs + 1 > rt) {
        rank.set(e.target, rs + 1)
        changed = true
      }
    }
    if (!changed) break
  }
  // cross 对归同层
  const crossPartner = new Map<string, string>()
  const validPairs: [string, string][] = []
  crossPairs.forEach(([a, b]) => {
    if (!idSet.has(a) || !idSet.has(b) || a === b) return
    validPairs.push([a, b])
    crossPartner.set(a, b)
    crossPartner.set(b, a)
    const ra = rank.get(a)
    const rb = rank.get(b)
    let r: number
    if (ra !== undefined && rb !== undefined) r = Math.max(ra, rb)
    else if (ra !== undefined) r = ra
    else if (rb !== undefined) r = rb
    else r = -1
    if (r >= 0) {
      rank.set(a, r)
      rank.set(b, r)
    }
  })
  // 双孤立 cross 对
  let maxRank = 0
  rank.forEach((v) => (maxRank = Math.max(maxRank, v)))
  validPairs.forEach(([a, b]) => {
    if (!rank.has(a) && !rank.has(b)) {
      rank.set(a, maxRank + 1)
      rank.set(b, maxRank + 1)
    }
  })
  // 仍无 rank（纯孤立）→ maxRank+1（修缺陷3：不再挤 row0）
  maxRank = 0
  rank.forEach((v) => (maxRank = Math.max(maxRank, v)))
  nodeIds.forEach((id) => {
    if (!rank.has(id)) rank.set(id, maxRank + 1)
  })

  /* --- Pass 2: 层内排序（重心法减交叉 + cross 对相邻） --- */
  const layers = new Map<number, string[]>()
  nodeIds.forEach((id) => {
    const r = rank.get(id) as number
    layers.set(r, [...(layers.get(r) ?? []), id])
  })
  const sortedRanks = [...layers.keys()].sort((a, b) => a - b)
  sortedRanks.forEach((r) => layers.get(r)!.sort()) // 确定性初值
  const posInLayer = new Map<string, number>()
  const updPos = () => {
    posInLayer.clear()
    sortedRanks.forEach((r) => layers.get(r)!.forEach((id, i) => posInLayer.set(id, i)))
  }
  updPos()
  const allChildren = new Map<string, string[]>()
  const allParents = new Map<string, string[]>()
  validEdges.forEach((e) => {
    allChildren.set(e.source, [...(allChildren.get(e.source) ?? []), e.target])
    allParents.set(e.target, [...(allParents.get(e.target) ?? []), e.source])
  })
  const bary = (id: string, refs: string[]): number => {
    const cur = posInLayer.get(id) ?? 0
    const valid = refs.filter((x) => idSet.has(x) && posInLayer.has(x))
    if (!valid.length) return cur
    return valid.reduce((s, x) => s + (posInLayer.get(x) ?? 0), 0) / valid.length
  }
  for (let sweep = 0; sweep < 3; sweep++) {
    for (let li = 1; li < sortedRanks.length; li++) {
      const layer = layers.get(sortedRanks[li])!
      layer.sort((a, b) => bary(a, allParents.get(a) ?? []) - bary(b, allParents.get(b) ?? []))
      updPos()
    }
    for (let li = sortedRanks.length - 2; li >= 0; li--) {
      const layer = layers.get(sortedRanks[li])!
      layer.sort((a, b) => bary(a, allChildren.get(a) ?? []) - bary(b, allChildren.get(b) ?? []))
      updPos()
    }
  }
  // cross 对相邻
  sortedRanks.forEach((r) => {
    const layer = layers.get(r)!
    validPairs.forEach(([a, b]) => {
      if (rank.get(a) !== r || rank.get(b) !== r) return
      const ia = layer.indexOf(a)
      const ib = layer.indexOf(b)
      if (ia < 0 || ib < 0 || Math.abs(ia - ib) === 1) return
      layer.splice(ib, 1)
      layer.splice(layer.indexOf(a) + 1, 0, b)
      updPos()
    })
  })

  /* --- Pass 3: 构树（主父）+ cross 对合成超节点 --- */
  const primaryParent = new Map<string, string>()
  validEdges.forEach((e) => {
    const cur = primaryParent.get(e.target)
    if (cur === undefined) primaryParent.set(e.target, e.source)
    else if ((rank.get(e.source) ?? 0) < (rank.get(cur) ?? 0)) primaryParent.set(e.target, e.source)
  })
  const childrenByPrim = new Map<string, string[]>()
  primaryParent.forEach((p, c) => childrenByPrim.set(p, [...(childrenByPrim.get(p) ?? []), c]))
  // 超节点：rep 与 partner；partner 被吸收
  const supernode = new Map<string, string>() // rep -> partner
  const absorbed = new Set<string>()
  const pairSeen = new Set<string>()
  sortedRanks.forEach((r) => {
    const layer = layers.get(r)!
    validPairs.forEach(([a, b]) => {
      if (rank.get(a) !== r || rank.get(b) !== r) return
      const key = a < b ? `${a}|${b}` : `${b}|${a}`
      if (pairSeen.has(key)) return
      pairSeen.add(key)
      const rep = layer.indexOf(a) <= layer.indexOf(b) ? a : b
      supernode.set(rep, rep === a ? b : a)
      absorbed.add(rep === a ? b : a)
    })
  })
  const active = nodeIds.filter((id) => !absorbed.has(id))
  const activeSet = new Set(active)
  // 子节点集合（超节点取 rep+partner 并集，去重、仅 active、排除自身）
  const compositeChildren = (id: string): string[] => {
    let raw = childrenByPrim.get(id) ?? []
    const partner = supernode.get(id)
    if (partner) raw = [...raw, ...(childrenByPrim.get(partner) ?? [])]
    return [...new Set(raw)].filter((c) => activeSet.has(c) && c !== id)
  }
  // 有效父（跳过 absorbed 祖先）
  const findActiveAncestor = (id: string): string | null => {
    let cur = primaryParent.get(id)
    while (cur && absorbed.has(cur)) cur = primaryParent.get(cur)
    return cur && activeSet.has(cur) ? cur : null
  }
  const activeRoots = active.filter((id) => !findActiveAncestor(id))

  /* --- Pass 4: 中轴对称摆放（subtree 宽度 + 后序 place） --- */
  const widthOf = new Map<string, number>()
  const calcWidth = (id: string): number => {
    if (widthOf.has(id)) return widthOf.get(id) as number
    const partner = supernode.get(id)
    const baseW = partner ? ext(id) + PAIRSEP + ext(partner) : ext(id)
    const chs = compositeChildren(id)
    let w = baseW
    if (chs.length) {
      const total = chs.reduce((s, c) => s + calcWidth(c), 0) + 48 * (chs.length - 1)
      w = Math.max(baseW, total)
    }
    widthOf.set(id, w)
    return w
  }
  active.forEach(calcWidth)
  const symCenter = new Map<string, number>()
  const place = (id: string, centerX: number) => {
    const partner = supernode.get(id)
    if (partner) {
      const blockW = ext(id) + PAIRSEP + ext(partner)
      symCenter.set(id, centerX - blockW / 2 + ext(id) / 2)
      symCenter.set(partner, centerX + blockW / 2 - ext(partner) / 2)
    } else {
      symCenter.set(id, centerX)
    }
    const chs = compositeChildren(id)
    if (!chs.length) return
    const childW = chs.map((c) => widthOf.get(c) as number)
    const total = childW.reduce((s, x) => s + x, 0) + 48 * (chs.length - 1)
    let cursor = centerX - total / 2
    chs.forEach((c, i) => {
      place(c, cursor + childW[i] / 2)
      cursor += childW[i] + 48
    })
  }
  if (activeRoots.length) {
    const rootW = activeRoots.map((r) => widthOf.get(r) as number)
    const total = rootW.reduce((s, x) => s + x, 0) + 48 * (activeRoots.length - 1)
    let cursor = -total / 2
    activeRoots.forEach((r, i) => {
      place(r, cursor + rootW[i] / 2)
      cursor += rootW[i] + 48
    })
  }

  /* --- Pass 5: 层轴坐标（同层共中心线）+ 映射为中心点 + 归一化 --- */
  const layerCoord = new Map<number, number>()
  let cursorY = 0
  sortedRanks.forEach((r) => {
    const maxD = Math.max(...layers.get(r)!.map(dep))
    layerCoord.set(r, cursorY + maxD / 2)
    cursorY += maxD + 56
  })
  // 返回中心点（匹配 dagre 契约：autoLayout 摆放时用 p.x - w/2 转左上角）
  const centers = new Map<string, { x: number; y: number }>()
  nodeIds.forEach((id) => {
    const s = symCenter.get(id) ?? 0
    const l = layerCoord.get(rank.get(id) as number) ?? 0
    if (rankdir === 'TB') centers.set(id, { x: s, y: l })
    else centers.set(id, { x: l, y: s })
  })
  // 归一化：使整体包围盒左上角为 (0,0)
  let minX = Infinity
  let minY = Infinity
  centers.forEach((p, id) => {
    const sz = sizeOf(id)
    minX = Math.min(minX, p.x - sz.w / 2)
    minY = Math.min(minY, p.y - sz.h / 2)
  })
  if (!isFinite(minX) || !isFinite(minY)) return null
  const pos = new Map<string, { x: number; y: number }>()
  centers.forEach((p, id) => pos.set(id, { x: p.x - minX, y: p.y - minY }))
  return pos
}

export async function autoLayout(graph: Graph, rankdir: LayoutDir, opts?: { skipBatch?: boolean }): Promise<AutoLayoutResult> {
  /* legend 节点不参与布局（位置由 syncLegend 管理） */
  const all = graph.getNodes().filter((n) => n.shape !== 'legend')
  if (all.length < 2) return { ranker: 'none', fallback: false }
  const ids = new Set(all.map((n) => n.id))

  /* 1) 自底向上预计算区域尺寸（子节点 bbox + 边距 + 标题留白），嵌套区域用已拟合尺寸 */
  const regions = all.filter((n) => n.shape === TOPO_REGION)
  const byDepthDesc = [...regions].sort((a, b) => nodeDepth(b) - nodeDepth(a))
  const fitSize = new Map<string, { w: number; h: number }>()
  byDepthDesc.forEach((r) => {
    const kids = all.filter((n) => n.getParentId() === r.id)
    if (!kids.length) {
      const s = r.getSize()
      fitSize.set(r.id, { w: s.width, h: s.height })
      return
    }
    const ms = kids.map((k) => {
      const fs = fitSize.get(k.id)
      const b = fs
        ? { x: k.getPosition().x, y: k.getPosition().y, width: fs.w, height: fs.h }
        : (() => {
            const bb = k.getBBox()
            return { x: bb.x, y: bb.y, width: bb.width, height: bb.height }
          })()
      return { box: b, overhang: k.shape === TOPO_REGION ? 0 : labelOverhang(b.height) }
    })
    const fit = regionSizeOf(ms)
    fitSize.set(r.id, fit ?? { w: REGION_MIN_W, h: REGION_MIN_H })
  })
  const sizeOf = (n: Node): { w: number; h: number } => {
    const fs = fitSize.get(n.id)
    if (fs) return fs
    const s = n.getSize()
    return { w: s.width, h: s.height }
  }

  /* 2) 边按真实端点/布局方向传入；交叉轴边（cross，如 HA/堆叠对的左右向）不参与分层——
      否则 network-simplex 会把对端推进下一 rank，破坏同层对齐。cross 对单独收集给对称布局。 */
  const seen = new Set<string>()
  const edges: { id: string; source: string; target: string }[] = []
  const crossPairs: [string, string][] = []
  graph.getEdges().forEach((e) => {
    const data = e.getData() as { layout?: { src: string; tgt: string }; cross?: boolean } | undefined
    const srcCell = e.getSourceCell()?.id
    const tgtCell = e.getTargetCell()?.id
    if (data?.cross) {
      // cross 对：记录真实两端（供对称布局同层约束）
      if (srcCell && tgtCell && ids.has(srcCell) && ids.has(tgtCell) && srcCell !== tgtCell) {
        crossPairs.push([srcCell, tgtCell])
      }
      return
    }
    const ld = data?.layout
    const src = ld?.src || srcCell
    const tgt = ld?.tgt || tgtCell
    if (!src || !tgt || !ids.has(src) || !ids.has(tgt) || src === tgt) return
    const key = `${src}->${tgt}`
    if (seen.has(key)) return
    seen.add(key)
    edges.push({ id: key, source: src, target: tgt })
  })

  /* 3) ranker 降级链：network-simplex 对含区块级链路的 compound 图存在崩溃缺陷 */
  const sizeMap = new Map<string, { w: number; h: number }>()
  const inputNodes = all.map((n) => {
    const sz = sizeOf(n)
    sizeMap.set(n.id, sz)
    return {
      id: n.id,
      parentId: n.getParentId() || undefined,
      width: sz.w,
      height: sz.h
    }
  })
  let pos: Map<string, { x: number; y: number }> | null = null
  let usedRanker = ''

  /* 3a) 自研中轴对称布局优先：顶层节点（含区域作块）+ 区域子图递归。
        cross 对转同层约束（修缺陷2/3）；失败/异常 → 回落 dagre。 */
  try {
    const anc = new Map<string, string>()
    all.forEach((n) => {
      let p: Node | null = n
      while (p.getParent()) p = p.getParent() as Node
      anc.set(n.id, p.id)
    })
    const hoist = (id: string) => anc.get(id) ?? id
    const topNodes = all.filter((n) => !n.getParent())
    const topIds = topNodes.map((n) => n.id)
    const sizeOfId = (id: string) => sizeMap.get(id) ?? { w: 76, h: 76 }
    const topEdges: { source: string; target: string }[] = []
    const eSeen = new Set<string>()
    edges.forEach((e) => {
      const s = hoist(e.source)
      const t = hoist(e.target)
      if (s === t) return
      const key = `${s}->${t}`
      if (eSeen.has(key)) return
      eSeen.add(key)
      topEdges.push({ source: s, target: t })
    })
    const topPairs: [string, string][] = []
    const pSeen = new Set<string>()
    crossPairs.forEach(([a, b]) => {
      const s = hoist(a)
      const t = hoist(b)
      if (s === t) return
      const key = s < t ? `${s}|${t}` : `${t}|${s}`
      if (pSeen.has(key)) return
      pSeen.add(key)
      topPairs.push([s, t])
    })
    const res = symmetricLayout(topIds, sizeOfId, topEdges, topPairs, rankdir)
    if (res) {
      const combined = new Map(res)
      // 区域子图：对每个区域，取其直接子节点 + 内部边/对，递归对称布局后按区域位偏移
      regions.forEach((region) => {
        const kids = all.filter((n) => n.getParentId() === region.id)
        if (kids.length < 2) return
        const kidIds = kids.map((k) => k.id)
        const kidSet = new Set(kidIds)
        const kEdges: { source: string; target: string }[] = []
        const keSeen = new Set<string>()
        edges.forEach((e) => {
          if (!kidSet.has(e.source) || !kidSet.has(e.target)) return
          const key = `${e.source}->${e.target}`
          if (keSeen.has(key)) return
          keSeen.add(key)
          kEdges.push({ source: e.source, target: e.target })
        })
        const kPairs: [string, string][] = []
        crossPairs.forEach(([a, b]) => {
          if (kidSet.has(a) && kidSet.has(b)) kPairs.push([a, b])
        })
        const kRes = symmetricLayout(kidIds, sizeOfId, kEdges, kPairs, rankdir)
        if (!kRes) return
        // rp/rsz 均为中心点/尺寸；kRes 归一化使子图包围盒左上角为 0。
        // 偏移到「区域左上角 + 内边距」：dx/dy 即子图左上角应落点（因 min(p.x-w/2)=0）。
        const rp = combined.get(region.id) ?? { x: 0, y: 0 }
        const rsz = sizeOfId(region.id)
        const dx = rp.x - rsz.w / 2 + REGION_PAD_X
        const dy = rp.y - rsz.h / 2 + REGION_TITLE_BAR
        kRes.forEach((p, id) => combined.set(id, { x: p.x + dx, y: p.y + dy }))
      })
      pos = combined
      usedRanker = 'symmetric'
    }
  } catch {
    pos = null
    usedRanker = ''
  }

  /* 3b) dagre ranker 降级链（仅对称布局失败时）：network-simplex 对含区块级链路的 compound 图存在崩溃缺陷 */
  if (!pos) for (const ranker of ['network-simplex', 'tight-tree', 'longest-path'] as const) {
    const layout = new DagreLayout({
      rankdir,
      nodesep: 48,
      ranksep: 56,
      ranker: ranker as never,
      compound: true,
      nodeSize: (raw: { id: string }) => {
        const s = sizeMap.get(String(raw.id))
        return s ? [s.w, s.h] : [0, 0]
      }
    } as never)
    try {
      await layout.execute({ nodes: inputNodes, edges } as never)
      const tmp = new Map<string, { x: number; y: number }>()
      layout.forEachNode((n) => tmp.set(String(n.id), { x: n.x, y: n.y }))
      pos = tmp
      layout.destroy()
      usedRanker = ranker
      break
    } catch {
      try {
        layout.destroy()
      } catch {
        /* ignore */
      }
    }
  }

  suppressRefit++
  const ownBatch = !opts?.skipBatch
  if (ownBatch) graph.startBatch('auto-layout')
  try {
    if (pos) {
      const posMap: Map<string, { x: number; y: number }> = pos
      /* 4) 归一化到 (40,40) 起点 */
      let minX = Infinity
      let minY = Infinity
      all.forEach((n) => {
        const p = posMap.get(n.id)
        if (!p) return
        const sz = sizeMap.get(n.id)
        if (!sz) return
        minX = Math.min(minX, p.x - sz.w / 2)
        minY = Math.min(minY, p.y - sz.h / 2)
      })
      if (isFinite(minX) && isFinite(minY)) {
        const offX = 40 - minX
        const offY = 40 - minY
        const place = (n: Node) => {
          const p = posMap.get(String(n.id))
          if (!p) return
          const sz = sizeMap.get(String(n.id))
          if (!sz) return
          n.setPosition(Math.round(p.x - sz.w / 2 + offX), Math.round(p.y - sz.h / 2 + offY))
        }
        ;[...regions].sort((a, b) => nodeDepth(a) - nodeDepth(b)).forEach(place)
        all.filter((n) => n.shape !== TOPO_REGION).forEach(place)
      }
      refitAllRegions(graph)
    } else {
      /* 5) 全部 ranker 失败 → 应急网格（保证不堆叠） */
      const topNodes = all.filter((n) => !n.getParent())
      gridPack(graph, topNodes, edges, rankdir)
      refitAllRegions(graph)
    }
    /* 端点绑定：布局后所有设备边按声明 dir 重绑固定四端点 */
    bindAllEdgePorts(graph)
    /* 图例跟随新内容锚点（左上/右上） */
    repositionLegend(graph)
  } finally {
    if (ownBatch) graph.stopBatch('auto-layout')
    suppressRefit--
  }

  return {
    ranker: pos ? usedRanker : 'grid',
    fallback: !pos || (usedRanker !== 'network-simplex' && usedRanker !== 'symmetric')
  }
}
