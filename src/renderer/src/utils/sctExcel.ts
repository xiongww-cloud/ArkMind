/**
 * 华为 SCT（配置器）Excel 报价单专用解析工具
 *
 * 解析逻辑参考旧版 ServiceFeeAllocator，但复用 excelRaw（SheetJS）的 SheetData 结构：
 *   - 列映射：按中文表头关键词动态定位（不依赖固定列下标）
 *   - 序号行 = 分组起始（硬件或服务父项）
 *   - 服务父项：分组名含服务关键词，下方明细行作为子项
 *   - 合并单元格：单价/总价为空时向上扫描取合并区起始行
 *
 * SCT 报价单典型结构（多 sheet 中第 3 个 sheet）：
 *   - 表头：序号 / 部件编码 / 型号 / 描述 / 单套数量 / 总数 / 成交单价 / 成交总价
 *   - 分组行：序号列有数字，名称跨列合并，成交单价/成交总价垂直合并（rowspan）
 *   - 明细行：首列含 sbomid 标记，无序号
 *   - 汇总行：首列含 productTotalPrice / serviceTotalPrice / projectTotalPrice
 */

import { parseHardwareName } from './parseHardwareName'
import type { RawDataRow, SheetData } from './excelRaw'

// ═══════════════════════════════════════════════════════════════════════════
//  识别信号
// ═══════════════════════════════════════════════════════════════════════════

const SCT_META_RE =
  /^(sitecfgid|productcfgid|sbomid|partnumber|erpid|productTotalPrice|serviceTotalPrice|projectTotalPrice)\b/
const SUMMARY_META_RE = /productTotalPrice|serviceTotalPrice|projectTotalPrice/
const SUMMARY_LABEL_RE = /设备总价|服务总价|项目总价/
const COL_MARKER_RE = /^COL_[A-Z0-9_]+\.\d+$/

/**
 * 判断当前 sheet 是否为华为 SCT 报价单。
 * 信号（任一命中即判定）：
 *   1. 中文表头组合：序号 + 型号 +（部件编码|单套数量|成交单价）
 *   2. 首列元数据 blob：前 15 行中 ≥2 行首列匹配 SCT key-value 逗号串
 *   3. 表头含 COL_xxx.数字 标记（兜底）
 */
export function detectSCTSheet(sheet: SheetData): boolean {
  const hs = sheet.headers.join(',')
  if (
    hs.includes('序号') &&
    hs.includes('型号') &&
    (hs.includes('部件编码') || hs.includes('单套数量') || hs.includes('成交单价'))
  ) {
    return true
  }

  let hits = 0
  for (const row of sheet.rows.slice(0, 15)) {
    const c0 = (row.cells[0] || '').trim()
    if (SCT_META_RE.test(c0) && c0.includes(',')) {
      hits++
      if (hits >= 2) return true
    }
  }

  if (sheet.headers.some((h) => COL_MARKER_RE.test(h))) return true

  return false
}

// ═══════════════════════════════════════════════════════════════════════════
//  列映射（关键词动态检测）
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 表头关键词别名（参考旧版 HEADER_ALIASES）：
 * key → 匹配词列表，按优先级，精确匹配优先、contains 匹配兜底。
 */
const HEADER_ALIASES: Record<string, string[]> = {
  sn: ['序号', 'No', 'no', '#'],
  deviceName: ['部件编码', '设备名称', '设备名', '名称', 'Part', 'part', 'SALECODE'],
  model: ['型号', 'Model', 'model', '产品型号'],
  params: ['描述', '规格', 'Specs', 'specs', 'Description'],
  qty: ['总数', '数量', 'Qty', 'qty'],
  price: ['成交单价', '单价', 'Price', 'price'],
  totalPrice: ['成交总价', '总价', '合计', 'Total', 'total', 'Amount']
}

/** SCT 固定列布局兜底（0-based）：col1=序号 col2=名称(合并) col7=单价 col8=总价 */
const SCT_FIXED_LAYOUT: Record<string, number> = {
  sn: 1,
  deviceName: 2,
  price: 7,
  totalPrice: 8
}

/**
 * 按表头关键词动态定位 SCT 列映射。
 * 返回 headerIndex → fieldKey 的映射；找不到的列不映射。
 * 关键词检测失败的关键字段回退到 SCT 固定列布局。
 */
export function buildSCTMapping(sheet: SheetData): (string | undefined)[] {
  const m: (string | undefined)[] = sheet.headers.map(() => undefined)

  // 第一轮：精确匹配（大小写不敏感）
  const used = new Set<string>()
  for (const [key, aliases] of Object.entries(HEADER_ALIASES)) {
    for (const alias of aliases) {
      const idx = sheet.headers.findIndex(
        (h) => h && String(h).trim().toLowerCase() === alias.toLowerCase()
      )
      if (idx >= 0 && m[idx] === undefined) {
        m[idx] = key
        used.add(key)
        break
      }
    }
  }
  // 第二轮：contains 匹配（未精确命中的字段）
  for (const [key, aliases] of Object.entries(HEADER_ALIASES)) {
    if (used.has(key)) continue
    for (const alias of aliases) {
      const idx = sheet.headers.findIndex((h) => {
        if (!h) return false
        const clean = String(h).trim().toLowerCase()
        return clean.includes(alias.toLowerCase())
      })
      if (idx >= 0 && m[idx] === undefined) {
        m[idx] = key
        used.add(key)
        break
      }
    }
  }

  // 兜底：关键字段未定位时使用 SCT 固定列布局
  for (const [key, col] of Object.entries(SCT_FIXED_LAYOUT)) {
    if (!used.has(key) && col < m.length) {
      if (m[col] === undefined) m[col] = key
    }
  }

  return m
}

// ═══════════════════════════════════════════════════════════════════════════
//  行决策（向导 preset 用）
// ═══════════════════════════════════════════════════════════════════════════

/**
 * SCT 行分类（向导 preset 使用，用于检测阶段的 decisions）：
 *   - 汇总行（productTotalPrice/serviceTotalPrice/projectTotalPrice 或 设备总价等标签）→ ignore
 *   - 分组行（序号列有数字）→ device
 *   - 明细行（首列含 sbomid）→ ignore
 *   - 其余（空行/残余行）→ ignore
 */
export function classifySCTRow(row: RawDataRow, mapping: (string | undefined)[]): string | null {
  const meta0 = (row.cells[0] || '').trim()
  const joined = row.cells.join(',')

  if (SUMMARY_META_RE.test(meta0) || SUMMARY_LABEL_RE.test(joined)) return 'ignore'

  const snIdx = mapping.findIndex((k) => k === 'sn')
  const seqVal = snIdx >= 0 ? (row.cells[snIdx] || '').trim() : (row.cells[1] || '').trim()
  if (seqVal && /^\d+$/.test(seqVal)) return 'device'

  if (meta0.includes('sbomid')) return 'ignore'

  return 'ignore'
}

/** SCT 空行跳过 */
export function shouldSkipSCTRow(row: RawDataRow): boolean {
  return row.cells.every((c) => !c.trim())
}

// ═══════════════════════════════════════════════════════════════════════════
//  单元格读取 / 金额解析
// ═══════════════════════════════════════════════════════════════════════════

/** 单元格数值解析（处理货币格式 "¥1,234.56" / "1,234.56"） */
export function parseAmount(raw: unknown): number {
  if (raw == null) return 0
  if (typeof raw === 'number') return isNaN(raw) ? 0 : raw
  const cleaned = String(raw)
    .replace(/[¥$,，\s]/g, '')
    .trim()
  const num = Number(cleaned)
  return isNaN(num) ? 0 : num
}

/** 按列映射读取单元格文本 */
function cellAt(row: RawDataRow, mapping: (string | undefined)[], fieldKey: string): string {
  const idx = mapping.findIndex((k) => k === fieldKey)
  if (idx < 0 || idx >= row.cells.length) return ''
  return (row.cells[idx] || '').trim()
}

/**
 * 向上扫描合并区起始行获取金额（参考旧版 getMergedCellAmount）。
 * 当前行值为空时，向上找同列非零值；遇上一序号行停止（不跨分组）。
 */
function readUpAmount(rows: RawDataRow[], fromIdx: number, colIdx: number, snIdx: number): number {
  if (colIdx < 0) return 0
  for (let r = fromIdx; r >= 0; r--) {
    const amt = parseAmount(rows[r]?.cells[colIdx])
    if (amt > 0) return amt
    if (r < fromIdx) {
      const seq = (rows[r]?.cells[snIdx] || '').trim()
      if (seq && /^\d+$/.test(seq)) break
    }
  }
  return 0
}

// ═══════════════════════════════════════════════════════════════════════════
//  SCT 解析 → 硬件 / 服务
// ═══════════════════════════════════════════════════════════════════════════

export interface SCTHardwareItem {
  uid: string
  rawName: string
  cleanModel: string
  customName: string | null
  specs: string
  quantity: number
  unit: string
  unitPrice: number
  subtotal: number
}

export interface SCTServiceSubItem {
  content: string
  serviceAmount: number
  isExpanded: boolean
  matchedDevices: Array<{ hwUid: string; cleanModel: string }>
}

export interface SCTServiceItem {
  uid: string
  name: string
  totalAmount: number
  subItems: SCTServiceSubItem[]
}

export interface SCTParseResult {
  hardwareItems: SCTHardwareItem[]
  serviceItems: SCTServiceItem[]
}

/**
 * token 级模糊匹配（参考旧版 fuzzyMatchTokens）：
 * query 的每个 token 需依次包含于 target 对应位置的 token 中，「x」为通配符。
 */
export function fuzzyMatchTokens(query: string, target: string): boolean {
  const qTokens = query.split(/\s+/).filter(Boolean)
  const tTokens = target.split(/\s+/).filter(Boolean)
  if (qTokens.length === 0 || tTokens.length === 0) return false
  if (qTokens.length > tTokens.length) return false

  for (let start = 0; start <= tTokens.length - qTokens.length; start++) {
    let match = true
    for (let k = 0; k < qTokens.length; k++) {
      if (qTokens[k].toLowerCase() === 'x') continue
      if (!tTokens[start + k].toLowerCase().includes(qTokens[k].toLowerCase())) {
        match = false
        break
      }
    }
    if (match) return true
  }
  return false
}

function guid(prefix: string, counter: number): string {
  return `${prefix}-${counter}`
}

/**
 * 解析 SCT 报价单 sheet，产出硬件设备 + 服务父项（含子项）。
 *
 * @param sheet            SCT 报价单 SheetData（用户已选中正确 sheet 与表头）
 * @param serviceKeywords  判定服务父项的关键词（默认不含"规划"）
 */
export function parseSCTSheet(
  sheet: SheetData,
  serviceKeywords: string[] = ['服务', '实施', '支持']
): SCTParseResult {
  const mapping = buildSCTMapping(sheet)
  const snIdx = mapping.findIndex((k) => k === 'sn')
  const rows = sheet.rows

  // 序号行 = 分组起始
  const seqRows: number[] = []
  for (let i = 0; i < rows.length; i++) {
    const seq = snIdx >= 0 ? (rows[i].cells[snIdx] || '').trim() : (rows[i].cells[1] || '').trim()
    if (seq && /^\d+$/.test(seq)) seqRows.push(i)
  }

  const hardwareItems: SCTHardwareItem[] = []
  const serviceItems: SCTServiceItem[] = []
  let uidCounter = 0

  const isSummaryRow = (row: RawDataRow): boolean => {
    const meta0 = (row.cells[0] || '').trim()
    const joined = row.cells.join(',')
    return SUMMARY_META_RE.test(meta0) || SUMMARY_LABEL_RE.test(joined)
  }

  for (let si = 0; si < seqRows.length; si++) {
    const rowIdx = seqRows[si]
    const row = rows[rowIdx]
    const nextSeqIdx = si + 1 < seqRows.length ? seqRows[si + 1] : rows.length

    // 分组名：部件编码（合并主单元格）列优先，型号列兜底
    const rawName =
      cellAt(row, mapping, 'deviceName') ||
      cellAt(row, mapping, 'model') ||
      (row.cells[2] || '').trim()
    if (!rawName) continue

    const isService = serviceKeywords.some((kw) => rawName.includes(kw))

    if (isService) {
      // ── 服务父项 ──────────────────────────────────────────────
      const totalAmount = readUpAmount(
        rows,
        rowIdx,
        mapping.findIndex((k) => k === 'totalPrice'),
        snIdx
      )
      const svc: SCTServiceItem = {
        uid: guid('svc', ++uidCounter),
        name: rawName,
        totalAmount,
        subItems: []
      }
      // 下方明细行直到下一序号行，取"描述"作为子项
      for (let j = rowIdx + 1; j < nextSeqIdx; j++) {
        const subRow = rows[j]
        if (isSummaryRow(subRow)) continue
        const content =
          cellAt(subRow, mapping, 'params') ||
          cellAt(subRow, mapping, 'deviceName') ||
          cellAt(subRow, mapping, 'model') ||
          ''
        if (content) {
          svc.subItems.push({ content, serviceAmount: 0, isExpanded: false, matchedDevices: [] })
        }
      }
      if (svc.subItems.length > 0) {
        serviceItems.push(svc)
      }
    } else {
      // ── 硬件设备 ──────────────────────────────────────────────
      const specs = cellAt(row, mapping, 'params')
      const unitPrice = readUpAmount(
        rows,
        rowIdx,
        mapping.findIndex((k) => k === 'price'),
        snIdx
      )
      const totalPrice = readUpAmount(
        rows,
        rowIdx,
        mapping.findIndex((k) => k === 'totalPrice'),
        snIdx
      )

      // 数量：优先"总数"列；为空则 round(成交总价/成交单价)
      const qtyRaw = cellAt(row, mapping, 'qty')
      let quantity = parseAmount(qtyRaw)
      if (quantity <= 0 && unitPrice > 0 && totalPrice > 0) {
        quantity = Math.round(totalPrice / unitPrice)
      }
      if (quantity <= 0) quantity = 1

      const { cleanModel, customName } = parseHardwareName(rawName)

      hardwareItems.push({
        uid: guid('hw', ++uidCounter),
        rawName,
        cleanModel: cleanModel || rawName,
        customName,
        specs,
        quantity,
        unit: '台',
        unitPrice,
        subtotal: 0
      })
    }
  }

  // 服务子项 → 硬件模糊匹配
  for (const sv of serviceItems) {
    for (const detail of sv.subItems) {
      const query = detail.content.split(/\s+/)[0] || ''
      if (!query) continue
      for (const hw of hardwareItems) {
        if (fuzzyMatchTokens(query, hw.cleanModel || hw.rawName || '')) {
          detail.matchedDevices.push({ hwUid: hw.uid, cleanModel: hw.cleanModel || hw.rawName })
        }
      }
    }
  }

  return { hardwareItems, serviceItems }
}
