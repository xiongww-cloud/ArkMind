/**
 * 方案配置器 Excel 导入/导出工具
 *
 * 从 SolutionConfigurator.vue 中抽离，便于维护和复用。
 * 导入逻辑已迁移到 ExcelImportWizard + excelRaw.ts，本文件保留：
 *   - DeviceItem / ImportResult / ExportOptions 类型定义
 *   - exportToPMSExcel（导出）
 *   - buildItemsFromMapping（向导确认后组装设备清单）
 */

import * as ExcelJSModule from 'exceljs/dist/exceljs.min.js'
import type { RawDataRow, SheetData } from './excelRaw'

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

/** 按字段映射从原始行读取值 */
function getCellByField(row: RawDataRow, mapping: (string | undefined)[], fieldKey: string): string {
  const idx = mapping.findIndex(k => k === fieldKey)
  if (idx < 0 || idx >= row.cells.length) return ''
  return row.cells[idx].trim()
}

// ═══════════════════════════════════════════════════════════════════════════
//  WIZARD IMPORT — 按映射组装设备清单
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 向导确认后，按映射 + 行决策组装 DeviceItem[]（替代旧 parsePMSExcel）。
 *
 * @param rows       - 向导传入的 RawDataRow[]
 * @param mapping    - 列映射
 * @param decisions  - 每行决策（action: 'device' | 'subnet' | 'ignore'）
 * @param sheet      - Sheet 数据（含 mergeMap）
 * @param productList - 产品库列表（用于型号匹配）
 */
export function buildItemsFromMapping(
  rows: RawDataRow[],
  mapping: (string | undefined)[],
  decisions: { action: string; subnetName?: string }[],
  sheet: SheetData,
  productList: Array<{ id: string; name: string }> = []
): DeviceItem[] {
  const items: DeviceItem[] = []
  // 追踪子网归属：遇到 subnet action 时记录当前子网名
  let currentSubnet: string | null = null

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]
    const decision = decisions[i]
    if (!decision || decision.action === 'ignore' || decision.action === 'problem') continue

    // 子网行
    if (decision.action === 'subnet') {
      const name = decision.subnetName || getCellByField(row, mapping, 'model') || getCellByField(row, mapping, 'sn') || '未命名子网'
      currentSubnet = name
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
        name,
      })
      continue
    }

    // 设备行（action === 'device'）
    const model = getCellByField(row, mapping, 'model')
    const customName = getCellByField(row, mapping, 'deviceName')
    const specs = getCellByField(row, mapping, 'params')
    const qtyStr = getCellByField(row, mapping, 'qty')
    const unitStr = getCellByField(row, mapping, 'unit')
    const priceStr = getCellByField(row, mapping, 'price')

    const qty = parseInt(qtyStr) || 1
    const unit = unitStr || '台'
    const up = parseFloat(priceStr) || 0

    // 产品库匹配
    const match = productList.find(p => (p.name || '').toLowerCase() === model.toLowerCase())

    items.push({
      uid: guid(),
      productId: match?.id || '',
      model,
      customName,
      unitPrice: up,
      quantity: qty,
      unit,
      params: [],
      mergedText: specs || '',
      syncParams: false,
    })
  }

  return items
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
      top: { style: 'thin' },
      bottom: { style: 'thin' },
      left: { style: 'thin' },
      right: { style: 'thin' },
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

    // 不固定行高：单元格 wrapText 时由 Excel/WPS 打开后按内容自适应

    row.eachCell((cell: any, cn: number) => {
      if (!cell) return
      cell.font = { size: 10, color: { argb: 'FF444444' } }
      cell.border = {
        top: { style: 'thin' },
        bottom: { style: 'thin' },
        left: { style: 'thin' },
        right: { style: 'thin' },
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
      top: { style: 'thin' },
      bottom: { style: 'thin' },
      left: { style: 'thin' },
      right: { style: 'thin' },
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
