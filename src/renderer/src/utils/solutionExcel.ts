/**
 * 方案配置器 Excel 导入/导出工具
 *
 * 从 SolutionConfigurator.vue 中抽离，便于维护和复用。
 */

import * as ExcelJSModule from 'exceljs/dist/exceljs.min.js'

const ExcelJS = (ExcelJSModule as any).default || (ExcelJSModule as any).default?.default || ExcelJSModule

// ═══════════════════════════════════════════════════════════════════════════
//  TYPES
// ═══════════════════════════════════════════════════════════════════════════

export interface DeviceItem {
  uid: string
  rowType: 'device' | 'subnet'
  productId: string
  model: string
  customName: string
  unitPrice: number
  quantity: number
  unit: string
  params: any[]
  mergedText: string
  syncParams: boolean
  name?: string
}

export interface ImportResult {
  items: DeviceItem[]
  error?: string
}

export interface ExportOptions {
  projectName: string
  planName: string
  versionLabel: string
  totalAmount: number
  items: DeviceItem[]
}

// ═══════════════════════════════════════════════════════════════════════════
//  HELPERS
// ═══════════════════════════════════════════════════════════════════════════

function guid(): string {
  return Math.random().toString(36).substring(2, 11)
}

// ═══════════════════════════════════════════════════════════════════════════
//  PMS FORMAT IMPORT
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 解析 PMS 格式 Excel 文件，返回设备清单。
 *
 * PMS 格式列布局（无"设备名称"列时）:
 *   A:序号  B:产品型号  C:产品参数  D:数量  E:单位  F:单价  G:小计
 * 含"设备名称"列时:
 *   A:序号  B:设备名称  C:产品型号  D:产品参数  E:数量  F:单位  G:单价  H:小计
 *
 * @param file          - 用户选择的 .xlsx 文件
 * @param productList   - 产品库列表（用于自动匹配型号）
 * @returns 解析结果（设备清单 或 错误信息）
 */
export async function parsePMSExcel(
  file: File,
  productList: Array<{ id: string; name: string; price?: number }> = []
): Promise<ImportResult> {
  try {
    const buffer = await file.arrayBuffer()
    const wb = new (ExcelJS.Workbook as any)()
    await wb.xlsx.load(buffer)
    const ws = wb.worksheets[0]
    if (!ws) return { items: [], error: '未找到工作表' }

    // 解析表头判断是否有"设备名称"列
    const headerRow = ws.getRow(1)
    const headers: string[] = []
    headerRow.eachCell((cell: any) => {
      headers.push(String(cell.value || '').trim())
    })

    const hasCustomName = headers.some((h: string) => h.includes('设备名称'))
    const colMap = hasCustomName
      ? { sn: 1, cn: 2, pm: 3, pr: 4, qty: 5, un: 6, up: 7, st: 8 }
      : { sn: 1, pm: 2, pr: 3, qty: 4, un: 5, up: 6, st: 7 }

    const items: DeviceItem[] = []

    // ── 预扫描：检测 A 列（序号列）纵向合并单元格 → 子网范围 ──
    const subnetRanges: Array<{ name: string; startRow: number; endRow: number }> = []
    for (let r = 2; r <= ws.rowCount; r++) {
      const cell = ws.getRow(r).getCell(colMap.sn)
      if (cell.isMerged && cell.master === cell) {
        const name = String(cell.value || '').trim()
        if (name && !name.includes('合计') && !name.includes('总计')) {
          let endRow = r
          for (let r2 = r + 1; r2 <= ws.rowCount; r2++) {
            const c2 = ws.getRow(r2).getCell(colMap.sn)
            if (c2.isMerged && c2.master === cell) { endRow = r2 } else { break }
          }
          subnetRanges.push({ name, startRow: r, endRow })
        }
      }
    }

    const subnetStartRows = new Set(subnetRanges.map(s => s.startRow))
    const rowToSubnet = new Map<number, { name: string; startRow: number; endRow: number }>()
    for (const range of subnetRanges) {
      for (let r = range.startRow; r <= range.endRow; r++) rowToSubnet.set(r, range)
    }

    for (let r = 2; r <= ws.rowCount; r++) {
      const row = ws.getRow(r)
      const snCell = row.getCell(colMap.sn)
      const snVal = snCell.value

      // 检查合计行
      if (snVal !== null && snVal !== undefined && snVal !== '') {
        const snStr = String(snVal).trim()
        if (snStr.includes('合计') || snStr.includes('总计') || snStr === '合计：') break
      }

      // 子网主单元格 → emit subnet
      if (subnetStartRows.has(r)) {
        const range = rowToSubnet.get(r)!
        items.push({
          uid: guid(),
          rowType: 'subnet',
          productId: '',
          model: '',
          customName: '',
          unitPrice: 0,
          quantity: 0,
          unit: '',
          params: [],
          mergedText: '',
          syncParams: false,
          name: range.name,
        })
        continue
      }

      // 子网从属行或独立设备行
      const subnet = rowToSubnet.get(r)
      const snIsNumeric = snVal !== null && snVal !== undefined && snVal !== '' && !isNaN(Number(snVal))

      // 不在子网范围内且序号不是数字 → 跳过（备注行等）
      if (!subnet && !snIsNumeric) continue

      const pmVal = row.getCell(colMap.pm).value
      const model = String(pmVal || '').trim()
      if (!model) continue

      const customName = hasCustomName
        ? String(row.getCell(colMap.cn).value || '').trim()
        : ''
      const specs = String(row.getCell(colMap.pr).value || '').trim()
      const qty = parseInt(String(row.getCell(colMap.qty).value || '1')) || 1
      const unit = String(row.getCell(colMap.un).value || '台').trim() || '台'
      const up = parseFloat(String(row.getCell(colMap.up).value || '0')) || 0

      const match = productList.find(
        (p) => (p.name || '').toLowerCase() === model.toLowerCase()
      )

      items.push({
        uid: guid(),
        productId: match?.id || '',
        model,
        customName,
        unitPrice: up,
        quantity: qty,
        unit,
        params: [],
        mergedText: specs || (match as any)?.full_specs || (match as any)?.standard_specs || '',
        syncParams: false,
      })
    }

    if (items.length === 0) {
      return { items: [], error: '未解析到有效设备行' }
    }

    return { items }
  } catch (ex: any) {
    return { items: [], error: ex.message || '文件解析失败' }
  }
}

// ═══════════════════════════════════════════════════════════════════════════
//  PMS FORMAT EXPORT
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 将设备清单导出为 PMS 格式 Excel 文件并触发浏览器下载。
 *
 * @param opts - 导出配置（项目名、方案名、版本号、金额、设备清单）
 */
export async function exportToPMSExcel(opts: ExportOptions): Promise<void> {
  const { projectName, planName, versionLabel, totalAmount, items } = opts

  // 动态决定是否包含"设备名称"列
  const hasCustomName = items.some(
    (item) =>
      item.rowType !== 'subnet' &&
      typeof item.customName === 'string' &&
      item.customName.trim() !== ''
  )
  const totalCols = hasCustomName ? 8 : 7
  const paramColIdx = hasCustomName ? 4 : 3

  const wb = new (ExcelJS.Workbook as any)()
  const ws = wb.addWorksheet('报价单')

  ws.columns = hasCustomName
    ? [
        { header: '序号', key: 'sn', width: 5 },
        { header: '设备名称', key: 'cn', width: 15 },
        { header: '产品型号', key: 'pm', width: 15 },
        { header: '产品参数', key: 'pr', width: 65 },
        { header: '数量', key: 'qty', width: 5 },
        { header: '单位', key: 'un', width: 5 },
        { header: '单价', key: 'up', width: 15 },
        { header: '小计', key: 'st', width: 15 },
      ]
    : [
        { header: '序号', key: 'sn', width: 5 },
        { header: '产品型号', key: 'pm', width: 20 },
        { header: '产品参数', key: 'pr', width: 65 },
        { header: '数量', key: 'qty', width: 5 },
        { header: '单位', key: 'un', width: 5 },
        { header: '单价', key: 'up', width: 15 },
        { header: '小计', key: 'st', width: 15 },
      ]

  // 表头样式
  const hr = ws.getRow(1)
  hr.font = { bold: true, size: 11, color: { argb: 'FF333333' } }
  hr.alignment = { horizontal: 'center', vertical: 'middle', wrapText: true }
  hr.height = 28
  hr.eachCell((c: any) => {
    if (!c) return
    c.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFE8E8E8' } }
    c.border = {
      top: { style: 'medium' },
      bottom: { style: 'medium' },
      left: { style: 'medium' },
      right: { style: 'medium' },
    }
  })

  let excelRowNum = 2
  let seqNum = 1

  items.forEach((item) => {
    const ri = excelRowNum
    const row = ws.getRow(ri)

    // 子网分隔行
    if (item.rowType === 'subnet') {
      const label = item.name || '未命名子网'
      for (let c = 1; c <= totalCols; c++) {
        const cell = row.getCell(c)
        cell.value = c === 1 ? label : ''
        cell.font = { bold: true, size: 10, color: { argb: 'FF0066CC' } }
        cell.alignment = { horizontal: 'center', vertical: 'middle', wrapText: true }
        cell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFE6F0FA' } }
        cell.border = {
          top: { style: 'thin' },
          bottom: { style: 'thin' },
          left: { style: 'thin' },
          right: { style: 'thin' },
        }
      }
      const endColLetter = String.fromCharCode(64 + totalCols)
      ws.mergeCells(`A${ri}:${endColLetter}${ri}`)
      row.height = 24
      excelRowNum++
      return
    }

    const pname = item.model || ''
    const pt = item.mergedText || ''
    const st = (item.unitPrice || 0) * (item.quantity || 0)

    row.getCell('sn').value = seqNum++
    if (hasCustomName) row.getCell('cn').value = item.customName || ''
    row.getCell('pm').value = pname
    row.getCell('pr').value = pt
    row.getCell('qty').value = item.quantity || 0
    row.getCell('un').value = item.unit || '台'
    row.getCell('up').value = item.unitPrice || 0
    row.getCell('st').value = st

    const lines = pt.split('\n').reduce((sum: number, l: string) => sum + Math.max(1, Math.ceil(l.length / 45)), 0)
    row.height = lines * 27

    row.eachCell((cell: any, cn: number) => {
      if (!cell) return
      cell.font = { size: 10, color: { argb: 'FF444444' } }
      cell.border = {
        top: { style: 'medium' },
        bottom: { style: 'medium' },
        left: { style: 'medium' },
        right: { style: 'medium' },
      }
      if (cn === paramColIdx) {
        cell.alignment = { horizontal: 'left', vertical: 'middle', wrapText: true }
      } else {
        cell.alignment = { horizontal: 'center', vertical: 'middle', wrapText: true }
      }
    })

    row.getCell('up').numFmt = '#,##0.00'
    row.getCell('st').numFmt = '#,##0.00'
    excelRowNum++
  })

  // 合计行
  const tri = excelRowNum
  const tr = ws.getRow(tri)
  const labelEndCol = totalCols - 2
  const amountStartCol = totalCols - 1
  const labelEndLetter = String.fromCharCode(64 + labelEndCol)
  const amountStartLetter = String.fromCharCode(64 + amountStartCol)
  const endLetter = String.fromCharCode(64 + totalCols)

  ws.mergeCells(`A${tri}:${labelEndLetter}${tri}`)
  ws.mergeCells(`${amountStartLetter}${tri}:${endLetter}${tri}`)

  tr.getCell(1).value = '合计：'
  tr.getCell(1).font = { bold: true, size: 11, color: { argb: 'FF333333' } }
  tr.getCell(1).alignment = { horizontal: 'center', vertical: 'middle' }
  tr.getCell(1).fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFEEEEEE' } }

  tr.getCell(amountStartCol).value = totalAmount || 0
  tr.getCell(amountStartCol).font = { bold: true, size: 10, color: { argb: 'FF444444' } }
  tr.getCell(amountStartCol).alignment = { horizontal: 'center', vertical: 'middle' }
  tr.getCell(amountStartCol).numFmt = '#,##0.00'
  tr.getCell(amountStartCol).fill = {
    type: 'pattern',
    pattern: 'solid',
    fgColor: { argb: 'FFFFFFFF' },
  }

  tr.height = 26

  for (let c = 1; c <= totalCols; c++) {
    const cell = tr.getCell(c)
    if (!cell) continue
    cell.border = {
      top: { style: 'medium' },
      bottom: { style: 'medium' },
      left: { style: 'medium' },
      right: { style: 'medium' },
    }
    if (!cell.fill || typeof cell.fill !== 'object' || !cell.fill.type) {
      cell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFFAFAFA' } }
    }
  }

  const fileName = `${projectName}_${planName}_配置清单_${versionLabel}.xlsx`
  const buf = await wb.xlsx.writeBuffer()
  // 使用 Tauri save dialog 保存文件
  try {
    const { solutionDownloadExcel } = await import('@/api/db-client')
    const array = new Uint8Array(buf)
    const result = await solutionDownloadExcel({ buffer: Array.from(array), fileName })
    if (!result.success && result.error !== 'cancelled') {
      console.warn('导出 Excel 失败:', result.error)
    }
  } catch (e) {
    // Fallback: browser download (for dev mode)
    const { saveAs } = await import('file-saver')
    saveAs(new Blob([buf], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' }), fileName)
  }
}
