import { describe, it, expect } from 'vitest'
import { buildItemsFromMapping } from '../utils/solutionExcel'
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
})
