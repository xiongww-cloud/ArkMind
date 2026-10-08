/**
 * 方案配置器 Excel 导入/导出工具
 *
 * 从 SolutionConfigurator.vue 中抽离，便于维护和复用。
 * 导入逻辑已迁移到 ExcelImportWizard + excelRaw.ts，本文件保留：
 *   - DeviceItem / ImportResult / ExportOptions 类型定义
 *   - exportToPMSExcel（导出，后端 rust_xlsxwriter 生成）
 *   - buildItemsFromMapping（向导确认后组装设备清单）
 */

import type { RawDataRow, SheetData } from './excelRaw'

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
//  PMS FORMAT EXPORT — 后端 rust_xlsxwriter 生成（行高自适应 + 公式）
// ═══════════════════════════════════════════════════════════════════════════

/**
 * 将设备清单导出为报价 Excel 文件（保存对话框落盘）。
 * 生成端已迁到后端 doc_writer::build_solution_quote_xlsx：
 * 小计列 =数量×单价 公式、合计行 =SUM 公式（Excel/WPS 改数量自动重算），
 * 全部行不设固定高度 → 按 wrapText 内容自适应。
 */
export async function exportToPMSExcel(opts: ExportOptions): Promise<void> {
  const { projectName, planName, versionLabel, items } = opts

  // 动态决定是否包含"设备名称"列（与后端布局共用同一判据）
  const hasCustomName = items.some(
    (item) =>
      item.rowType !== 'subnet' &&
      typeof item.customName === 'string' &&
      item.customName.trim() !== ''
  )

  const payloadItems = items.map((item) =>
    item.rowType === 'subnet'
      ? { type: 'subnet' as const, label: item.name || '' }
      : {
          type: 'device' as const,
          customName: item.customName || '',
          model: item.model || '',
          params: item.mergedText || '',
          qty: item.quantity || 0,
          unit: item.unit || '台',
          price: item.unitPrice || 0,
        }
  )

  const { invoke } = await import('@tauri-apps/api/core')
  const r = await invoke<{ success: boolean; bufferBase64?: string; error?: string }>('solution_build_excel', {
    data: { hasCustomName, items: payloadItems },
  })
  if (!r?.success || !r.bufferBase64) {
    throw new Error(r?.error || '生成报价单失败')
  }
  const bin = atob(r.bufferBase64)
  const array = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) array[i] = bin.charCodeAt(i)

  const fileName = `${projectName}_${planName}_配置清单_${versionLabel}.xlsx`
  try {
    const { solutionDownloadExcel } = await import('@/api/db-client')
    const result = await solutionDownloadExcel({ buffer: Array.from(array), fileName })
    if (!result.success && result.error !== 'cancelled') {
      console.warn('导出 Excel 失败:', result.error)
    }
  } catch (e) {
    console.warn('导出 Excel 保存失败:', e)
    throw e
  }
}
