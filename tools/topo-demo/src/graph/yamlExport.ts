import type { Graph, Node, Edge } from '@antv/x6'
import { TOPO_NODE, TOPO_REGION, TOPO_TEXT } from './node'
import { iconIdBySrc } from './iconCatalog'
import {
  builtinTypeNames,
  getActivePreset,
  resolveColor,
  COLOR_NAME,
  type LinkTypePreset
} from './linkTypes'
import {
  readEdgeShape,
  readLineStyle,
  readEndMarkers,
  readEdgeColor,
  readEdgeWidth,
  edgeLabelText,
  isDoubleEdge
} from './setup'

/* ---------------- 导出结果 ---------------- */

export interface YamlExportResult {
  yaml: string
  skipped: number // 被跳过的元素数（文本/图例/悬空边/自环）
}

/* 端口 id → dir（与 DIR_PORTS 互逆；声明端端口决定 dir） */
const PORT_DIR: Record<string, string> = { bottom: '下', top: '上', right: '右', left: '左' }

/* hex → 色板名（导出时优先写名字，非色板则写 hex） */
const HEX_TO_NAME = new Map<string, string>(
  Object.entries(COLOR_NAME).map(([name, hex]) => [hex.toLowerCase(), name])
)
function colorOut(hex: string): string {
  return HEX_TO_NAME.get(hex.toLowerCase()) ?? hex
}

function isLegend(node: Node): boolean {
  return node.shape === 'legend' || !!(node.getData() as { legend?: boolean } | undefined)?.legend
}

/** 稳定 yid：优先 data.yid；否则按出现顺序生成 devN / blockN（保证可回写） */
function buildYidMap(graph: Graph): Map<string, string> {
  const map = new Map<string, string>()
  const used = new Set<string>()
  let devN = 0
  let blockN = 0
  const assign = (node: Node, prefix: 'dev' | 'block') => {
    const yid = String((node.getData() as { yid?: string } | undefined)?.yid ?? '')
    if (yid && !used.has(yid)) {
      used.add(yid)
      map.set(node.id, yid)
      return
    }
    let gen = ''
    do {
      gen = prefix === 'dev' ? `dev${++devN}` : `block${++blockN}`
    } while (used.has(gen))
    used.add(gen)
    map.set(node.id, gen)
  }
  graph.getNodes().forEach((n) => {
    if (n.shape === TOPO_REGION) assign(n, 'block')
    else if (n.shape === TOPO_NODE && !isLegend(n)) assign(n, 'dev')
  })
  return map
}

/* ---------------- graph → 语义 YAML（无坐标） ---------------- */

export function graphToYaml(graph: Graph): YamlExportResult {
  const yidOf = buildYidMap(graph)
  const builtin = new Set(builtinTypeNames())
  let skipped = 0

  /* --- blocks --- */
  const blocks = graph
    .getNodes()
    .filter((n) => n.shape === TOPO_REGION)
    .map((n) => {
      const d = (n.getData() ?? {}) as { label?: string; color?: string }
      return {
        id: yidOf.get(n.id) as string,
        label: d.label ?? '区域',
        color: d.color ?? ''
      }
    })

  /* --- devices --- */
  const devNodes = graph
    .getNodes()
    .filter((n) => n.shape === TOPO_NODE && !isLegend(n))
  const devices = devNodes.map((n) => {
    const d = (n.getData() ?? {}) as { icon?: string; label?: string }
    const parentId = n.getParentId()
    const block = parentId ? yidOf.get(parentId) : undefined
    return {
      id: yidOf.get(n.id) as string,
      icon: (d.icon && iconIdBySrc(d.icon)) || '',
      name: d.label ?? '',
      block: block || ''
    }
  })
  // 文本节点计入 skipped（YAML 无法表达）
  skipped += graph.getNodes().filter((n) => n.shape === TOPO_TEXT).length
  // 图例自动重建，不计 skipped

  /* --- links（单端声明 + 无序对去重） --- */
  const seenPairs = new Set<string>()
  const customTypes = new Map<string, LinkTypePreset>()
  interface OutLink {
    from: string
    to: string
    type: string
    dir: string
    label: string
    ov: Record<string, string | number>
  }
  const linksByDevice = new Map<string, OutLink[]>()

  const pairKey = (a: string, b: string) => (a < b ? `${a}\u0000${b}` : `${b}\u0000${a}`)

  graph.getEdges().forEach((edge) => {
    const src = edge.getSourceCell()
    const tgt = edge.getTargetCell()
    // 悬空边（终端是坐标点）/ 非节点 → 跳过
    if (!src || !tgt || !src.isNode() || !tgt.isNode()) {
      skipped++
      return
    }
    const srcY = yidOf.get(src.id)
    const tgtY = yidOf.get(tgt.id)
    if (!srcY || !tgtY) {
      skipped++
      return
    }
    if (srcY === tgtY) {
      skipped++ // 自环
      return
    }
    const pk = pairKey(srcY, tgtY)
    if (seenPairs.has(pk)) return // 已从另一端声明
    seenPairs.add(pk)

    const data = (edge.getData() ?? {}) as {
      linkType?: string
      a?: string
      b?: string
      dir?: string
    }

    // 声明端：优先 data.a 命中任一端，否则用 source 端
    let fromY = srcY
    let toY = tgtY
    if (data.a && data.a === tgtY && data.a !== srcY) {
      fromY = tgtY
      toY = srcY
    }

    // dir：data.dir 优先；空则从声明端端口反推
    let dir = data.dir || ''
    if (!dir) {
      const port = fromY === srcY ? edge.getSourcePortId() : edge.getTargetPortId()
      dir = (port && PORT_DIR[port]) || ''
    }

    // 类型 + overrides（对比预设）
    const type = data.linkType || ''
    const preset = type ? getActivePreset(type) : null
    const ov: Record<string, string | number> = {}
    const actualColor = readEdgeColor(edge)
    const actualWidth = readEdgeWidth(edge)
    const actualStyle = readLineStyle(edge)

    if (!preset) {
      // 无预设（手动边/未知类型）→ 完整写出样式
      ov.color = colorOut(actualColor)
      ov.width = actualWidth
      if (actualStyle !== 'solid') ov.style = actualStyle
    } else {
      const expColor = resolveColor(preset.color)
      if (expColor && actualColor.toLowerCase() !== expColor.toLowerCase()) ov.color = colorOut(actualColor)
      if (actualWidth !== preset.width) ov.width = actualWidth
      if (actualStyle !== preset.style) ov.style = actualStyle
    }

    // 箭头（仅导出可往返的前向/双向情形）
    const { head, tail } = readEndMarkers(edge)
    if (tail !== 'none') {
      ov.marker = tail
      ov.arrow = head !== 'none' ? 'both' : 'one'
    }

    // 形状：默认是 straight，只有偏离（rounded）才写；smooth/jump 无法用 YAML 表达 → 丢弃
    const shape = readEdgeShape(edge)
    if (shape === 'rounded') ov.shape = 'rounded'

    // 自定义链路类型收集（非内置 → 写进 linkTypes:）
    if (type && !builtin.has(type) && !customTypes.has(type)) {
      customTypes.set(type, {
        color: colorOut(actualColor),
        style: actualStyle,
        width: actualWidth,
        double: isDoubleEdge(edge) || undefined,
        desc: `自定义类型 ${type}`
      })
    }

    const label = edgeLabelText(edge)
    const out: OutLink = { from: fromY, to: toY, type, dir, label, ov }
    const arr = linksByDevice.get(fromY) ?? []
    arr.push(out)
    linksByDevice.set(fromY, arr)
  })

  /* --- 序列化 --- */
  const q = (s: string) => (/:|[#{}[\],&*?|>!%@`]/.test(s) || s.includes(' ') ? `"${s.replace(/"/g, '\\"')}"` : s)
  const L: string[] = []
  L.push('# 由拓扑编辑器导出（语义结构，不含坐标）')
  L.push('# 重要：请保持设备 id 稳定（增删改时尽量沿用原 id）——坐标与后续修改都按 id 关联')
  L.push('# 导入后自动布局；如需保留人工微调坐标，请配套导入本文件的 layout 段或 topology.layout.yaml')
  L.push('iconStyle: 简约')
  L.push('')

  if (blocks.length) {
    L.push('blocks:')
    blocks.forEach((b) => {
      const parts = [`id: ${q(b.id)}`]
      if (b.label && b.label !== b.id) parts.push(`label: ${q(b.label)}`)
      if (b.color) parts.push(`color: ${q(b.color)}`)
      L.push(`  - { ${parts.join(', ')} }`)
    })
    L.push('')
  }

  L.push('devices:')
  devices.forEach((d) => {
    L.push(`  - id: ${q(d.id)}`)
    if (d.icon) L.push(`    icon: ${q(d.icon)}`)
    if (d.name) L.push(`    name: ${q(d.name)}`)
    if (d.block) L.push(`    block: ${q(d.block)}`)
    const ls = linksByDevice.get(d.id)
    if (ls && ls.length) {
      L.push(`    links:`)
      ls.forEach((lk) => {
        const parts: string[] = [`to: ${q(lk.to)}`]
        if (lk.type) parts.push(`type: ${q(lk.type)}`)
        if (lk.dir) parts.push(`dir: ${lk.dir}`)
        if (lk.label) parts.push(`label: ${q(lk.label)}`)
        // overrides（color/width/style/arrow/marker/shape）
        ;['color', 'width', 'style', 'arrow', 'marker', 'shape'].forEach((k) => {
          if (lk.ov[k] !== undefined) parts.push(`${k}: ${k === 'style' || k === 'shape' || k === 'arrow' || k === 'marker' || k === 'color' ? q(String(lk.ov[k])) : lk.ov[k]}`)
        })
        L.push(`      - { ${parts.join(', ')} }`)
      })
    }
  })

  if (customTypes.size) {
    L.push('')
    L.push('linkTypes:')
    customTypes.forEach((p, name) => {
      const parts = [`color: ${p.color}`, `style: ${p.style}`, `width: ${p.width}`]
      if (p.double) parts.push('double: true')
      if (p.desc) parts.push(`desc: ${q(p.desc)}`)
      L.push(`  ${q(name)}: { ${parts.join(', ')} }`)
    })
  }

  return { yaml: L.join('\n'), skipped }
}

/* ---------------- graph → 布局 sidecar（yid→坐标） ---------------- */

export function graphToLayout(graph: Graph): string {
  const yidOf = buildYidMap(graph)
  const L: string[] = ['# 布局坐标覆盖（工具生成；重导入时按 yid 套在语义布局之上）', 'layout:']
  graph
    .getNodes()
    .filter((n) => n.shape === TOPO_NODE && !isLegend(n))
    .forEach((n) => {
      const yid = yidOf.get(n.id)
      if (!yid) return
      const b = n.getBBox()
      L.push(`  ${yid}: { x: ${Math.round(b.x)}, y: ${Math.round(b.y)} }`)
    })
  return L.join('\n')
}

/* ---------------- graph → 合并单文件（语义 + layout 段） ----------------
   用途：人自己存档 / 一次粘贴+「应用」完整还原（含人工微调坐标）。不给 AI 用。 */
export function graphToCombined(graph: Graph): YamlExportResult {
  const { yaml, skipped } = graphToYaml(graph)
  // 去掉语义头部里"配套导入 layout 段"的提示（本文件已自带 layout 段）
  const body = yaml
    .split('\n')
    .filter((l) => !l.includes('请配套导入本文件的 layout 段'))
    .join('\n')
  const layout = graphToLayout(graph)
    .split('\n')
    .filter((l) => !l.startsWith('#')) // 去 sidecar 自己的头注释
    .join('\n')
  return { yaml: `${body.trimEnd()}\n\n${layout}\n`, skipped }
}
