/**
 * ui-blocks：助手回复中的声明式交互组件。
 * 传输形态为 fenced code block（```ui-block {单行JSON} ```），对 markdown 流式渲染零侵入；
 * 解析失败/未知类型/超限一律降级为普通代码块显示（安全兜底，绝不白屏）。
 * 提交动作 = 把用户编辑结果格式化为结构化消息文本回流会话，由助手调用既有工具执行——前端无任何写路径。
 */

export type UiSeg = { kind: 'md' | 'block'; text: string }

const FENCE_RE = /^```ui-block[^\S\n]*\n([\s\S]*?)^```\s*$/gm

/** 切分原始 markdown：仅匹配「已闭合」的 ui-block fence；流式中间态自然归入 md 段 */
export function splitUiBlocks(raw: string): UiSeg[] {
  const segs: UiSeg[] = []
  let last = 0
  // 双反斜杠构造避免正则字面量歧义：行首 fence、行尾闭合 fence
  const re = new RegExp(FENCE_RE.source, 'gm')
  let m: RegExpExecArray | null
  while ((m = re.exec(raw)) !== null) {
    if (m.index > last) segs.push({ kind: 'md', text: raw.slice(last, m.index) })
    segs.push({ kind: 'block', text: (m[1] || '').trim() })
    last = m.index + m[0].length
  }
  if (last < raw.length) segs.push({ kind: 'md', text: raw.slice(last) })
  if (!segs.length) segs.push({ kind: 'md', text: raw })
  return segs
}

export function hasUiBlocks(raw: string): boolean {
  return /(^|\n)```ui-block/.test(raw || '')
}

export interface UiField {
  name: string
  label: string
  kind: 'text' | 'select' | 'checkbox'
  options?: string[]
  value?: string | boolean
  required?: boolean
  placeholder?: string
}
export interface UiFormBlock { type: 'form'; title: string; submit_label?: string; fields: UiField[] }
export interface UiColumn { key: string; label: string }
export interface UiTableBlock { type: 'table'; title: string; columns: UiColumn[]; rows: Record<string, string>[] }
export type UiBlock = UiFormBlock | UiTableBlock

export const LIMITS = { maxBlocksPerReply: 3, formFields: 8, tableRows: 30, tableCols: 8, textLen: 120 }

function withinLimits(b: any): boolean {
  if (!b || typeof b !== 'object') return false
  if (typeof b.title !== 'string' || !b.title) return false
  const t = (v: unknown) => String(v ?? '').length <= LIMITS.textLen
  if (b.type === 'form') {
    if (!Array.isArray(b.fields) || !b.fields.length || b.fields.length > LIMITS.formFields) return false
    return b.fields.every((f: any) => f && typeof f.name === 'string' && typeof f.label === 'string'
      && ['text', 'select', 'checkbox'].includes(f.kind) && t(f.label)
      && (f.kind !== 'select' || (Array.isArray(f.options) && f.options.length >= 1 && f.options.length <= 8 && f.options.every((o: any) => t(o)))))
  }
  if (b.type === 'table') {
    if (!Array.isArray(b.columns) || !b.columns.length || b.columns.length > LIMITS.tableCols) return false
    if (!Array.isArray(b.rows) || b.rows.length > LIMITS.tableRows) return false
    return b.columns.every((c: any) => c && typeof c.key === 'string' && typeof c.label === 'string' && t(c.label))
  }
  return false
}

/** 解析并校验 block JSON；失败返回 null（渲染端降级代码块） */
export function parseUiBlock(jsonText: string): UiBlock | null {
  try {
    const b = JSON.parse(jsonText)
    return withinLimits(b) ? (b as UiBlock) : null
  } catch { return null }
}

/** 提交态持久：按 消息id+内容指纹 记录，防止页面重进后同一组件重复提交 */
const doneKey = 'uiBlockSubmitted'
let doneSet: Set<string> | null = null
function loadDone(): Set<string> {
  if (!doneSet) {
    try { doneSet = new Set(JSON.parse(localStorage.getItem(doneKey) || '[]')) } catch { doneSet = new Set() }
  }
  return doneSet!
}
export function fnvHash(s: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) }
  return (h >>> 0).toString(36)
}
export function isSubmitted(msgId: string, blockRaw: string): boolean {
  return loadDone().has(`${msgId}:${fnvHash(blockRaw)}`)
}
export function markSubmitted(msgId: string, blockRaw: string): void {
  const set = loadDone()
  set.add(`${msgId}:${fnvHash(blockRaw)}`)
  const arr = [...set]
  // 只留最近 400 条（追加序即时间序）
  try { localStorage.setItem(doneKey, JSON.stringify(arr.slice(-400))) } catch { /* 存储满：内存态仍然生效 */ }
}

/** form 提交 → 结构化消息文本 */
export function formSubmitText(b: UiFormBlock, values: Record<string, string | boolean>): string {
  const lines: string[] = [`【组件提交·表单】${b.title}`]
  for (const f of b.fields) {
    const v = values[f.name]
    if (f.kind === 'checkbox') {
      if (v === true) lines.push(`- ${f.label}：是`)
      else if (v === false && f.required) lines.push(`- ${f.label}：否`)
    } else {
      const s = String(v ?? '').trim()
      if (!s) {
        if (f.required) lines.push(`- ${f.label}：（未填）`)
        continue
      }
      lines.push(`- ${f.label}：${s}`)
    }
  }
  return lines.join('\n')
}

export interface TableRowState { cells: Record<string, string>; deleted: boolean }

/** table diff → 结构化修订消息文本；无改动返回 null（禁提交） */
export function tableDiffText(b: UiTableBlock, orig: Record<string, string>[], state: TableRowState[]): string | null {
  const parts: string[] = []
  let added = 0
  state.forEach((row, i) => {
    if (row.deleted) {
      const idf = i < orig.length ? String(orig[i][b.columns[0].key] ?? '').trim() : ''
      parts.push(`- 删除第${i + 1}行${idf ? `「${idf}」` : ''}`)
      return
    }
    if (i >= orig.length) { added++ ; return }
    const changes: string[] = []
    for (const c of b.columns) {
      const o = String(orig[i][c.key] ?? ''), n = String(row.cells[c.key] ?? '').trim()
      if (n !== o) changes.push(`${c.label}=${n || '（清空）'}`)
    }
    if (changes.length) {
      const idf = String(orig[i][b.columns[0].key] ?? '').trim()
      parts.push(`- 修改第${i + 1}行${idf ? `「${idf}」` : ''}：${changes.join('；')}`)
    }
  })
  const addedRows = state.slice(orig.length).filter(r => !r.deleted)
  if (addedRows.length) {
    for (const r of addedRows) parts.push(`- 新增行：${b.columns.map(c => `${c.label}=${String(r.cells[c.key] ?? '').trim() || '?'}`).join('，')}`)
  }
  if (!parts.length) return null
  return [`【组件提交·清单修订】${b.title}`, ...parts].join('\n')
}
