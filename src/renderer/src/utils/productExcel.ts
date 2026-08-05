/**
 * 产品库 Excel 导入/导出工具
 *
 * 格式：单 Sheet「产品」，两列：产品名称 | 技术参数
 * 仿 solutionExcel.ts 模式（exceljs）。
 */

import * as ExcelJSModule from 'exceljs/dist/exceljs.min.js'

const ExcelJS = (ExcelJSModule as any).default || (ExcelJSModule as any).default?.default || ExcelJSModule

export interface ProductRow {
  name: string
  fullSpecs: string
}

export interface ProductParseResult {
  rows: ProductRow[]
  error?: string
}

/** 生成产品 Excel（名称 + 技术参数两列），返回可写盘的 ArrayBuffer */
export async function exportProductsToExcel(products: Array<{ name?: string; fullSpecs?: string; full_specs?: string }>): Promise<ArrayBuffer> {
  const wb = new (ExcelJS.Workbook as any)()
  const ws = wb.addWorksheet('产品')

  ws.columns = [
    { header: '产品名称', key: 'name', width: 30 },
    { header: '技术参数', key: 'fullSpecs', width: 80 },
  ]

  const hr = ws.getRow(1)
  hr.font = { bold: true, size: 11, color: { argb: 'FF333333' } }
  hr.alignment = { horizontal: 'center', vertical: 'middle' }
  hr.height = 24
  hr.eachCell((c: any) => {
    if (!c) return
    c.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFE8E8E8' } }
    c.border = {
      top: { style: 'thin' }, bottom: { style: 'thin' },
      left: { style: 'thin' }, right: { style: 'thin' },
    }
  })

  for (const p of products) {
    const name = (p.name || '').trim()
    if (!name) continue
    const specs = (p.fullSpecs || p.full_specs || '').trim()
    const row = ws.addRow({ name, fullSpecs: specs })
    row.alignment = { vertical: 'top', wrapText: true }
  }

  return wb.xlsx.writeBuffer()
}

/** 单元格值转纯文本（处理 exceljs rich text / 公式结果等） */
function cellText(v: any): string {
  if (v == null) return ''
  if (typeof v === 'object') {
    if (Array.isArray(v.richText)) return v.richText.map((t: any) => t.text || '').join('')
    if (v.result != null) return String(v.result)
    if (v.text != null) return String(v.text)
    return ''
  }
  return String(v)
}

/** 解析产品 Excel，跳过表头，名称为空的行忽略 */
export async function parseProductExcel(file: File): Promise<ProductParseResult> {
  try {
    const buffer = await file.arrayBuffer()
    const wb = new (ExcelJS.Workbook as any)()
    await wb.xlsx.load(buffer)
    const ws = wb.worksheets[0]
    if (!ws) return { rows: [], error: '未找到工作表' }

    const rows: ProductRow[] = []
    ws.eachRow((row: any, rowNumber: number) => {
      if (rowNumber === 1) return // 表头
      const name = cellText(row.getCell(1).value).trim()
      if (!name) return
      const fullSpecs = cellText(row.getCell(2).value).trim()
      rows.push({ name, fullSpecs })
    })

    if (rows.length === 0) return { rows: [], error: '未解析到有效产品行（第一列需为产品名称）' }
    return { rows }
  } catch (e: any) {
    return { rows: [], error: 'Excel 解析失败: ' + (e?.message || '文件格式错误') }
  }
}
