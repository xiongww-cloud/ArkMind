import { generateHTML } from '@tiptap/core'
import type { Extensions, JSONContent } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Underline from '@tiptap/extension-underline'
import TextAlign from '@tiptap/extension-text-align'

/**
 * 富文本共享扩展：段落区块内联编辑与预览渲染使用同一套 schema，
 * 保证"编辑器所见 = 渲染所出"。
 */
export const richExtensions: Extensions = [
  StarterKit.configure({ heading: { levels: [1, 2, 3] } }),
  Underline,
  TextAlign.configure({ types: ['heading', 'paragraph'] })
]

/** 纯文本 → Tiptap doc（多行拆分为多个段落） */
export function toDoc(plain: string): JSONContent {
  return {
    type: 'doc',
    content: plain
      .split('\n')
      .map((line) =>
        line
          ? { type: 'paragraph', content: [{ type: 'text', text: line }] }
          : { type: 'paragraph' }
      )
  }
}

/** 解析存储的 JSON 字符串；非 JSON（旧数据纯文本）自动转 doc */
export function parseDoc(json: string): JSONContent {
  const raw = typeof json === 'string' ? json.trim() : ''
  if (!raw) return { type: 'doc', content: [] }
  if (!raw.startsWith('{')) return toDoc(raw)
  try {
    const d = JSON.parse(raw)
    if (d && typeof d === 'object' && d.type === 'doc') return d as JSONContent
    return { type: 'doc', content: [d] }
  } catch {
    return toDoc(raw)
  }
}

/** doc → HTML（供预览渲染，调用方负责 DOMPurify 消毒） */
export function docToHtml(doc: JSONContent): string {
  try {
    return generateHTML(doc, richExtensions)
  } catch {
    return ''
  }
}

/** 提取纯文本（用于内联编辑回填 / 搜索） */
export function extractPlain(doc: JSONContent): string {
  const parts: string[] = []
  const walk = (n: JSONContent) => {
    if (n.type === 'text') parts.push(n.text ?? '')
    else if (n.content) for (const c of n.content) walk(c)
    if (n.type === 'paragraph' || n.type === 'listItem') parts.push('\n')
  }
  if (doc?.content) for (const c of doc.content) walk(c)
  return parts
    .join('')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}
