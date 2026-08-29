import { describe, it, expect } from 'vitest'
import { getTheme, sampleBlock, validateSlide, uid } from '@/presentation/elements'

describe('presentation schema', () => {
  it('migrates legacy table headers+rows to structured columns+rows', () => {
    const slide = validateSlide({
      id: uid(),
      layout: 'content',
      blocks: [
        {
          id: uid(),
          type: 'table',
          span: 12,
          scale: 1,
          data: {
            headers: ['型号', '参数'],
            rows: [
              ['A', '1'],
              ['B', '2']
            ]
          }
        } as never
      ]
    })
    const t = slide.blocks[0].data as unknown as {
      columns: Array<{ id: string }>
      rows: Array<{ cells: Array<{ content: string }> }>
    }
    expect(t.columns.length).toBe(2)
    // 表头行 + 两行数据
    expect(t.rows.length).toBe(3)
    expect(t.rows[0].cells[0].content).toBe('型号')
    expect(t.rows[1].cells[1].content).toBe('1')
    expect(t.columns[0].id).toBeTruthy()
  })

  it('pads/trims rows to column count', () => {
    const slide = validateSlide({
      id: uid(),
      layout: 'content',
      blocks: [
        {
          id: uid(),
          type: 'table',
          span: 12,
          scale: 1,
          data: {
            columns: [{ id: 'c1' }, { id: 'c2' }, { id: 'c3' }],
            rows: [
              { cells: [{ content: 'A' }] },
              { cells: [{ content: 'x' }, { content: 'y' }, { content: 'z' }, { content: 'w' }] }
            ]
          }
        } as never
      ]
    })
    const t = slide.blocks[0].data as unknown as {
      rows: Array<{ cells: Array<{ content: string }> }>
    }
    expect(t.rows[0].cells.length).toBe(3)
    expect(t.rows[0].cells[1].content).toBe('')
    expect(t.rows[1].cells.length).toBe(3)
  })

  it('invalid block in content layout degrades to paragraph', () => {
    const slide = validateSlide({
      id: uid(),
      layout: 'content',
      blocks: [
        { id: uid(), type: 'unknown-xyz', span: 6, scale: 1, data: { text: '兜底' } } as never
      ]
    })
    expect(slide.blocks[0].type).toBe('paragraph')
  })

  it('blocks missing an id get one assigned', () => {
    const slide = validateSlide({
      id: uid(),
      layout: 'content',
      blocks: [{ id: '', type: 'heading', data: { text: '标题' }, span: 6 } as never]
    })
    expect(slide.blocks[0].id).toBeTruthy()
  })

  it('getTheme falls back to default for unknown id', () => {
    expect(getTheme('nope').id).toBe('default')
    expect(getTheme('dark').id).toBe('dark')
  })

  it('sampleBlock always carries an id and valid span/scale', () => {
    const b = sampleBlock('bullets')
    expect(b.id).toBeTruthy()
    expect(b.span).toBe(12)
    expect(b.scale).toBe(1)
  })
})
