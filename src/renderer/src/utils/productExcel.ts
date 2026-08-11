/**
 * 产品库 Excel 导出工具
 *
 * 导入逻辑已迁移到 ExcelImportWizard + excelRaw.ts，本文件仅保留导出。
 */

import * as ExcelJSModule from 'exceljs/dist/exceljs.min.js'

const ExcelJS = (ExcelJSModule as any).default || (ExcelJSModule as any).default?.default || ExcelJSModule

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
