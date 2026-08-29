import type {
  BlockDataMap,
  BlockInstance,
  BlockType,
  PresetBlockMeta,
  PresetLayoutMeta,
  PresentationTheme,
  Slide,
  TableCell,
  TableColumn,
  TableData,
  TableRow
} from '@/types/presentation'

// ═══════════════════════════════════════════════════════════════════
// 预设元素注册表 —— 演示材料的单一来源（版式 + 区块）
// AI 组合 / 手动装配 / 校验 / 导出 全部基于此注册表
// ═══════════════════════════════════════════════════════════════════

// ─── 主题（结构对象，切换只改 CSS token，不重建内容） ──────────────
export const THEMES: PresentationTheme[] = [
  {
    id: 'default',
    name: '简约',
    colors: {
      background: '#ffffff',
      text: '#1d2129',
      accent: '#2563eb',
      tint: '#eff6ff',
      line: '#e5e7eb',
      muted: '#86909c',
      card: '#ffffff'
    },
    fonts: { family: "Inter, 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif" },
    radius: '10px',
    shadow: '0 2px 8px rgba(0,0,0,0.06)'
  },
  {
    id: 'dark',
    name: '深色',
    colors: {
      background: '#12161f',
      backgroundImage: 'linear-gradient(150deg, #12161f 0%, #1a2233 100%)',
      text: '#f3f5f9',
      accent: '#60a5fa',
      tint: 'rgba(96, 165, 250, 0.12)',
      line: 'rgba(255, 255, 255, 0.14)',
      muted: '#94a3b8',
      card: '#1a2233'
    },
    fonts: { family: "Inter, 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif" },
    radius: '10px',
    shadow: '0 2px 8px rgba(0,0,0,0.3)'
  },
  {
    id: 'brand',
    name: '品牌',
    colors: {
      background: '#f2f7ff',
      backgroundImage: 'linear-gradient(160deg, #f2f7ff 0%, #e7efff 100%)',
      text: '#16233f',
      accent: '#1d4ed8',
      tint: 'rgba(29, 78, 216, 0.08)',
      line: '#d3e0f5',
      muted: '#64748b',
      card: '#ffffff'
    },
    fonts: { family: "Inter, 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif" },
    radius: '10px',
    shadow: '0 2px 8px rgba(0,0,0,0.08)'
  }
]

export function getTheme(id?: string): PresentationTheme {
  return THEMES.find((t) => t.id === id) ?? THEMES[0]
}

// ─── 区块注册表 ──────────────────────────────────────────────────
export const BLOCKS: PresetBlockMeta[] = [
  { type: 'kicker', name: '眉题', icon: 'K', props: ['text'], sample: { text: '核心能力' } },
  { type: 'heading', name: '标题', icon: 'H', props: ['text'], sample: { text: '页面标题' } },
  {
    type: 'paragraph',
    name: '段落',
    icon: '¶',
    props: ['text'],
    sample: {
      text: '{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"这里是一段说明文字。"}]}]}'
    }
  },
  {
    type: 'bullets',
    name: '要点',
    icon: '•',
    props: ['items'],
    sample: { items: ['要点一', '要点二', '要点三'] }
  },
  {
    type: 'num-list',
    name: '步骤',
    icon: '1.',
    props: ['items'],
    sample: { items: ['第一步', '第二步', '第三步'] }
  },
  {
    type: 'table',
    name: '表格',
    icon: 'T',
    props: ['columns', 'rows'],
    sample: {
      columns: [{ id: 'c1' }, { id: 'c2' }],
      rows: [
        { cells: [{ content: '维度' }, { content: '说明' }] },
        { cells: [{ content: 'A' }, { content: '内容A' }] },
        { cells: [{ content: 'B' }, { content: '内容B' }] }
      ]
    }
  },
  {
    type: 'kpi-row',
    name: '数据卡',
    icon: 'KPI',
    props: ['items'],
    sample: {
      items: [
        { value: '99.9%', label: '可用性', sub: 'SLA 保障' },
        { value: '-44%', label: '成本下降' }
      ]
    }
  },
  {
    type: 'cards-grid',
    name: '卡片网格',
    icon: '▧',
    props: ['items'],
    sample: {
      items: [
        { icon: '◆', title: '能力一', desc: '能力说明文字' },
        { icon: '◆', title: '能力二', desc: '能力说明文字' },
        { icon: '◆', title: '能力三', desc: '能力说明文字' }
      ]
    }
  },
  {
    type: 'columns',
    name: '左右分栏',
    icon: 'Ⅱ',
    props: ['left', 'right'],
    sample: {
      left: { title: '方案A', items: ['优势一', '优势二'] },
      right: { title: '方案B', items: ['优势一', '优势二'] }
    }
  },
  {
    type: 'timeline',
    name: '时间轴',
    icon: 'TL',
    props: ['items'],
    sample: {
      items: [
        { time: '一期', title: '基础建设', desc: '平台部署与核心模块上线' },
        { time: '二期', title: '深化应用', desc: '业务系统全面接入' }
      ]
    }
  },
  {
    type: 'image',
    name: '图片',
    icon: 'IMG',
    props: ['src', 'size', 'fit'],
    sample: {}
  },
  {
    type: 'chart',
    name: '图表',
    icon: 'CH',
    props: ['kind', 'labels', 'values'],
    sample: { kind: 'bar', labels: ['Q1', 'Q2', 'Q3', 'Q4'], values: [40, 65, 80, 92] }
  },
  {
    type: 'quote',
    name: '引用',
    icon: 'Q',
    props: ['text', 'author'],
    sample: { text: '引用一段话', author: '—— 署名' }
  },
  {
    type: 'cta',
    name: '行动号召',
    icon: 'CTA',
    props: ['text', 'subtext'],
    sample: { text: '联系我们', subtext: '期待深入交流' }
  },
  { type: 'divider', name: '分隔线', icon: '—', props: [], sample: {} }
]

// ─── 版式注册表 ──────────────────────────────────────────────────
export const LAYOUTS: PresetLayoutMeta[] = [
  {
    id: 'cover',
    name: '封面',
    icon: 'CO',
    shell: 'hero',
    allowedBlocks: ['kicker', 'heading', 'paragraph', 'cta', 'image', 'chart']
  },
  {
    id: 'section',
    name: '章节页',
    icon: '§',
    shell: 'hero',
    allowedBlocks: ['heading', 'paragraph', 'bullets']
  },
  {
    id: 'agenda',
    name: '目录',
    icon: 'AG',
    shell: 'header',
    allowedBlocks: ['heading', 'num-list']
  },
  {
    id: 'content',
    name: '内容页',
    icon: 'CO',
    shell: 'header',
    allowedBlocks: BLOCKS.map((b) => b.type)
  },
  { id: 'table', name: '表格页', icon: 'T', shell: 'header', allowedBlocks: ['heading', 'table'] },
  {
    id: 'timeline',
    name: '时间轴页',
    icon: 'TL',
    shell: 'header',
    allowedBlocks: ['heading', 'timeline']
  },
  {
    id: 'stats',
    name: '数据页',
    icon: 'KPI',
    shell: 'header',
    allowedBlocks: ['heading', 'kpi-row']
  },
  {
    id: 'image-text',
    name: '图文页',
    icon: 'IM',
    shell: 'header',
    allowedBlocks: ['heading', 'image', 'paragraph', 'bullets']
  },
  { id: 'quote', name: '引用页', icon: 'Q', shell: 'hero', allowedBlocks: ['quote'] },
  {
    id: 'closing',
    name: '结束页',
    icon: 'CL',
    shell: 'hero',
    allowedBlocks: ['heading', 'paragraph', 'cta']
  }
]

// ─── 工具函数 ────────────────────────────────────────────────────
export function getLayout(id: string): PresetLayoutMeta {
  return LAYOUTS.find((l) => l.id === id) ?? LAYOUTS.find((l) => l.id === 'content')!
}

export function getBlock(type: string): PresetBlockMeta {
  return BLOCKS.find((b) => b.type === type) ?? BLOCKS.find((b) => b.type === 'paragraph')!
}

export function sampleBlock(type: string): BlockInstance {
  const meta = getBlock(type)
  return {
    id: uid(),
    type: meta.type,
    data: JSON.parse(JSON.stringify(meta.sample)) as never,
    span: 12,
    scale: 1
  }
}

// ─── 栅格跨度预设（12 列） ────────────────────────────────────────
export const BLOCK_SPANS: Array<{ value: number; label: string }> = [
  { value: 12, label: '全宽' },
  { value: 8, label: '2/3' },
  { value: 6, label: '半宽' },
  { value: 4, label: '1/3' },
  { value: 3, label: '1/4' }
]

/** 规整 span 到合法集合 */
export function normalizeSpan(span?: number): number {
  if (typeof span === 'number' && BLOCK_SPANS.some((s) => s.value === span)) return span
  return 12
}

// ─── 字号档位预设 ────────────────────────────────────────────────
export const BLOCK_SCALES: Array<{ value: number; label: string }> = [
  { value: 0.85, label: '紧凑' },
  { value: 1, label: '标准' },
  { value: 1.15, label: '宽松' }
]

/** 规整 scale 到合法集合 */
export function normalizeScale(scale?: number): number {
  if (typeof scale === 'number' && BLOCK_SCALES.some((s) => s.value === scale)) return scale
  return 1
}

/** 布局是否允许某区块 */
export function layoutAllows(layoutId: string, blockType: string): boolean {
  return getLayout(layoutId).allowedBlocks.includes(blockType)
}

// ─── Block data 规范化（per-type 校验 + 旧表格形状迁移） ───────────
function str(raw: Record<string, unknown>, k: string, def = ''): string {
  return typeof raw[k] === 'string' ? (raw[k] as string) : def
}
function optStr(raw: Record<string, unknown>, k: string): string | undefined {
  return typeof raw[k] === 'string' && raw[k] ? (raw[k] as string) : undefined
}
function strArr(raw: Record<string, unknown>, k: string): string[] {
  return Array.isArray(raw[k]) ? (raw[k] as unknown[]).map((x) => String(x ?? '')) : []
}
function objArr(raw: Record<string, unknown>, k: string): Record<string, unknown>[] {
  return Array.isArray(raw[k]) ? (raw[k] as Record<string, unknown>[]) : []
}
function num(raw: Record<string, unknown>, k: string): number | undefined {
  const v = raw[k]
  return typeof v === 'number' && Number.isFinite(v) ? v : undefined
}

/** 单元格规范化到列数（缺失补齐 / 多余截断） */
function normalizeCells(cells: unknown[], n: number): TableCell[] {
  return Array.from({ length: n }, (_, i) => {
    const c = cells[i]
    if (c && typeof c === 'object') {
      const cc = c as Record<string, unknown>
      return {
        content: typeof cc.content === 'string' ? cc.content : String(c ?? ''),
        rowspan: typeof cc.rowspan === 'number' && cc.rowspan > 1 ? cc.rowspan : undefined,
        colspan: typeof cc.colspan === 'number' && cc.colspan > 1 ? cc.colspan : undefined
      }
    }
    return { content: String(c ?? '') }
  })
}

/** 旧表格形状（headers+rows 二维数组）→ 结构化 columns + rows(cells) */
function migrateTable(raw: Record<string, unknown>): TableData {
  const hasColumns = Array.isArray(raw.columns)
  if (hasColumns) {
    const columns: TableColumn[] = (raw.columns as unknown[]).map((c, i) => {
      const cc = c && typeof c === 'object' ? (c as Record<string, unknown>) : {}
      return {
        id: typeof cc.id === 'string' && cc.id ? cc.id : `c${i + 1}`,
        width: typeof cc.width === 'number' && cc.width > 0 ? cc.width : undefined
      }
    })
    const n = columns.length || 1
    const rows: TableRow[] = (Array.isArray(raw.rows) ? (raw.rows as unknown[]) : []).map((r) => {
      const rr = r && typeof r === 'object' ? (r as Record<string, unknown>) : {}
      const cells = Array.isArray(rr.cells)
        ? (rr.cells as unknown[])
        : Array.isArray(r)
          ? (r as unknown[])
          : []
      return { cells: normalizeCells(cells, n) }
    })
    return { columns, rows }
  }
  // 旧形状：headers + rows
  const headers = strArr(raw, 'headers')
  const oldRows = Array.isArray(raw.rows) ? (raw.rows as unknown[]) : []
  let n = headers.length
  for (const r of oldRows) {
    if (Array.isArray(r)) n = Math.max(n, r.length)
  }
  n = Math.max(n, 1)
  const columns: TableColumn[] = Array.from({ length: n }, (_, i) => ({
    id: `c${i + 1}`,
    width: undefined
  }))
  const rows: TableRow[] = []
  if (headers.length) rows.push({ cells: normalizeCells([...headers], n) })
  for (const r of oldRows) {
    if (Array.isArray(r)) rows.push({ cells: normalizeCells(r, n) })
  }
  return { columns, rows }
}

/** 规范化单个区块 data（返回可安全使用的 Record 形状） */
function normalizeBlockData(
  type: BlockType,
  raw: Record<string, unknown>
): Record<string, unknown> {
  switch (type) {
    case 'kicker':
    case 'heading':
      return { text: str(raw, 'text') }
    case 'paragraph':
      return { text: str(raw, 'text') }
    case 'bullets':
    case 'num-list':
      return { items: strArr(raw, 'items') }
    case 'table':
      return migrateTable(raw) as unknown as Record<string, unknown>
    case 'kpi-row':
      return {
        items: objArr(raw, 'items').map((it) => ({
          value: str(it, 'value'),
          label: str(it, 'label'),
          sub: optStr(it, 'sub')
        }))
      }
    case 'cards-grid':
      return {
        items: objArr(raw, 'items').map((it) => ({
          icon: str(it, 'icon', '◆'),
          title: str(it, 'title'),
          desc: str(it, 'desc')
        }))
      }
    case 'columns':
      return {
        left: {
          title: str((raw.left as Record<string, unknown>) ?? {}, 'title'),
          items: strArr((raw.left as Record<string, unknown>) ?? {}, 'items')
        },
        right: {
          title: str((raw.right as Record<string, unknown>) ?? {}, 'title'),
          items: strArr((raw.right as Record<string, unknown>) ?? {}, 'items')
        }
      }
    case 'timeline':
      return {
        items: objArr(raw, 'items').map((it) => ({
          time: str(it, 'time'),
          title: str(it, 'title'),
          desc: optStr(it, 'desc')
        }))
      }
    case 'image':
      return {
        src: optStr(raw, 'src'),
        size: num(raw, 'size'),
        fit: raw.fit === 'cover' ? 'cover' : raw.fit === 'contain' ? 'contain' : undefined
      }
    case 'chart':
      return {
        kind: raw.kind === 'line' ? 'line' : 'bar',
        labels: strArr(raw, 'labels'),
        values: strArr(raw, 'values')
          .map((x) => Number(x))
          .filter((x) => Number.isFinite(x))
      }
    case 'quote':
      return { text: str(raw, 'text'), author: optStr(raw, 'author') }
    case 'cta':
      return { text: str(raw, 'text'), subtext: optStr(raw, 'subtext') }
    case 'divider':
      return {}
  }
}

/** 校验单区块：id 补齐、data 规范化、span/scale 规整 */
export function validateBlock(input: BlockInstance): BlockInstance {
  const meta = getBlock(input.type)
  const data = normalizeBlockData(meta.type, input.data as unknown as Record<string, unknown>)
  return {
    id: input.id || uid(),
    type: meta.type,
    data: data as never,
    span: normalizeSpan(input.span),
    scale: normalizeScale(input.scale)
  }
}

/** 校验单页：非法 layout / block 回退到安全值，span 规整，旧表格自动迁移 */
export function validateSlide(slide: Slide): Slide {
  const layout = getLayout(slide.layout)
  const safe: Slide = { id: slide.id, layout: layout.id, blocks: [], note: slide.note }
  for (const b of slide.blocks) {
    if (layout.allowedBlocks.includes(b.type)) {
      safe.blocks.push(validateBlock(b))
    } else if (layout.id === 'content') {
      // content 版式允许全部；非法类型退化为段落
      safe.blocks.push({
        id: b.id || uid(),
        type: 'paragraph',
        data: { text: String((b.data as unknown as Record<string, unknown>)?.text ?? '') } as never,
        span: 12,
        scale: 1
      })
    }
  }
  if (safe.blocks.length === 0) {
    safe.blocks.push(sampleBlock('paragraph'))
  }
  return safe
}

export function uid(): string {
  return crypto.randomUUID()
}

// ─── 便捷构造器 ──────────────────────────────────────────────────
export function mkSlide(layout: string, blocks: BlockInstance[], note?: string): Slide {
  return validateSlide({ id: uid(), layout, blocks, note })
}

export type { BlockDataMap }
