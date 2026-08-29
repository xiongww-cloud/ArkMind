import { describe, it, expect } from 'vitest'
import { docToHtml, extractPlain, parseDoc, toDoc } from '@/presentation/richText'
import type { JSONContent } from '@tiptap/core'

describe('presentation rich text', () => {
  it('toDoc splits plain text into paragraphs', () => {
    const doc = toDoc('第一行\n第二行')
    expect(doc.type).toBe('doc')
    expect(doc.content?.length).toBe(2)
  })

  it('docToHtml renders text and marks', () => {
    const doc: JSONContent = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: '普通' },
            { type: 'text', text: '加粗', marks: [{ type: 'bold' }] },
            { type: 'text', text: '斜体', marks: [{ type: 'italic' }] }
          ]
        },
        {
          type: 'bulletList',
          content: [
            {
              type: 'listItem',
              content: [{ type: 'paragraph', content: [{ type: 'text', text: '条目' }] }]
            }
          ]
        }
      ]
    }
    const html = docToHtml(doc)
    expect(html).toContain('普通')
    expect(html).toContain('<strong>加粗</strong>')
    expect(html).toContain('<em>斜体</em>')
    expect(html).toContain('<li>')
  })

  it('extractPlain joins visible text', () => {
    const doc = toDoc('你好\n世界')
    expect(extractPlain(doc)).toBe('你好\n世界')
  })

  it('parseDoc handles plain strings (legacy data)', () => {
    const doc = parseDoc('一段旧文本')
    expect(doc.type).toBe('doc')
    expect(extractPlain(doc)).toBe('一段旧文本')
  })

  it('parseDoc handles invalid json gracefully', () => {
    const doc = parseDoc('{oops')
    expect(doc.type).toBe('doc')
  })

  it('parseDoc returns empty doc for empty string', () => {
    expect(parseDoc('').content?.length ?? 0).toBe(0)
  })
})
