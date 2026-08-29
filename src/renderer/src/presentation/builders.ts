import type { BlockInstance } from '@/types/presentation'
import { uid } from '@/presentation/elements'

/** 区块便捷构造器（自带 id，供命令/溯源/AI 规则复用） */
export const B = {
  kicker: (text: string): BlockInstance => ({ id: uid(), type: 'kicker', data: { text } }),
  heading: (text: string): BlockInstance => ({ id: uid(), type: 'heading', data: { text } }),
  paragraph: (text: string): BlockInstance => ({
    id: uid(),
    type: 'paragraph',
    data: {
      text: JSON.stringify({
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text }] }]
      })
    }
  }),
  bullets: (items: string[]): BlockInstance => ({ id: uid(), type: 'bullets', data: { items } }),
  numList: (items: string[]): BlockInstance => ({
    id: uid(),
    type: 'num-list',
    data: { items }
  }),
  table: (headers: string[], rows: string[][]): BlockInstance => ({
    id: uid(),
    type: 'table',
    data: {
      columns: headers.map((_, i) => ({ id: `c${i + 1}`, width: undefined })),
      rows: [
        { cells: headers.map((h) => ({ content: h })) },
        ...rows.map((r) => ({ cells: r.map((c) => ({ content: c })) }))
      ]
    }
  }),
  kpis: (items: { value: string; label: string; sub?: string }[]): BlockInstance => ({
    id: uid(),
    type: 'kpi-row',
    data: { items }
  }),
  cards: (items: { icon: string; title: string; desc: string }[]): BlockInstance => ({
    id: uid(),
    type: 'cards-grid',
    data: { items }
  }),
  columns: (
    left: { title: string; items: string[] },
    right: { title: string; items: string[] }
  ): BlockInstance => ({
    id: uid(),
    type: 'columns',
    data: { left, right }
  }),
  timeline: (items: { time: string; title: string; desc?: string }[]): BlockInstance => ({
    id: uid(),
    type: 'timeline',
    data: { items }
  }),
  quote: (text: string, author: string): BlockInstance => ({
    id: uid(),
    type: 'quote',
    data: { text, author }
  }),
  cta: (text: string, subtext?: string): BlockInstance => ({
    id: uid(),
    type: 'cta',
    data: { text, subtext }
  }),
  image: (): BlockInstance => ({ id: uid(), type: 'image', data: {} }),
  chart: (kind: 'bar' | 'line', labels: string[], values: number[]): BlockInstance => ({
    id: uid(),
    type: 'chart',
    data: { kind, labels, values }
  })
}

/** 设置区块栅格跨度 */
export const W = (b: BlockInstance, span: number): BlockInstance => ({ ...b, span })
