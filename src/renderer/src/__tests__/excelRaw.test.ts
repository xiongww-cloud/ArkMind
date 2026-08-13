import { describe, it, expect } from 'vitest'
import { guessMapping, detectHeaderRow, type ImportField } from '../utils/excelRaw'

describe('excelRaw.guessMapping', () => {
  const fields: ImportField[] = [
    { key: 'model', label: '型号', guesses: ['型号', 'Model'] },
    { key: 'qty', label: '数量', guesses: ['数量'], numeric: true },
    { key: 'price', label: '单价', guesses: ['单价', '价格'], numeric: true },
  ]

  it('按关键词命中列映射', () => {
    const mapping = guessMapping(['设备型号', '数量', '含税单价'], fields)
    expect(mapping[0]).toBe('model')
    expect(mapping[1]).toBe('qty')
    expect(mapping[2]).toBe('price')
  })

  it('无匹配列映射为 undefined', () => {
    const mapping = guessMapping(['备注', '数量'], fields)
    expect(mapping[0]).toBeUndefined()
    expect(mapping[1]).toBe('qty')
  })

  it('同一字段不会重复分配给多列', () => {
    const mapping = guessMapping(['型号', '产品型号', '数量'], fields)
    expect(mapping[0]).toBe('model')
    expect(mapping[1]).toBeUndefined()
    expect(mapping[2]).toBe('qty')
  })

  it('guess 关键词按优先级取先命中者', () => {
    const f: ImportField[] = [{ key: 'price', label: '单价', guesses: ['单价', '价格'] }]
    const mapping = guessMapping(['价格'], f)
    expect(mapping[0]).toBe('price')
  })
})

describe('excelRaw.detectHeaderRow', () => {
  it('识别文本最多且后续有数据的行', () => {
    const rows = [
      ['报价单', '', ''],
      ['型号', '数量', '单价'],
      ['A-100', '2', '500'],
    ]
    expect(detectHeaderRow(rows)).toBe(2)
  })

  it('首行即表头时返回 1', () => {
    const rows = [
      ['型号', '数量'],
      ['A-100', '2'],
    ]
    expect(detectHeaderRow(rows)).toBe(1)
  })

  it('空数据回退 1', () => {
    expect(detectHeaderRow([])).toBe(1)
    expect(detectHeaderRow([['', '']])).toBe(1)
  })
})
