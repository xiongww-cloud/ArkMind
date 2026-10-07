// 富文本编辑器（Tiptap）存储格式为 HTML；AI 工具（manage_note/manage_todo）写入的是 Markdown。
// 渲染端统一经此函数归一：已是 HTML 原样返回，否则按 Markdown 转 HTML。

let _marked: any = null

async function getMarked() {
  if (_marked) return _marked
  const mod: any = await import('marked')
  _marked = mod.marked || mod.default || mod
  return _marked
}

export async function richContentToHtml(raw: string): Promise<string> {
  const s = (raw || '').trim()
  if (!s) return ''
  if (s.startsWith('<')) return raw
  try {
    const m = await getMarked()
    return String(m.parse(s))
  } catch {
    return raw
  }
}
