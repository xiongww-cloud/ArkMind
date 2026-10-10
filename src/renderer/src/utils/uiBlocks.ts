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
  const keyOf = (i: number) => {
    const idf = i < orig.length ? String(orig[i][b.columns[0].key] ?? '').trim() : ''
    return idf ? `「${idf}」` : `第${i + 1}行`
  }
  const parts: string[] = []
  state.forEach((row, i) => {
    if (row.deleted) { parts.push(`- 删除${i < orig.length ? `${keyOf(i)}行` : '新增行'}`); return }
    if (i >= orig.length) return
    const changes: string[] = []
    for (const c of b.columns) {
      const o = String(orig[i][c.key] ?? ''), n = String(row.cells[c.key] ?? '').trim()
      if (n !== o) changes.push(`${c.label}=${n || '（清空）'}`)
    }
    if (changes.length) parts.push(`- 修改${keyOf(i)}行：${changes.join('；')}`)
  })
  for (const r of state.slice(orig.length).filter(x => !x.deleted)) {
    parts.push(`- 新增行：${b.columns.map(c => `${c.label}=${String(r.cells[c.key] ?? '').trim() || '?'}`).join('，')}`)
  }
  if (!parts.length) return null
  return [`【组件提交·清单修订】${b.title}`, ...parts].join('\n')
}

// ─── 交互切分：ui-block fence + GFM 表格段（代码围栏内一律不识别） ───────────

export type ISeg = { kind: 'md' | 'block' | 'table'; text: string }

const TABLE_SEP_RE = /^\s*\|?[\s:|-]+\|?\s*$/

/** 逐行扫描切分：闭合 ui-block fence → block；围栏外连续 | 行块（≥2 行且第二行为分隔行）→ table；其余归 md */
export function splitInteractive(raw: string): ISeg[] {
  const segs: ISeg[] = []
  const lines = (raw || '').split('\n')
  let mdBuf: string[] = []
  const flushMd = () => { if (mdBuf.length) { segs.push({ kind: 'md', text: mdBuf.join('\n') }); mdBuf = [] } }
  let i = 0
  let inFence = false
  let fenceLang = ''
  while (i < lines.length) {
    const line = lines[i]
    const fence = line.trimStart().match(/^```(\S*)/)
    if (fence) {
      if (!inFence) {
        inFence = true; fenceLang = fence[1] || ''
        if (fenceLang === 'ui-block') {
          const body: string[] = []
          i++
          while (i < lines.length && !lines[i].trimStart().startsWith('```')) { body.push(lines[i]); i++ }
          if (i < lines.length) i++ // 闭合行
          inFence = false
          flushMd()
          segs.push({ kind: 'block', text: body.join('\n').trim() })
          continue
        }
        mdBuf.push(line); i++; continue
      }
      inFence = false
      mdBuf.push(line); i++; continue
    }
    if (inFence) { mdBuf.push(line); i++; continue }
    // 表格块：本行 | 开头 + 下一行是分隔行 + 再下一行 | 开头（至少一行数据）
    if (line.trimStart().startsWith('|') && i + 2 < lines.length + 1
      && lines[i + 1] && TABLE_SEP_RE.test(lines[i + 1]) && lines[i + 1].includes('|')
      && lines[i + 2] && lines[i + 2].trimStart().startsWith('|')) {
      const tbl: string[] = [line, lines[i + 1]]
      let j = i + 2
      while (j < lines.length && lines[j].trimStart().startsWith('|')) { tbl.push(lines[j]); j++ }
      flushMd()
      segs.push({ kind: 'table', text: tbl.join('\n') })
      i = j
      continue
    }
    mdBuf.push(line); i++
  }
  flushMd()
  if (!segs.length) segs.push({ kind: 'md', text: raw })
  return segs
}

export function hasInteractive(raw: string): boolean {
  return /(^|\n)```ui-block/.test(raw || '') || /(^|\n)\|/.test(raw || '')
}

/** GFM 表格源文本 → 行列单元格（去首尾竖线、trim）；非法结构返回 null（降级普通 markdown） */
export function parseMarkdownTable(src: string): { headers: string[], rows: string[][] } | null {
  const lines = src.trim().split('\n')
  if (lines.length < 3) return null
  const cells = (l: string) => l.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map(c => c.trim())
  const headers = cells(lines[0])
  if (!headers.length) return null
  const rows = lines.slice(2).map(cells).map(r => { while (r.length < headers.length) r.push(''); return r.slice(0, headers.length) })
  if (!rows.length) return null
  return { headers, rows }
}
