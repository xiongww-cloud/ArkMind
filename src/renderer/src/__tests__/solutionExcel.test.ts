import { describe, it, expect } from 'vitest'
import { buildItemsFromMapping, classifySolutionRow } from '../utils/solutionExcel'
import type { RawDataRow, SheetData } from '../utils/excelRaw'

function makeRow(rowNumber: number, cells: string[]): RawDataRow {
  return { rowNumber, cells, merges: [] }
}

const sheet: SheetData = {
  name: 'S1',
  headerRowNumber: 1,
  headers: ['型号', '数量', '单价'],
  rows: [],
  maxCols: 3,
  mergeMap: new Map(),
}

// mapping: col0=model, col1=qty, col2=price
const mapping = ['model', 'qty', 'price']

describe('solutionExcel.buildItemsFromMapping', () => {
  it('设备行组装：型号/数量/单价/默认单位', () => {
    const rows = [makeRow(2, ['A-100', '3', '500'])]
    const items = buildItemsFromMapping(rows, mapping, [{ action: 'device' }], sheet, [])
    expect(items).toHaveLength(1)
    expect(items[0]).toMatchObject({
      model: 'A-100',
      quantity: 3,
      unitPrice: 500,
      unit: '台',
    })
  })

  it('子网行生成 subnet 条目并记录名称', () => {
    const rows = [makeRow(2, ['核心子网', '', ''])]
    const items = buildItemsFromMapping(rows, mapping, [{ action: 'subnet', subnetName: '核心子网' }], sheet, [])
    expect(items).toHaveLength(1)
    expect(items[0].rowType).toBe('subnet')
    expect(items[0].name).toBe('核心子网')
  })

  it('ignore / problem 行被跳过', () => {
    const rows = [makeRow(2, ['合计', '', '']), makeRow(3, ['A-100', '1', '100'])]
    const items = buildItemsFromMapping(
      rows,
      mapping,
      [{ action: 'ignore' }, { action: 'problem' }],
      sheet,
      []
    )
    expect(items).toHaveLength(0)
  })

  it('型号匹配产品库（大小写不敏感）', () => {
    const rows = [makeRow(2, ['a-100', '1', '100'])]
    const items = buildItemsFromMapping(rows, mapping, [{ action: 'device' }], sheet, [
      { id: 'prod-1', name: 'A-100' },
    ])
    expect(items[0].productId).toBe('prod-1')
  })

  it('数量/单价非法时回退默认值', () => {
    const rows = [makeRow(2, ['A-100', 'abc', 'xyz'])]
    const items = buildItemsFromMapping(rows, mapping, [{ action: 'device' }], sheet, [])
    expect(items[0].quantity).toBe(1)
    expect(items[0].unitPrice).toBe(0)
  })

  it('子网名兜底：无 subnetName 时依次取名称/型号/参数/序号列', () => {
    const m = ['deviceName', 'model', 'params', 'sn']
    const r: RawDataRow = { rowNumber: 2, cells: ['核心机房', '', '', ''], merges: [] }
    expect(buildItemsFromMapping([r], m, [{ action: 'subnet' }], sheet, [])[0].name).toBe('核心机房')
    const r2: RawDataRow = { rowNumber: 2, cells: ['', '', '汇聚区', 'A2'], merges: [] }
    expect(buildItemsFromMapping([r2], m, [{ action: 'subnet' }], sheet, [])[0].name).toBe('汇聚区')
  })

  it('行文本落在未映射/数量列时，取行内首个非空单元格作为子网名', () => {
    const m = ['deviceName', 'model', 'qty']
    const r: RawDataRow = { rowNumber: 2, cells: ['', '', '无线AP区'], merges: [] }
    expect(buildItemsFromMapping([r], m, [{ action: 'subnet' }], sheet, [])[0].name).toBe('无线AP区')
  })
})

describe('solutionExcel.classifySolutionRow — 子网自动识别', () => {
  // mapping: col0=deviceName, col1=model, col2=sn
  const map4 = ['deviceName', 'model', 'sn', 'qty']
  function row(cells: string[], merges: RawDataRow['merges'] = []): RawDataRow {
    return { rowNumber: 2, cells, merges }
  }
  function mergeMaster(col1: number): RawDataRow['merges'][number] {
    return { masterRow: 2, masterCol: col1, rowSpan: 3, colSpan: 1, row: 2, col: col1, isMaster: true }
  }

  it('合计/总计行 → ignore', () => {
    expect(classifySolutionRow(row(['', '', '合计', '']), map4, sheet)).toBe('ignore')
    expect(classifySolutionRow(row(['总计金额', '', '', '']), map4, sheet)).toBe('ignore')
  })

  it('任意列纵向合并主格且无型号 → subnet（不限于序号列）', () => {
    expect(classifySolutionRow(row(['核心区域', '', '', ''], [mergeMaster(1)]), map4, sheet)).toBe('subnet')
    // 旧规则只认序号列——名称列合并现在同样识别
    expect(classifySolutionRow(row(['', '', 'A1', ''], [mergeMaster(3)]), map4, sheet)).toBe('subnet')
  })

  it('无型号且恰好单格文本 → subnet；多格内容 → device（型号可空的真设备）', () => {
    expect(classifySolutionRow(row(['汇聚交换机区', '', '', '']), map4, sheet)).toBe('subnet')
    expect(classifySolutionRow(row(['', '', '', '12']), map4, sheet)).toBe('subnet')
    // 名称+数量+单价多格成行数据 = 无型号设备
    expect(classifySolutionRow(row(['汇聚交换机', '', '', '2']), map4, sheet)).toBe('device')
  })

  it('有型号 → device（即使行带合并）', () => {
    expect(classifySolutionRow(row(['xxx', 'HW-5500', '', '2'], [mergeMaster(4)]), map4, sheet)).toBe('device')
  })

  it('全空行 → ignore（合并块空白中间行不会漏判为子网）', () => {
    expect(classifySolutionRow(row(['', '', '', '']), map4, sheet)).toBe('ignore')
  })
})
