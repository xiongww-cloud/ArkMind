/**
 * 通用 Excel 原始解析工具（与方案/产品场景无关）
 *
 * 使用 SheetJS（xlsx）解析，同时支持 .xls（旧格式）和 .xlsx（新格式）。
 * 方案导入 / 产品导入各自定义 fields 配置，共用此解析器。
 */

import * as XLSXModule from 'xlsx/xlsx.mjs'

const XLSX = (XLSXModule as any).default || XLSXModule

// ═══════════════════════════════════════════════════════════════════════════
//  TYPES
// ═══════════════════════════════════════════════════════════════════════════

/** 工作表原始解析结果 */
export interface SheetData {
  name: string
  /** 表头行号（1-based） */
  headerRowNumber: number
  /** 表头各列名 */
  headers: string[]
  /** 数据行（跳过表头行） */
  rows: RawDataRow[]
  /** 最大列数 */
  maxCols: number
  /** 所有合并单元格信息，key = "row,col"（1-based） */
  mergeMap: Map<string, MergeInfo>
}

/** 合并单元格信息（在合并范围内的所有行/列都记录，主单元格 isMaster=true） */
export interface MergeInfo {
  masterRow: number
  masterCol: number
  rowSpan: number
  colSpan: number
  /** 当前记录对应的行号 */
  row: number
  /** 当前记录对应的列号 */
  col: number
  isMaster: boolean
}

/** 原始数据行（解析后跳过表头行） */
export interface RawDataRow {
  /** Excel 原始行号（1-based） */
  rowNumber: number
  /** 每个单元格的纯文本值，长度 = maxCols */
  cells: string[]
  /** 该行包含的合并单元格信息 */
  merges: MergeInfo[]
}

/** 完整 Excel 解析结果 */
export interface RawExcelData {
  sheets: SheetData[]
}

// ═══════════════════════════════════════════════════════════════════════════
//  列映射猜测
// ═══════════════════════════════════════════════════════════════════════════

/** 导入目标字段定义 */
export interface ImportField {
  /** 字段内部标识 */
  key: string
  /** 显示名称 */
  label: string
  /** 是否必填（必须映射才能继续） */
  required?: boolean
  /** 列名猜测关键词（按优先级排列，第一个匹配优先） */
  guesses: string[]
  /** 是否为数值列（数量/单价），校验用 */
  numeric?: boolean
  /** 默认值提示（用于行审查时显示） */
  defaultHint?: string
}

/**
 * 根据识别出的表头列名，猜测每列对应的字段映射。
 * 返回 headerIndex → ImportField.key 的映射；无匹配的列映射为 undefined（即忽略）。
 * 冲突去重：已匹配的字段不会重复分配。
 */
export function guessMapping(headers: string[], fields: ImportField[]): (string | undefined)[] {
  const used = new Set<string>()
  const result: (string | undefined)[] = headers.map(() => undefined)
  // 第一轮：精确/包含匹配（按字段优先级）
  for (const field of fields) {
    for (const guess of field.guesses) {
      const idx = headers.findIndex((h, i) => result[i] === undefined && h.includes(guess))
      if (idx >= 0) {
        result[idx] = field.key
        used.add(field.key)
        break
      }
    }
  }
  return result
}

// ═══════════════════════════════════════════════════════════════════════════
//  表头行自动识别
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 在前 maxRows 行中找"文本单元格最多且后续行有数据"的行号。
 * 返回 1-based 行号（找不到回退 1）。
 */
export function detectHeaderRow(rows: any[][], maxRows = 5): number {
  let best = 1
  let bestScore = 0
  for (let r = 0; r < Math.min(maxRows, rows.length); r++) {
    const row = rows[r] || []
    const textCount = row.filter(c => c != null && String(c).trim().length > 0).length
    // 需要下一行也有数据才算
    const nextRow = rows[r + 1] || []
    const nextTextCount = nextRow.filter(c => c != null && String(c).trim().length > 0).length
    const score = textCount + (nextTextCount > 0 ? textCount : 0)
    if (score > bestScore && textCount >= 2) {
      bestScore = score
      best = r + 1
    }
  }
  return best
}

// ═══════════════════════════════════════════════════════════════════════════
//  通用解析（SheetJS）
// ═══════════════════════════════════════════════════════════════════════════

/** SheetJS 单元格转纯文本 */
function cellToText(cell: XLSX.CellObject | undefined): string {
  if (!cell) return ''
  // 数字类型优先用原始值（避免千位分隔符/货币符号导致 parseFloat 失败）
  if (cell.t === 'n' && cell.v != null) return String(cell.v)
  if (cell.w != null) return cell.w.trim()
  if (cell.v != null) {
    if (typeof cell.v === 'boolean') return String(cell.v)
    return String(cell.v).trim()
  }
  return ''
}

/**
 * 通用解析 Excel 文件，返回所有工作表的原始行列数据。
 * 使用 SheetJS 解析，同时支持 .xls 和 .xlsx 格式。
 *
 * @param file        - 用户选择的文件对象
 * @param sheetIndex  - 指定解析哪个 sheet（不传解析全部）
 * @param headerRow   - 指定表头行号（不传自动识别）
 */
export async function parseExcelRaw(
  file: File,
  sheetIndex?: number,
  headerRow?: number
): Promise<RawExcelData> {
  const buffer = await file.arrayBuffer()
  const data = new Uint8Array(buffer)
  const wb = XLSX.read(data, { type: 'array' })

  const sheets: SheetData[] = []
  const sheetNames = wb.SheetNames || []

  const startIdx = sheetIndex !== undefined ? sheetIndex : 0
  const endIdx = sheetIndex !== undefined ? sheetIndex + 1 : sheetNames.length

  for (let wi = startIdx; wi < endIdx; wi++) {
    const ws = wb.Sheets[sheetNames[wi]]
    if (!ws) continue

    // 解析范围
    const ref = ws['!ref'] || 'A1'
    const range = XLSX.utils.decode_range(ref)
    const startRow = range.s.r  // 0-based
    const endRow = range.e.r
    const startCol = range.s.c
    const endCol = range.e.c
    const rowCount = endRow - startRow + 1
    const maxCols = endCol - startCol + 1

    // 收集合并单元格信息
    const mergeMap = new Map<string, MergeInfo>()
    const merges = ws['!merges'] || []
    for (const m of merges) {
      // SheetJS merge: {s: {r, c}, e: {r, c}} — 0-based
      const masterRow0 = m.s.r
      const masterCol0 = m.s.c
      const endRow0 = m.e.r
      const endCol0 = m.e.c
      const rowSpan = endRow0 - masterRow0 + 1
      const colSpan = endCol0 - masterCol0 + 1
      for (let r = masterRow0; r <= endRow0; r++) {
        for (let c = masterCol0; c <= endCol0; c++) {
          // 转为 1-based
          mergeMap.set(`${r + 1},${c + 1}`, {
            masterRow: masterRow0 + 1, masterCol: masterCol0 + 1,
            rowSpan, colSpan,
            row: r + 1, col: c + 1,
            isMaster: r === masterRow0 && c === masterCol0,
          })
        }
      }
    }

    // 把所有行转为二维纯文本数组
    const rawRows: (string[])[] = []
    for (let r = startRow; r <= endRow; r++) {
      const cells: string[] = []
      for (let c = startCol; c <= endCol; c++) {
        const addr = XLSX.utils.encode_cell({ r, c })
        cells.push(cellToText(ws[addr]))
      }
      rawRows.push(cells)
    }

    // 识别表头行
    const headerRowNum = headerRow || detectHeaderRow(rawRows)
    const headers = rawRows[headerRowNum - 1] || []

    // 数据行（跳过表头行）
    const dataRows: RawDataRow[] = []
    for (let r = headerRowNum; r < rowCount; r++) {
      const rawRow = rawRows[r]
      if (!rawRow) continue
      const rowMerges: MergeInfo[] = []
      for (let c = 1; c <= maxCols; c++) {
        const mi = mergeMap.get(`${r + 1},${c}`)
        if (mi) rowMerges.push(mi)
      }
      dataRows.push({
        rowNumber: r + 1, // 1-based
        cells: rawRow,
        merges: rowMerges,
      })
    }

    sheets.push({
      name: sheetNames[wi],
      headerRowNumber: headerRowNum,
      headers,
      rows: dataRows,
      maxCols,
      mergeMap,
    })
  }

  return { sheets }
}
