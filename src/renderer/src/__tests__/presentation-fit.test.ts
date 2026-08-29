import { describe, it, expect } from 'vitest'
import { ensureLayoutCompat, finalizeAiSlide, normalizeAiSlide } from '@/utils/presentationAi'
import { fitSlide } from '@/presentation/fit'
import type { Slide } from '@/types/presentation'

describe('presentation AI slide normalization', () => {
  it('converts top-level heading/title into a heading block', () => {
    const s = normalizeAiSlide({
      layout: 'content',
      heading: '项目背景',
      blocks: [{ type: 'bullets', data: { items: ['a'] } }]
    })
    expect(s.blocks[0].type).toBe('heading')
    expect((s.blocks[0].data as unknown as { text: string }).text).toBe('项目背景')
    expect(s.layout).toBe('content')
  })

  it('does not duplicate heading when blocks already contain one', () => {
    const s = normalizeAiSlide({
      layout: 'content',
      heading: '标题',
      blocks: [
        { type: 'heading', data: { text: '已有' } },
        { type: 'bullets', data: { items: [] } }
      ]
    })
    const headings = s.blocks.filter((b) => b.type === 'heading')
    expect(headings.length).toBe(1)
    expect((headings[0].data as unknown as { text: string }).text).toBe('已有')
  })

  it('falls back to content layout for unknown layout', () => {
    const s = normalizeAiSlide({ layout: 'nope', heading: 'x', blocks: [] })
    expect(s.layout).toBe('content')
  })

  it('ensureLayoutCompat upgrades to content when a block is not allowed', () => {
    const s: Slide = {
      id: 'x',
      layout: 'stats',
      blocks: [
        { id: 'h', type: 'heading', data: { text: '标题' }, span: 12, scale: 1 },
        { id: 'b', type: 'bullets', data: { items: ['a'] }, span: 12, scale: 1 }
      ]
    }
    const out = ensureLayoutCompat(s)
    expect(out.layout).toBe('content')
    expect(out.blocks.length).toBe(2)
  })

  it('finalizeAiSlide keeps heading and validates slide', () => {
    const s = finalizeAiSlide({
      layout: 'stats',
      heading: '指标',
      blocks: [{ type: 'kpi-row', data: { items: [{ value: '1', label: 'x' }] } }]
    })
    expect(s.blocks.some((b) => b.type === 'heading')).toBe(true)
    expect(s.blocks.some((b) => b.type === 'kpi-row')).toBe(true)
    expect(s.blocks[0].id).toBeTruthy()
  })
})

describe('presentation fitSlide (overflow guard)', () => {
  function denseSlide(): Slide {
    return {
      id: 's1',
      layout: 'content',
      blocks: [
        { id: 'h', type: 'heading', data: { text: '标题' }, span: 12, scale: 1 },
        {
          id: 'b',
          type: 'bullets',
          data: { items: ['一', '二', '三', '四', '五', '六', '七', '八', '九'] },
          span: 12,
          scale: 1
        }
      ]
    }
  }

  it('caps bullets to 6 and truncates long items', () => {
    const s = fitSlide(denseSlide())
    const b = s.blocks.find((x) => x.type === 'bullets') as never as { data: { items: string[] } }
    expect(b.data.items.length).toBe(6)
  })

  it('caps table rows/cols', () => {
    const s: Slide = {
      id: 't',
      layout: 'content',
      blocks: [
        {
          id: 'tb',
          type: 'table',
          data: {
            columns: Array.from({ length: 8 }, (_, i) => ({ id: `c${i}` })),
            rows: Array.from({ length: 12 }, (_, i) => ({
              cells: Array.from({ length: 8 }, (_, j) => ({ content: `${i}-${j}` }))
            }))
          },
          span: 12,
          scale: 1
        }
      ]
    }
    const out = fitSlide(s)
    const t = out.blocks[0].data as unknown as { columns: unknown[]; rows: unknown[] }
    expect(t.columns.length).toBe(5)
    expect(t.rows.length).toBe(7)
  })

  it('dense content downgrades body scale to 0.85 (heading unchanged)', () => {
    const out = fitSlide(denseSlide())
    const heading = out.blocks.find((x) => x.type === 'heading')
    const bullets = out.blocks.find((x) => x.type === 'bullets')
    expect(heading?.scale).toBe(1)
    expect(bullets?.scale).toBe(0.85)
  })

  it('does not mutate the input slide', () => {
    const slide = denseSlide()
    const before = JSON.stringify(slide)
    fitSlide(slide)
    expect(JSON.stringify(slide)).toBe(before)
  })
})
