// ─── 演示材料类型定义 ────────────────────────────────────────────
// 核心原则：Deck JSON 是唯一事实源，HTML 只是渲染层。
// AI 决定"是什么"，Layout Engine 决定"怎么显示"。

export type PresentationStatus = 'discussing' | 'generated'

// ─── 主题（第一等化结构对象：渲染层消费 CSS token） ──────────────
export interface PresentationTheme {
  id: string
  name: string
  colors: {
    background: string
    backgroundImage?: string
    text: string
    accent: string
    tint: string
    line: string
    muted: string
    card: string
  }
  fonts: { family: string }
  radius: string
  shadow: string
}

// ─── Block data：每类型一个结构化数据接口 ────────────────────────
export interface TextData {
  /** paragraph 存 Tiptap JSON 字符串（schema 友好）；heading/kicker 为纯文本 */
  text: string
}

export interface ListData {
  items: string[]
}

/** 表格：结构化列（含列宽）+ 行（每行单元格，可合并）。rows[0] 为表头行。 */
export interface TableColumn {
  id: string
  /** 列宽百分比（1-100）；缺省 = 自动 */
  width?: number
}
export interface TableCell {
  content: string
  rowspan?: number
  colspan?: number
}
export interface TableRow {
  cells: TableCell[]
}
export interface TableData {
  columns: TableColumn[]
  rows: TableRow[]
}

export interface KpiItem {
  value: string
  label: string
  sub?: string
}
export interface KpiData {
  items: KpiItem[]
}

export interface CardItem {
  icon: string
  title: string
  desc: string
}
export interface CardsData {
  items: CardItem[]
}

export interface ColumnSide {
  title: string
  items: string[]
}
export interface ColumnsData {
  left: ColumnSide
  right: ColumnSide
}

export interface TimelineItem {
  time: string
  title: string
  desc?: string
}
export interface TimelineData {
  items: TimelineItem[]
}

export interface ImageData {
  src?: string
  /** 图片宽度占本列比例（0.25 ~ 1），默认 1 */
  size?: number
  fit?: 'contain' | 'cover'
}

export interface ChartData {
  kind: 'bar' | 'line'
  labels: string[]
  values: number[]
}

export interface QuoteData {
  text: string
  author?: string
}

export interface CtaData {
  text: string
  subtext?: string
}

export type DividerData = Record<string, never>

export interface BlockDataMap {
  kicker: TextData
  heading: TextData
  paragraph: TextData
  bullets: ListData
  'num-list': ListData
  table: TableData
  'kpi-row': KpiData
  'cards-grid': CardsData
  columns: ColumnsData
  timeline: TimelineData
  image: ImageData
  chart: ChartData
  quote: QuoteData
  cta: CtaData
  divider: DividerData
}

export type BlockType = keyof BlockDataMap

/** 页内区块实例：指向元素注册表中的 PresetBlockMeta.type，data 为内容字段 */
export interface BlockInstance<T extends BlockType = BlockType> {
  id: string
  type: T
  data: BlockDataMap[T]
  /** 12 栅格跨度：6=半宽 4=1/3 8=2/3 3=1/4；缺省/12=全宽 */
  span?: number
  /** 字号档位倍率：0.85 紧凑 / 1 标准 / 1.15 宽松 */
  scale?: number
}

/** 单页幻灯片：版式壳 + 有序区块（自由组合的结果） */
export interface Slide {
  id: string
  /** 指向 PresetLayoutMeta.id */
  layout: string
  blocks: BlockInstance[]
  /** 演讲备注 */
  note?: string
}

/** 元素注册表：区块元数据 */
export interface PresetBlockMeta {
  type: BlockType
  name: string
  /** AppIcon 名称或内置 key */
  icon: string
  /** data 允许的字段 */
  props: string[]
  /** 新建区块时的示例内容 */
  sample: Record<string, unknown>
}

/** 元素注册表：版式元数据 */
export interface PresetLayoutMeta {
  id: string
  name: string
  icon: string
  /** 页面骨架类型：hero 居中 / header 标题+正文 */
  shell: 'hero' | 'header'
  /** 该版式允许的区块类型（AI 组合与校验约束） */
  allowedBlocks: string[]
}

export interface DiscussionMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  timestamp: number
  typing?: boolean
  attachments?: Array<{ path: string; name: string; kind: 'image' | 'doc'; previewUrl?: string }>
}

export interface PresentationItem {
  id: string
  title: string
  subtitle?: string
  /** 主题 id（指向 THEMES 注册表） */
  theme: string
  folderId: string | null
  discussion: DiscussionMessage[]
  slides: Slide[]
  /** 完整 HTML 内容（新架构） */
  htmlContent?: string
  /** 每页缩略图（base64 data URL，由 SVG foreignObject 生成） */
  thumbnails?: string[]
  status: PresentationStatus
  /** 静态演示版：模拟生成中的瞬态标记 */
  generating?: boolean
  createdAt: number
  updatedAt: number
}

export interface PresentationFolder {
  id: string
  name: string
  parentId: string | null
  sortOrder: number
}

/** 目录树节点（含子级，递归） */
export interface FolderNode extends PresentationFolder {
  children: FolderNode[]
}
