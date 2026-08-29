import type { BlockInstance, Slide } from '@/types/presentation'
import { extractPlain, parseDoc, toDoc } from '@/presentation/richText'

// ─── 版面容量预算（对应后端提示词中的容量约束，防止元素溢出） ─────
const MAX_BLOCKS = 6
const MAX_BULLETS = 6
const MAX_BULLET_CHARS = 60
const MAX_PARAGRAPH_CHARS = 120
const MAX_QUOTE_CHARS = 120
const MAX_TABLE_ROWS = 7
const MAX_TABLE_COLS = 5
const MAX_CELL_CHARS = 20
const MAX_KPIS = 4
const MAX_CARDS = 3

function truncate(s: string, n: number): string {
  const t = s.trim()
  return t.length > n ? t.slice(0, n - 1) + '…' : t
}

/**
 * 保守裁剪：对 AI 产出做版面兜底，保证 1280×720 一屏放得下。
 * - 超限区块/条目/行列裁剪；超长文本截断
 * - 内容密集（区块多/大表格/条目多）时正文区块自动降为紧凑字号
 * 返回新对象，不改动入参。
 */
export function fitSlide(slide: Slide): Slide {
  let dense = slide.blocks.length > MAX_BLOCKS

  const fitted: BlockInstance[] = slide.blocks.slice(0, MAX_BLOCKS).map((b) => {
    const data = { ...b.data } as Record<string, unknown>
    switch (b.type) {
      case 'bullets':
      case 'num-list': {
        const items = Array.isArray(data.items) ? (data.items as string[]) : []
        if (items.length > MAX_BULLETS) dense = true
        return {
          ...b,
          data: {
            items: items.slice(0, MAX_BULLETS).map((x) => truncate(String(x), MAX_BULLET_CHARS))
          }
        }
      }
      case 'paragraph': {
        const raw = typeof data.text === 'string' ? data.text : ''
        if (raw.length > 600) {
          const plain = extractPlain(parseDoc(raw))
          return {
            ...b,
            data: { text: JSON.stringify(toDoc(truncate(plain, MAX_PARAGRAPH_CHARS))) }
          }
        }
        return b
      }
      case 'quote': {
        const t = typeof data.text === 'string' ? data.text : ''
        return t.length > MAX_QUOTE_CHARS
          ? { ...b, data: { text: truncate(t, MAX_QUOTE_CHARS), author: data.author } }
          : b
      }
      case 'table': {
        const columns = Array.isArray(data.columns)
          ? (data.columns as Array<{ id: string; width?: number }>).slice(0, MAX_TABLE_COLS)
          : []
        const rows = Array.isArray(data.rows)
          ? (data.rows as Array<{
              cells: Array<{ content: string; rowspan?: number; colspan?: number }>
            }>)
          : []
        if (rows.length > MAX_TABLE_ROWS) dense = true
        return {
          ...b,
          data: {
            columns,
            rows: rows.slice(0, MAX_TABLE_ROWS).map((r) => ({
              cells: r.cells
                .slice(0, MAX_TABLE_COLS)
                .map((c) => ({ ...c, content: truncate(c.content, MAX_CELL_CHARS) }))
            }))
          }
        }
      }
      case 'kpi-row': {
        const items = Array.isArray(data.items)
          ? (data.items as Array<{ value: string; label: string; sub?: string }>).slice(0, MAX_KPIS)
          : []
        return { ...b, data: { items } }
      }
      case 'cards-grid': {
        const items = Array.isArray(data.items)
          ? (data.items as Array<{ icon: string; title: string; desc: string }>).slice(0, MAX_CARDS)
          : []
        return { ...b, data: { items } }
      }
      default:
        return b
    }
  })

  if (dense) {
    fitted.forEach((b) => {
      if (b.type !== 'heading' && b.type !== 'kicker') b.scale = 0.85
    })
  }
  return { ...slide, blocks: fitted }
}
