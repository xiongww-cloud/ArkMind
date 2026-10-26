/**
 * 设备参数纯文本的解析 / 序列化 / 检查 / 自动修复。
 * 规范格式：每行 "1、指标：要求"（行首可带 ▲/★ 标记前缀）。
 * 存储真源始终是整段文本；本模块只做文本 ↔ 结构的无损往返与规范化。
 */

export interface ParamRow {
  /** 行首序号（解析自原文；null = 无法识别序号的裸行） */
  num: number | null
  /** 行首标记（▲ 重点 / ★ 废标 / ''） */
  mark: string
  /** 指标名（冒号前） */
  indicator: string
  /** 要求内容（冒号后） */
  requirement: string
  /** 无法按「指标：要求」拆分时的整行原文（serialize 时输出为 N、raw） */
  raw: string
  /** 解析自原文的行首分隔形式（用于 lint 归一检测） */
  sepForm: string
  /** 该行是以半角冒号切分的（lint 用） */
  halfColon: boolean
}

export type LintRule = 'seq' | 'sep' | 'colon' | 'space' | 'punct' | 'ending'

export interface LintIssue {
  rule: LintRule
  /** 1-based 行号（无法定位到行的问题为 0） */
  line: number
  message: string
}

export interface FixSummary {
  seq: number
  sep: number
  colon: number
  space: number
  punct: number
  ending: number
}

const MARK_RE = /^[▲★]/
/** 行首序号写法①：（1） (1) 括号包序号 */
const SEP_PAREN_RE = /^(\s*)([▲★]?)(\s*)([（(])(\d+)[)）](\s*)/
/** 行首序号写法②：1、 1． 1. 1) 1） 1。 */
const SEP_PLAIN_RE = /^(\s*)([▲★]?)(\s*)(\d+)(\s*)(、|．|\.|。|\)|）)(\s*)/

/** 解析整段参数文本为行结构。空行丢弃；无法识别的行保留为 raw 行（不丢数据）。 */
export function parseParams(text: string): ParamRow[] {
  if (!text) return []
  const rows: ParamRow[] = []
  for (const rawLine of String(text).split(/\r?\n/)) {
    const line = rawLine.replace(/\s+$/, '')
    if (!line.trim()) continue
    let mark = ''
    let num: number | null = null
    let body = line
    let sepForm = ''
    const mp = SEP_PAREN_RE.exec(line)
    const mpl = SEP_PLAIN_RE.exec(line)
    const m = mp || mpl
    if (m) {
      num = parseInt(mp ? m[5] : m[4], 10)
      mark = m[2] || ''
      sepForm = mp ? '（）' : m[6]
      body = line.slice(m[0].length)
    } else if (MARK_RE.test(line.trim())) {
      // 带标记但无序号的行：剥掉标记作为 raw 行
      mark = line.trim().match(MARK_RE)![0]
      body = line.trim().slice(mark.length)
    }
    // 以首个全角或半角冒号切分指标 / 要求（谁在前用谁）
    const fullIdx = body.indexOf('：')
    const halfIdx = body.indexOf(':')
    let cut = -1
    let halfColon = false
    if (fullIdx >= 0 && halfIdx >= 0) {
      cut = Math.min(fullIdx, halfIdx)
      halfColon = halfIdx < fullIdx
    } else if (halfIdx >= 0) {
      cut = halfIdx
      halfColon = true
    } else {
      cut = fullIdx
    }
    if (cut >= 0) {
      rows.push({
        num,
        mark,
        indicator: body.slice(0, cut).trim(),
        requirement: body.slice(cut + 1).trim(),
        raw: '',
        sepForm,
        halfColon
      })
    } else {
      rows.push({ num, mark, indicator: '', requirement: '', raw: body.trim(), sepForm, halfColon: false })
    }
  }
  return rows
}

/** 行结构 → 规范文本（序号自动连续，分隔统一 "N、"，指标与要求间全角冒号）。 */
export function serializeParams(rows: ParamRow[]): string {
  return rows
    .map((r, i) => {
      const n = i + 1
      if (r.raw) return `${r.mark}${n}、${r.raw}`
      if (!r.indicator && !r.requirement) return ''
      if (!r.indicator) return `${r.mark}${n}、${r.requirement}`
      if (!r.requirement) return `${r.mark}${n}、${r.indicator}`
      return `${r.mark}${n}、${r.indicator}：${r.requirement}`
    })
    .filter((l) => l !== '')
    .join('\n')
}

/** 规范化整段文本（等价于 parse → serialize，幂等）。 */
export function normalizeParams(text: string): string {
  return serializeParams(parseParams(text))
}

/** 两段文本是否「实质一致」（仅格式差异不算差异，供拉取参数时的覆盖判断）。 */
export function paramsEquivalent(a: string, b: string): boolean {
  return normalizeParams(a || '') === normalizeParams(b || '')
}

/** 检查整段文本，返回问题列表（四类规则）。 */
export function lintParams(text: string): LintIssue[] {
  const issues: LintIssue[] = []
  if (!text || !text.trim()) return issues
  const rows = parseParams(text)
  if (rows.length === 0) return issues

  // 1) 序号：缺号 / 重号 / 乱序（裸行不参与序号检查）
  const numbered = rows.filter((r) => r.num !== null)
  let seqOk = true
  for (let i = 0; i < numbered.length; i++) {
    if (numbered[i].num !== i + 1) {
      seqOk = false
      break
    }
  }
  if (numbered.length > 0 && !seqOk) {
    issues.push({
      rule: 'seq',
      line: 0,
      message: `序号不连续（当前：${numbered.map((r) => r.num).join('、')}，应为 1～${numbered.length}）`
    })
  }

  rows.forEach((r, i) => {
    const line = i + 1
    // 2) 行首分隔符未归一为 "N、" / 缺序号
    if (r.num === null) {
      issues.push({ rule: 'sep', line, message: '缺少行首序号「N、」' })
    } else if (r.sepForm !== '、') {
      const cur = r.sepForm === '（）' ? `（${r.num}）` : `${r.num}${r.sepForm}`
      issues.push({ rule: 'sep', line, message: `行首分隔符应为「N、」（当前「${cur}」）` })
    }
    // 3a) 指标与要求之间使用了半角冒号
    if (r.halfColon) {
      issues.push({ rule: 'colon', line, message: '指标与要求之间应使用全角冒号「：」' })
    }
  })

  // 3b) 行尾空格 / 连续空格（对原文逐行查，parse 已丢弃行尾空格信息）
  String(text)
    .split(/\r?\n/)
    .forEach((rawLine, i) => {
      if (/\s+$/.test(rawLine)) {
        issues.push({ rule: 'space', line: i + 1, message: '行尾存在多余空格' })
      }
    })
  if (/ {2,}/.test(text)) {
    issues.push({ rule: 'space', line: 0, message: '存在连续空格' })
  }

  // 4) 全角/半角逗号、分号混用（仅当两者都出现才提示）
  const halfComma = (text.match(/,/g) || []).length
  const fullComma = (text.match(/，/g) || []).length
  if (halfComma > 0 && fullComma > 0) {
    issues.push({
      rule: 'punct',
      line: 0,
      message: `全角「，」(${fullComma}) 与半角「,」(${halfComma}) 混用`
    })
  }
  const halfSemi = (text.match(/;/g) || []).length
  const fullSemi = (text.match(/；/g) || []).length
  if (halfSemi > 0 && fullSemi > 0) {
    issues.push({
      rule: 'punct',
      line: 0,
      message: `全角「；」(${fullSemi}) 与半角「;」(${halfSemi}) 混用`
    })
  }

  // 5) 行尾标点：非末行应以「；」结尾，末行应以「。」结尾
  const visibleLines = String(text)
    .split(/\r?\n/)
    .filter((l) => l.trim())
  visibleLines.forEach((rawLine, i) => {
    const need = i === visibleLines.length - 1 ? '。' : '；'
    if (!rawLine.trimEnd().endsWith(need)) {
      issues.push({ rule: 'ending', line: i + 1, message: `行尾应以「${need}」结尾` })
    }
  })

  return issues
}

/** 一键修复：返回规范化后的文本与各类修复计数。 */
export function fixParams(text: string): { text: string; fixes: FixSummary } {
  const fixes: FixSummary = { seq: 0, sep: 0, colon: 0, space: 0, punct: 0, ending: 0 }
  if (!text || !text.trim()) return { text: text || '', fixes }

  const before = lintParams(text)
  fixes.seq = before.some((i) => i.rule === 'seq') ? 1 : 0
  fixes.sep = before.filter((i) => i.rule === 'sep').length
  fixes.colon = before.filter((i) => i.rule === 'colon').length
  fixes.space = before.filter((i) => i.rule === 'space').length
  fixes.punct = before.filter((i) => i.rule === 'punct').length
  fixes.ending = before.filter((i) => i.rule === 'ending').length

  // 逗号/分号统一：按段内多数派（平票取全角）
  let out = text.replace(/\r\n/g, '\n')
  const pick = (half: string, full: string) => {
    const h = (out.match(new RegExp(`\\${half}`, 'g')) || []).length
    const f = (out.match(new RegExp(full, 'g')) || []).length
    return f >= h ? full : half
  }
  const comma = pick(',', '，')
  const semi = pick(';', '；')
  out = out.replace(/,/g, comma).replace(/，/g, comma)
  out = out.replace(/;/g, semi).replace(/；/g, semi)

  // 序号/分隔符/冒号/空格归一：parse → serialize
  const normalized = normalizeParams(out)

  // 行尾标点归一：非末行「；」，末行「。」（先剥原有结尾标点再补，避免叠标点）
  const lines = normalized.split('\n')
  const withEnding = lines.map((line, i) => {
    const need = i === lines.length - 1 ? '。' : '；'
    const stripped = line.replace(/[。．.;；，,、：:！!？?]+$/, '')
    return stripped ? stripped + need : line
  })

  return { text: withEnding.join('\n'), fixes }
}

// ─── 控标内容清理 ────────────────────────────────────────────────────────────

/** 证明材料关键词：命中所在标点段即删；指标名命中则整行删（要加词改这里） */
export const PROOF_KEYWORDS = ['截图', '证明', '报告', '盖章']

/** 段落分界标点（分隔符随前段走，删段即连分隔符一并删除）。
 * 手写扫描而非 lookbehind 正则：旧版 WebKit（macOS 11–12）不支持 lookbehind，正则字面量会整模块语法错误。 */
const SEG_PUNCTS = '，,；;。、'
function splitByPunct(s: string): string[] {
  const out: string[] = []
  let start = 0
  for (let i = 0; i < s.length; i++) {
    if (SEG_PUNCTS.includes(s[i])) {
      out.push(s.slice(start, i + 1))
      start = i + 1
    }
  }
  if (start < s.length) out.push(s.slice(start))
  return out
}

export interface StripControlResult {
  text: string
  /** 被剥掉的 ★/▲ 符号数（行首标记 + 内嵌字符） */
  marksStripped: number
  /** 被删除的证明材料段数 */
  segmentsRemoved: number
  /** 被整行删除的行数（要求清空或指标名命中关键词） */
  rowsRemoved: number
}

function hitKeyword(s: string): boolean {
  return PROOF_KEYWORDS.some((k) => s.includes(k))
}

/**
 * 删除控标内容：剥 ★/▲ 符号（保留行）；按标点切段删除含证明材料关键词的段。
 * 只删符号与小段文本，绝不删除整条参数行——
 * 指标名或裸行命中关键词时同样只对其内容做段删（指标名无标点，命中即整段去、行仍保留）。
 * 最后自动跑标点修复规则。
 */
export function stripControlContent(text: string): StripControlResult {
  const result: StripControlResult = { text: text || '', marksStripped: 0, segmentsRemoved: 0, rowsRemoved: 0 }
  if (!text || !text.trim()) return result

  const kept: ParamRow[] = []
  for (const row of parseParams(text)) {
    // 1) 剥 ★/▲：行首标记 + 指标/要求/裸行内嵌字符
    let { mark, indicator, requirement, raw } = row
    if (mark) {
      result.marksStripped++
      mark = ''
    }
    const sweep = (s: string) => {
      const hits = (s.match(/[★▲]/g) || []).length
      result.marksStripped += hits
      return hits ? s.replace(/[★▲]/g, '') : s
    }
    indicator = sweep(indicator)
    requirement = sweep(requirement)
    raw = sweep(raw)

    if (raw) {
      // 2) 裸行：同样按段删，剩余重接为 raw；行本身保留
      const segs = splitByPunct(raw).filter((s) => s !== '')
      const left = segs.filter((s) => {
        if (hitKeyword(s)) {
          result.segmentsRemoved++
          return false
        }
        return true
      }).join('')
      kept.push({ ...row, mark, indicator, requirement, raw: left })
      continue
    }

    // 3) 要求按标点切段，命中关键词的段删除（行保留，要求清空也保留指标名行）
    const segments = splitByPunct(requirement).filter((s) => s !== '')
    const survivors: string[] = []
    for (const seg of segments) {
      if (hitKeyword(seg)) result.segmentsRemoved++
      else survivors.push(seg)
    }
    kept.push({ ...row, mark, indicator, requirement: survivors.join('').trim(), raw })
  }

  // 4) 重排序号 + 标点修复规则（全半角多数派、行尾；/。）
  result.text = fixParams(serializeParams(kept)).text
  return result
}
