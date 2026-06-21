let _dompurify: any = null

async function getDOMPurify() {
  if (_dompurify) return _dompurify
  const mod = await import('dompurify')
  _dompurify = mod.default || mod
  return _dompurify
}

const PURIFY_CONFIG = {
  ADD_TAGS: ['input'],
  ADD_ATTR: ['checked', 'disabled', 'type'],
}

export async function sanitizeHtml(html: string): Promise<string> {
  if (!html) return ''
  try {
    const DOMPurify = await getDOMPurify()
    return DOMPurify.sanitize(html, PURIFY_CONFIG)
  } catch {
    return html.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  }
}

export function sanitizeHtmlSync(html: string): string {
  if (!html) return ''
  if (_dompurify) return _dompurify.sanitize(html, PURIFY_CONFIG)
  return html.replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
