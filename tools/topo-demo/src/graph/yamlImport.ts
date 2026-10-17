import { Graph, Node } from '@antv/x6'
import { parseDocument, LineCounter } from 'yaml'
import { hasIcon } from './iconArt'
import {
  resolveIconForStyle,
  normalizeIconStyle,
  suggestCatalogId,
  inCatalog,
  type IconStyle
} from './iconCatalog'
import { createTopoNodeMeta, createRegionMeta, applyRegionColor } from './node'
import { refitRegion, refitAllRegions, autoLayout, cancelRegionRefit, type AutoLayoutResult } from './editing'
import { syncLegend } from './legend'
import {
  createEdgeCell,
  createDoubleEdge,
  applyPresetBase,
  applyEdgeShape,
  applyLineStyle,
  applyEndMarkers,
  applyEdgeLabel,
  bindEdgePorts,
  type MarkerName,
  type EdgeShape,
  type LineStyle
} from './setup'
import {
  mergeLinkTypes,
  lookupLinkType,
  resolveColor,
  builtinTypeNames,
  setActiveMergedTypes,
  type LinkTypePreset
} from './linkTypes'

export interface YamlError {
  line: number | null
  message: string
}

export interface YamlResult {
  ok: boolean
  errors: YamlError[]
  warnings: YamlError[]
  devices: number
  edges: number
  blocks: number
  axis: '上下' | '左右'
  axisSource: '显式' | '多数决' | '默认'
  crossAxis: number
  needsLayout: boolean
}

/* ---------- 内部 IR ---------- */

interface DeviceSpec {
  id: string
  src: string
  w: number
  h: number
  label: string
  x?: number
  y?: number
  block?: string
}

interface BlockSpec {
  label?: string
  color?: string // hex
  line: number | null
}

interface LinkOverrides {
  style?: LineStyle
  color?: string // hex
  width?: number
  arrow?: 'none' | 'one' | 'both'
  marker?: MarkerName
  shape?: EdgeShape
  label?: string
}

interface LinkIR {
  a: string
  b: string
  line: number | null
  forward: boolean // 布局方向：a 在前
  explicitDir?: string // 设备 links 必填；legacy edges 无
  crossAxis: boolean
  typeCanonical?: string
  preset?: LinkTypePreset
  overrides: LinkOverrides
}

interface Parsed {
  ok: boolean
  errors: YamlError[]
  warnings: YamlError[]
  devices: DeviceSpec[]
  blocks: Map<string, BlockSpec>
  links: LinkIR[]
  axis: '上下' | '左右'
  axisSource: '显式' | '多数决' | '默认'
  crossAxis: number
  needsLayout: boolean
  layout: Record<string, unknown> | null // 同文件顶层 layout:（yid→坐标覆盖）
}

const ID_RE = /^[a-zA-Z0-9_-]{1,48}$/
const DIR_VALUES = new Set(['上', '下', '左', '右'])
const STYLES = new Set(['solid', 'dashed', 'dotted'])
const ARROWS = new Set(['none', 'one', 'both'])
const MARKERS = new Set(['classic', 'block', 'diamond', 'cross'])
const REL_SHAPES = new Set(['straight', 'rounded'])

function pairKey(a: string, b: string): string {
  return a < b ? `${a}\u0000${b}` : `${b}\u0000${a}`
}

function fail(errors: YamlError[], warnings: YamlError[] = []): Parsed {
  return {
    ok: false,
    errors,
    warnings,
    devices: [],
    blocks: new Map(),
    links: [],
    axis: '上下',
    axisSource: '默认',
    crossAxis: 0,
    needsLayout: false,
    layout: null
  }
}

/* ---------- 解析 + 校验（含去重、轴判定） ---------- */

function parseAndValidate(text: string): Parsed {
  const errors: YamlError[] = []
  const warnings: YamlError[] = []
  const lineCounter = new LineCounter()
  let doc
  try {
    doc = parseDocument(text, { lineCounter })
  } catch (e) {
    return fail([{ line: null, message: `YAML 语法错误：${(e as Error).message}` }])
  }
  if (doc.errors.length) {
    doc.errors.forEach((err) =>
      errors.push({ line: err.pos ? lineCounter.linePos(err.pos[0]).line : null, message: err.message })
    )
    return fail(errors)
  }

  const lineOf = (path: (string | number)[]): number | null => {
    try {
      const ynode = doc.getIn(path, true) as { range?: [number, number, number] } | null
      return ynode?.range ? lineCounter.linePos(ynode.range[0]).line : null
    } catch {
      return null
    }
  }

  const data = doc.toJS() as Record<string, unknown>

  /* iconStyle */
  let iconStyle: IconStyle = '简约'
  if (data.iconStyle !== undefined && data.iconStyle !== null) {
    const st = normalizeIconStyle(data.iconStyle)
    if (!st) {
      errors.push({ line: lineOf(['iconStyle']), message: `iconStyle 非法："${String(data.iconStyle)}"（可选 简约|写实）` })
    } else iconStyle = st
  }

  /* axis */
  let axisExplicit: '上下' | '左右' | null = null
  if (data.axis !== undefined && data.axis !== null) {
    const a = String(data.axis).trim()
    if (a === '上下' || a === 'TB') axisExplicit = '上下'
    else if (a === '左右' || a === 'LR') axisExplicit = '左右'
    else errors.push({ line: lineOf(['axis']), message: `axis 非法："${a}"（可选 上下|左右）` })
  }

  /* linkTypes 字段级合并 */
  const { merged, errors: ltErrors } = mergeLinkTypes(data.linkTypes, (i) => lineOf(['linkTypes', i]))
  ltErrors.forEach((m) => errors.push({ line: null, message: m }))
  setActiveMergedTypes(merged)

  /* ---- 区块表 ---- */
  const blocks = new Map<string, BlockSpec>()
  if (data.blocks !== undefined && data.blocks !== null) {
    if (!Array.isArray(data.blocks)) {
      errors.push({ line: 1, message: 'blocks 必须是列表' })
    } else {
      ;(data.blocks as unknown[]).forEach((raw, i) => {
        const line = lineOf(['blocks', i])
        const item = raw as Record<string, unknown> | null
        if (!item || typeof item !== 'object' || Array.isArray(item)) {
          errors.push({ line, message: `blocks[${i}] 不是有效对象` })
          return
        }
        const id = typeof item.id === 'string' ? item.id.trim() : ''
        if (!id) {
          errors.push({ line, message: `blocks[${i}].id 缺失或不是字符串` })
          return
        }
        const spec: BlockSpec = { line }
        if (item.label !== undefined && item.label !== null) spec.label = String(item.label)
        if (item.color !== undefined && item.color !== null) {
          const hex = resolveColor(String(item.color).trim())
          if (!hex) {
            errors.push({ line, message: `blocks[${i}].color 非法："${String(item.color)}"（色板名或 #hex）` })
            return
          }
          spec.color = hex
        }
        blocks.set(id, spec)
      })
    }
  }

  /* ---- 设备（兼容旧 nodes 写法） ---- */
  const devices: DeviceSpec[] = []
  const ids = new Set<string>()
  const devKey: 'devices' | 'nodes' | null = Array.isArray(data.devices)
    ? 'devices'
    : Array.isArray(data.nodes)
      ? 'nodes'
      : null
  const nodeArr = devKey ? (data[devKey] as unknown[]) : null

  if (!nodeArr || nodeArr.length === 0) {
    errors.push({ line: 1, message: '缺少 devices 列表（至少一个设备；兼容旧写法 nodes）' })
  } else {
    let anyCoord: number | null = null
    let anyMissing = false
    nodeArr.forEach((raw, i) => {
      const line = lineOf([devKey as string, i])
      const item = raw as Record<string, unknown> | null
      if (!item || typeof item !== 'object' || Array.isArray(item)) {
        errors.push({ line, message: `devices[${i}] 不是有效对象` })
        return
      }
      const id = typeof item.id === 'string' ? item.id.trim() : ''
      if (!id) {
        errors.push({ line, message: `devices[${i}].id 缺失或不是字符串` })
        return
      }
      if (!ID_RE.test(id)) {
        errors.push({ line, message: `devices[${i}].id "${id}" 非法（仅字母/数字/下划线/连字符，≤48 字符）` })
        return
      }
      if (ids.has(id)) {
        errors.push({ line, message: `设备 id 重复：${id}` })
        return
      }
      ids.add(id)

      const icon = typeof item.icon === 'string' ? item.icon.trim() : ''
      if (!icon) {
        errors.push({ line, message: `设备 ${id} 缺少 icon` })
        return
      }
      const resolved = resolveIconForStyle(icon, iconStyle)
      if (!resolved) {
        if (hasIcon(icon) || inCatalog(icon)) {
          errors.push({
            line,
            message: `设备 ${id} 的图标 "${icon}" 不在风格配对目录（nas/proxy/ids 等仅供面板手动拖放，YAML 请换用配对图标）`
          })
        } else {
          const s = suggestCatalogId(icon)
          errors.push({
            line,
            message: `设备 ${id} 的图标 "${icon}" 不存在${s ? `，是否想用 "${s}"？` : '（无相近图标）'}`
          })
        }
        return
      }

      const label =
        typeof item.label === 'string'
          ? item.label
          : typeof item.name === 'string'
            ? item.name
            : typeof item.label === 'number'
              ? String(item.label)
              : typeof item.name === 'number'
                ? String(item.name)
                : ''

      const hasX = item.x !== undefined && item.x !== null
      const hasY = item.y !== undefined && item.y !== null
      let x: number | undefined
      let y: number | undefined
      if (hasX || hasY) {
        if (!hasX || !hasY || !isFinite(Number(item.x)) || !isFinite(Number(item.y))) {
          errors.push({ line, message: `设备 ${id} 的 x/y 必须同时提供有限数值` })
          return
        }
        x = Number(item.x)
        y = Number(item.y)
        if (anyCoord === null) anyCoord = line
      } else {
        anyMissing = true
      }

      const blockRaw = item.block ?? item.region // region = 旧字段别名
      let block: string | undefined
      if (typeof blockRaw === 'string' && blockRaw.trim()) {
        block = blockRaw.trim()
        if (!blocks.has(block)) blocks.set(block, { line })
      }

      devices.push({ id, src: resolved.src, w: resolved.w, h: resolved.h, label, x, y, block })
    })
    if (anyMissing && anyCoord !== null) {
      warnings.push({
        line: anyCoord,
        message: '部分设备提供了 x/y，已按连线整体自动布局（给定坐标被忽略；全部给坐标则精确定位）'
      })
    }
  }

  const needsLayout = devices.length > 0 && devices.some((d) => d.x === undefined || d.y === undefined)
  if (errors.length) return fail(errors, warnings)

  /* ---- 链路采集（设备 links / 区块 links / 旧 edges）→ 按行号排序后去重 ---- */
  const rawLinks: {
    a: string
    b: string
    line: number | null
    item: Record<string, unknown>
    explicitDir?: string
  }[] = []

  const pushLinkList = (owner: string, list: unknown, prefix: (string | number)[]) => {
    if (list === undefined || list === null) return
    if (!Array.isArray(list)) {
      errors.push({ line: lineOf(prefix), message: `${owner}.links 必须是列表` })
      return
    }
    ;(list as unknown[]).forEach((raw, j) => {
      const line = lineOf([...prefix, j])
      const item = raw as Record<string, unknown> | null
      if (!item || typeof item !== 'object' || Array.isArray(item)) {
        errors.push({ line, message: `${owner}.links[${j}] 不是有效对象` })
        return
      }
      const to = typeof item.to === 'string' ? item.to.trim() : ''
      if (!to) {
        errors.push({ line, message: `${owner}.links[${j}].to 缺失` })
        return
      }
      const dir = item.dir !== undefined && item.dir !== null ? String(item.dir).trim() : ''
      if (!dir) {
        errors.push({
          line,
          message: `${owner}.links[${j}] 缺少 dir（必填：上|下|左|右，表示对端相对本设备的方位）`
        })
        return
      }
      if (!DIR_VALUES.has(dir)) {
        errors.push({ line, message: `${owner}.links[${j}].dir 非法："${dir}"（可选 上|下|左|右）` })
        return
      }
      rawLinks.push({ a: owner, b: to, line, item, explicitDir: dir })
    })
  }

  nodeArr?.forEach((raw, i) => {
    const item = raw as Record<string, unknown> | null
    if (!item || typeof item !== 'object') return
    const id = typeof item.id === 'string' ? item.id.trim() : ''
    if (ids.has(id)) pushLinkList(id, item.links, [devKey as string, i, 'links'])
  })

  if (Array.isArray(data.blocks)) {
    ;(data.blocks as unknown[]).forEach((raw, i) => {
      const item = raw as Record<string, unknown> | null
      if (!item || typeof item !== 'object') return
      const id = typeof item.id === 'string' ? item.id.trim() : ''
      if (id && blocks.has(id)) pushLinkList(id, item.links, ['blocks', i, 'links'])
    })
  }

  if (data.edges !== undefined && data.edges !== null) {
    if (!Array.isArray(data.edges)) {
      errors.push({ line: 1, message: 'edges 必须是列表' })
    } else {
      ;(data.edges as unknown[]).forEach((raw, i) => {
        const line = lineOf(['edges', i])
        const item = raw as Record<string, unknown> | null
        if (!item || typeof item !== 'object' || Array.isArray(item)) {
          errors.push({ line, message: `edges[${i}] 不是有效对象` })
          return
        }
        const from = typeof item.from === 'string' ? item.from.trim() : ''
        const to = typeof item.to === 'string' ? item.to.trim() : ''
        if (!from || !ids.has(from)) {
          errors.push({ line, message: `edges[${i}].from "${from || '(空)'}" 未引用已定义的设备或区块` })
          return
        }
        if (!to || !ids.has(to)) {
          errors.push({ line, message: `edges[${i}].to "${to || '(空)'}" 未引用已定义的设备或区块` })
          return
        }
        if (from === to) {
          errors.push({ line, message: `edges[${i}] 不允许自环（from = to = ${from}）` })
          return
        }
        rawLinks.push({ a: from, b: to, line, item })
      })
    }
  }
  if (errors.length) return fail(errors, warnings)

  /* 按文档行号排序 → 重复声明按最早行定位 */
  rawLinks.sort((x, y) => (x.line ?? 0) - (y.line ?? 0))

  const blockNames = new Set(blocks.keys())
  const links: LinkIR[] = []
  const seenByPair = new Map<string, { a: string; b: string; line: number | null }>()
  let tbVotes = 0
  let lrVotes = 0

  for (const raw of rawLinks) {
    if (!ids.has(raw.a) && !blockNames.has(raw.a)) {
      errors.push({ line: raw.line, message: `链路起点 "${raw.a}" 未引用已定义的设备或区块` })
      continue
    }
    if (!ids.has(raw.b) && !blockNames.has(raw.b)) {
      errors.push({ line: raw.line, message: `links.to "${raw.b}" 未引用已定义的设备或区块` })
      continue
    }
    if (raw.a === raw.b) {
      errors.push({ line: raw.line, message: `链路不允许自环（${raw.a}）` })
      continue
    }

    /* 类型 → 预设 */
    let preset: LinkTypePreset | undefined
    let typeCanonical: string | undefined
    const typeRaw = raw.item.type !== undefined && raw.item.type !== null ? String(raw.item.type).trim() : ''
    if (typeRaw) {
      const hit = lookupLinkType(typeRaw, merged)
      if (hit.preset) {
        preset = hit.preset
        typeCanonical = hit.canonical
      } else {
        warnings.push({
          line: raw.line,
          message: `未知链路类型 "${typeRaw}"（可用：${builtinTypeNames().join('/')}），已回退默认线型`
        })
      }
    }

    /* 覆盖字段 */
    const ov: LinkOverrides = {}
    const it = raw.item
    let bad = false
    if (it.style !== undefined && it.style !== null) {
      const v = String(it.style)
      if (!STYLES.has(v)) {
        errors.push({ line: raw.line, message: `style 非法："${v}"（solid|dashed|dotted）` })
        bad = true
      } else ov.style = v as LineStyle
    }
    if (it.color !== undefined && it.color !== null) {
      const hex = resolveColor(String(it.color).trim())
      if (!hex) {
        errors.push({ line: raw.line, message: `color 非法："${String(it.color)}"（色板名或 #hex）` })
        bad = true
      } else ov.color = hex
    }
    if (it.width !== undefined && it.width !== null) {
      const n = Number(it.width)
      if (!isFinite(n) || n <= 0 || n > 8) {
        errors.push({ line: raw.line, message: 'width 非法（0~8 数值）' })
        bad = true
      } else ov.width = n
    }
    if (it.arrow !== undefined && it.arrow !== null) {
      const v = String(it.arrow)
      if (!ARROWS.has(v)) {
        errors.push({ line: raw.line, message: `arrow 非法："${v}"（none|one|both）` })
        bad = true
      } else ov.arrow = v as 'none' | 'one' | 'both'
    }
    if (it.marker !== undefined && it.marker !== null) {
      const v = String(it.marker)
      if (!MARKERS.has(v)) {
        errors.push({ line: raw.line, message: `marker 非法："${v}"（${[...MARKERS].join('|')}）` })
        bad = true
      } else ov.marker = v as MarkerName
    }
    if (it.shape !== undefined && it.shape !== null) {
      const v = String(it.shape)
      if (!REL_SHAPES.has(v)) {
        errors.push({ line: raw.line, message: `shape 非法："${v}"（straight|rounded）` })
        bad = true
      } else ov.shape = v as EdgeShape
    }
    if (it.label !== undefined && it.label !== null) ov.label = String(it.label)
    if (bad) continue

    /* 方位：设备 links 的 dir 必填；legacy edges 无 dir 视为下游 */
    const dir = raw.explicitDir
    const forward = !dir || dir === '下' || dir === '右'

    /* 单端声明：同一无序对只允许出现一次，重复 → 错误（无行序取舍） */
    const key = pairKey(raw.a, raw.b)
    const prev = seenByPair.get(key)
    if (prev) {
      const sameEnd = prev.a === raw.a
      errors.push({
        line: raw.line,
        message: sameEnd
          ? `设备 ${raw.a} 重复声明链路 {${raw.a} ↔ ${raw.b}}（首次在第 ${prev.line ?? '?'} 行）；每条链路只在一端声明一次`
          : `链路 {${raw.a} ↔ ${raw.b}} 已由 ${prev.a} 在第 ${prev.line ?? '?'} 行声明；每条链路只在一端声明一次`
      })
      continue
    }
    seenByPair.set(key, { a: raw.a, b: raw.b, line: raw.line })

    /* 轴计票：只统计被接受的链路 */
    if (dir === '左' || dir === '右') lrVotes++
    else tbVotes++

    links.push({
      a: raw.a,
      b: raw.b,
      line: raw.line,
      forward,
      explicitDir: dir,
      crossAxis: false,
      typeCanonical,
      preset,
      overrides: ov
    })
  }
  if (errors.length) return fail(errors, warnings)

  /* 轴判定与跨轴统计 */
  const axis: '上下' | '左右' = axisExplicit ?? (lrVotes > tbVotes ? '左右' : '上下')
  const axisSource: '显式' | '多数决' | '默认' = axisExplicit
    ? '显式'
    : tbVotes + lrVotes > 0
      ? '多数决'
      : '默认'
  const pair = axis === '上下' ? new Set(['上', '下']) : new Set(['左', '右'])
  let crossCount = 0
  links.forEach((L) => {
    if (L.explicitDir && !pair.has(L.explicitDir)) {
      L.crossAxis = true
      crossCount++
    }
  })
  if (axisExplicit && crossCount > 0) {
    warnings.push({
      line: null,
      message: `${crossCount} 条链路方位（${axis === '上下' ? '左/右' : '上/下'}）与主轴 ${axis} 不同向，已按主轴近似排列`
    })
  }

  /* 同文件顶层 layout:（yid→坐标覆盖，可选）——非法则忽略不报错 */
  let layout: Record<string, unknown> | null = null
  if (data.layout !== undefined && data.layout !== null) {
    if (typeof data.layout === 'object' && !Array.isArray(data.layout)) {
      layout = data.layout as Record<string, unknown>
    } else {
      warnings.push({ line: lineOf(['layout']), message: 'layout 必须是映射对象（yid → {x,y}），已忽略' })
    }
  }

  return {
    ok: true,
    errors,
    warnings,
    devices,
    blocks,
    links,
    axis,
    axisSource,
    crossAxis: crossCount,
    needsLayout,
    layout
  }
}

/* ---------- 对外 API：只解析校验 ---------- */

export function parseTopoYaml(text: string): YamlResult {
  const r = parseAndValidate(text)
  return {
    ok: r.ok,
    errors: r.errors,
    warnings: r.warnings,
    devices: r.devices.length,
    edges: r.links.length,
    blocks: r.blocks.size,
    axis: r.axis,
    axisSource: r.axisSource,
    crossAxis: r.crossAxis,
    needsLayout: r.needsLayout
  }
}

/**
 * 解析并原子应用（校验失败画布不动；整个导入含布局在单个批次内 → ⌘Z 一步回退）。
 * - 设备 links/区块 links/旧 edges → 链路（无序对去重，最早行为准）
 * - 缺坐标 → 按主轴 dagre compound 布局；区块贴合子内容
 * - data.layout 存方位解算后的布局方向（可与连线方向不同）
 */
export async function applyTopoYaml(graph: Graph, text: string): Promise<YamlResult> {
  const r = parseAndValidate(text)
  const base: YamlResult = {
    ok: r.ok,
    errors: r.errors,
    warnings: r.warnings,
    devices: r.devices.length,
    edges: r.links.length,
    blocks: r.blocks.size,
    axis: r.axis,
    axisSource: r.axisSource,
    crossAxis: r.crossAxis,
    needsLayout: r.needsLayout
  }
  if (!r.ok) return base

  const deviceMap = new Map<string, Node>()
  const regionMap = new Map<string, Node>()
  let lay: AutoLayoutResult = { ranker: 'none', fallback: false }

  const ensureRegion = (name: string): Node => {
    let reg = regionMap.get(name)
    if (!reg) {
      const spec = r.blocks.get(name)
      reg = graph.addNode({ ...createRegionMeta(), x: 0, y: 0 } as never) as Node
      const title = spec?.label ?? name
      reg.setAttrByPath('label/text', title)
      reg.setData({ ...(reg.getData() || {}), label: title, yid: name })
      if (spec?.color) applyRegionColor(reg, spec.color)
      regionMap.set(name, reg)
    }
    return reg
  }

  try {
    cancelRegionRefit()
    graph.startBatch('yaml-import')
    try {
    graph.cleanSelection()
    graph.clearCells()

    // 区块先建（含空区块）
    r.blocks.forEach((_, name) => ensureRegion(name))

    // 设备
    r.devices.forEach((d) => {
      const node = graph.addNode({
        ...createTopoNodeMeta(d.src, d.label, d.w, d.h),
        x: d.x ?? 0,
        y: d.y ?? 0
      } as never) as Node
      deviceMap.set(d.id, node)
      node.setData({ ...(node.getData() || {}), yid: d.id })
      if (d.block) node.addTo(ensureRegion(d.block))
    })

    // 精确坐标路径：区块立即贴合
    if (!r.needsLayout) {
      regionMap.forEach((reg) => refitRegion(graph, reg))
    }

    // 链路 → 边
    r.links.forEach((L) => {
      const sa = deviceMap.get(L.a) ?? regionMap.get(L.a)
      const tb = deviceMap.get(L.b) ?? regionMap.get(L.b)
      if (!sa || !tb) return
      const p = L.preset
      const edge = p?.double ? createDoubleEdge(resolveColor(p.color) || '#4b5563', p.width) : createEdgeCell()
      if (p) applyPresetBase(edge, p)
      const ov = L.overrides
      if (ov.style) applyLineStyle([edge], ov.style)
      if (ov.color) edge.attr('line/stroke', ov.color)
      if (ov.width) edge.attr('line/strokeWidth', ov.width)
      if (ov.arrow) {
        const m: MarkerName = ov.marker ?? 'classic'
        applyEndMarkers([edge], ov.arrow === 'both' ? m : 'none', m)
      } else if (ov.marker) {
        applyEndMarkers([edge], ov.marker, ov.marker)
      }
      if (ov.shape) applyEdgeShape([edge], ov.shape)
      if (ov.label) applyEdgeLabel([edge], { text: ov.label })
      edge.setSource(sa)
      edge.setTarget(tb)
      const layoutSrc = L.forward ? sa.id : tb.id
      const layoutTgt = L.forward ? tb.id : sa.id
      edge.setData({
        linkType: L.typeCanonical ?? '',
        a: L.a,
        b: L.b,
        cross: L.crossAxis,
        dir: L.explicitDir ?? '',
        layout: { src: layoutSrc, tgt: layoutTgt }
      })
      graph.addEdge(edge)
    })

    // 缺坐标 → 布局（ranker 降级链；全败 → 应急网格，永不抛错、不堆叠）
    if (r.needsLayout) {
      lay = await autoLayout(graph, r.axis === '左右' ? 'LR' : 'TB', { skipBatch: true })
    }
    /* 同文件 layout: 段 → 按 yid 套坐标覆盖（老设备恢复人工坐标，新设备保留自动位） */
    if (r.layout) applyCoordsByYid(graph, r.layout)
    /* 端点绑定：所有设备边按声明 dir 绑定固定四端点 */
    graph.getEdges().forEach((e) => bindEdgePorts(e))
    /* 图例自动生成（纳入同一批次 → ⌘Z 一步含图例回退） */
    syncLegend(graph)
    } finally {
      graph.stopBatch('yaml-import')
      cancelRegionRefit()
    }

    graph.zoomToFit({ padding: 60, maxScale: 1.2 })
  } catch (e) {
    return {
      ...base,
      ok: false,
      errors: [{ line: null, message: `应用失败：${(e as Error).message}` }]
    }
  }

  const warnings = [...r.warnings]
  if (lay.fallback) {
    warnings.push(
      lay.ranker === 'grid'
        ? { line: null, message: '自动布局主排序器不可用，已采用备用网格排列（建议人工微调）' }
        : { line: null, message: `自动布局排序器已降级为 ${lay.ranker}（主排序器对含区块级链路的图存在缺陷）` }
    )
  }
  return { ...base, ok: true, errors: [], warnings }
}

/* ---------------- 布局 sidecar：按 yid 套坐标覆盖 ---------------- */

export interface LayoutOverrideResult {
  ok: boolean
  applied: number
  message: string
}

/** 按 yid 套坐标到当前图节点（独立 ⌘Z 批次；套完触发区域贴合）。
    返回 [已套用数, 总条目数]。老设备（同 yid）恢复坐标，新设备不在 map 里→保留现位。 */
function applyCoordsByYid(graph: Graph, layout: Record<string, unknown>): [number, number] {
  const byYid = new Map<string, Node>()
  graph.getNodes().forEach((n) => {
    const yid = String((n.getData() as { yid?: string } | undefined)?.yid ?? '')
    if (yid) byYid.set(yid, n)
  })

  let applied = 0
  const entries = Object.entries(layout)
  graph.startBatch('layout-override')
  try {
    entries.forEach(([yid, v]) => {
      const node = byYid.get(yid)
      if (!node) return
      const x = Number((v as { x?: unknown })?.x)
      const y = Number((v as { y?: unknown })?.y)
      if (!isFinite(x) || !isFinite(y)) return
      node.setPosition(x, y)
      applied++
    })
    refitAllRegions(graph)
  } finally {
    graph.stopBatch('layout-override')
  }
  return [applied, entries.length]
}

/** 解析 `layout:` sidecar（yid → {x,y}），按 data.yid 套到当前图节点。
    不清空画布、独立 ⌘Z 批次；套完触发区域贴合。 */
export function applyLayoutOverride(graph: Graph, text: string): LayoutOverrideResult {
  const doc = parseDocument(text, { keepSourceTokens: true })
  if (doc.errors.length) {
    return { ok: false, applied: 0, message: `布局文件解析失败：${doc.errors[0].message}` }
  }
  const root = doc.toJS() as { layout?: unknown }
  const layout = root?.layout
  if (!layout || typeof layout !== 'object' || Array.isArray(layout)) {
    return { ok: false, applied: 0, message: '布局文件缺少顶层 layout: 映射' }
  }

  const [applied, total] = applyCoordsByYid(graph, layout as Record<string, unknown>)
  const unmatched = total - applied
  let message = `已套用 ${applied} 个设备坐标`
  if (unmatched > 0) message += `（${unmatched} 个 yid 未匹配到设备，已忽略）`
  return { ok: true, applied, message }
}
