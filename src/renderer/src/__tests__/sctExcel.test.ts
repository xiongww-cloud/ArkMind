import { describe, it, expect } from 'vitest'
import { detectSCTSheet, buildSCTMapping, classifySCTRow, parseSCTSheet } from '../utils/sctExcel'
import { parseHardwareName } from '../utils/parseHardwareName'
import type { RawDataRow, SheetData } from '../utils/excelRaw'

function makeRow(rowNumber: number, cells: string[]): RawDataRow {
  return { rowNumber, cells, merges: [] }
}

// 模拟 SCT 报价单：中文表头（用户手动选中）
const sctHeaders = [
  '',
  '序号',
  '部件编码',
  '型号',
  '描述',
  '单套数量',
  '总数',
  '成交单价 (CNY)',
  '成交总价 (CNY)',
  ''
]

function makeSCTSheet(rows: RawDataRow[]): SheetData {
  return {
    name: '报价单',
    headerRowNumber: 2,
    headers: sctHeaders,
    rows,
    maxCols: 10,
    mergeMap: new Map()
  }
}

describe('sctExcel.detectSCTSheet', () => {
  it('中文表头组合（序号+型号+部件编码）识别为 SCT', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.1,',
        '1',
        'OceanStor Dorado 5000_1 V700R001_站点1',
        '',
        '',
        '',
        '',
        '94188.86',
        '94188.86',
        ''
      ])
    ])
    expect(detectSCTSheet(sheet)).toBe(true)
  })

  it('首列元数据 blob 命中识别为 SCT（≥2 行）', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.621216255_1227478101,productcfgid.621216255,',
        '1',
        '名称1',
        '',
        '',
        '',
        '',
        '100',
        '100',
        ''
      ]),
      makeRow(4, [
        'sitecfgid.621216255_1227478101,sbomid3.44624242,',
        '',
        '88142DVM',
        '88142DVM',
        '描述',
        '1',
        '1',
        '',
        '',
        ''
      ])
    ])
    expect(detectSCTSheet(sheet)).toBe(true)
  })

  it('普通方案表不误判', () => {
    const sheet: SheetData = {
      name: 'S1',
      headerRowNumber: 1,
      headers: ['型号', '数量', '单价'],
      rows: [makeRow(2, ['A-100', '3', '500'])],
      maxCols: 3,
      mergeMap: new Map()
    }
    expect(detectSCTSheet(sheet)).toBe(false)
  })
})

describe('sctExcel.buildSCTMapping', () => {
  it('动态列映射：按关键词定位 sn/deviceName/model/price/totalPrice', () => {
    const sheet = makeSCTSheet([])
    const m = buildSCTMapping(sheet)
    expect(m[1]).toBe('sn')
    expect(m[2]).toBe('deviceName')
    expect(m[3]).toBe('model')
    expect(m[7]).toBe('price') // 成交单价 (CNY)
    expect(m[8]).toBe('totalPrice') // 成交总价 (CNY)
  })

  it('关键词找不到时回退到 SCT 固定列布局', () => {
    const sheet: SheetData = {
      name: '报价单',
      headerRowNumber: 2,
      headers: ['', '', '', '', '', '', '', '', '', ''],
      rows: [],
      maxCols: 10,
      mergeMap: new Map()
    }
    const m = buildSCTMapping(sheet)
    expect(m[1]).toBe('sn')
    expect(m[2]).toBe('deviceName')
    expect(m[7]).toBe('price')
    expect(m[8]).toBe('totalPrice')
  })
})

describe('sctExcel.classifySCTRow', () => {
  const mapping = buildSCTMapping(makeSCTSheet([]))

  it('分组行（序号有数字）→ device', () => {
    const row = makeRow(3, [
      'sitecfgid.1,',
      '2',
      'OceanStor Dorado 5000',
      '',
      '',
      '',
      '',
      '100',
      '200',
      ''
    ])
    expect(classifySCTRow(row, mapping)).toBe('device')
  })

  it('明细行（sbomid 标记）→ ignore', () => {
    const row = makeRow(4, [
      'sitecfgid.1,sbomid3.44624242,',
      '',
      '88142DVM',
      '88142DVM',
      '描述',
      '1',
      '1',
      '',
      '',
      ''
    ])
    expect(classifySCTRow(row, mapping)).toBe('ignore')
  })

  it('汇总行（productTotalPrice 标记）→ ignore', () => {
    const row = makeRow(30, [
      ',productTotalPrice.1,partType.1,',
      '',
      '',
      '设备总价',
      '',
      '',
      '',
      '',
      '',
      '2937413.48'
    ])
    expect(classifySCTRow(row, mapping)).toBe('ignore')
  })

  it('空行 → ignore', () => {
    const row = makeRow(31, ['', '', '', '', '', '', '', '', '', ''])
    expect(classifySCTRow(row, mapping)).toBe('ignore')
  })
})

describe('sctExcel.parseSCTSheet', () => {
  it('硬件分组：型号清洗 + 单价 + 数量推导', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.1,',
        '2',
        'OceanStor Dorado 5000_1 V700R001_站点1',
        '',
        '',
        '',
        '',
        '1326920.06',
        '2653840.12',
        ''
      ])
    ])
    const { hardwareItems, serviceItems } = parseSCTSheet(sheet)
    expect(hardwareItems).toHaveLength(1)
    expect(serviceItems).toHaveLength(0)
    expect(hardwareItems[0].cleanModel).toBe('OceanStor Dorado')
    expect(hardwareItems[0].customName).toBe('站点1')
    expect(hardwareItems[0].unitPrice).toBe(1326920.06)
    expect(hardwareItems[0].quantity).toBe(2) // 2653840.12 / 1326920.06 = 2
  })

  it('单价为 0 时数量回退为 1', () => {
    const sheet = makeSCTSheet([
      makeRow(3, ['sitecfgid.1,', '3', 'DCS虚拟化套件 8_站点1', '', '', '', '', '', '', ''])
    ])
    const { hardwareItems } = parseSCTSheet(sheet)
    expect(hardwareItems[0].quantity).toBe(1)
    expect(hardwareItems[0].cleanModel).toBe('DCS虚拟化套件 8')
  })

  it('服务分组：生成服务父项 + 收集下方明细行作为子项', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.1,',
        '1',
        '灾备服务(站点内报价)_站点1',
        '',
        '',
        '',
        '',
        '94188.86',
        '94188.86',
        ''
      ]),
      makeRow(4, [
        'sitecfgid.1,sbomid3.1,',
        '',
        '88142DVM',
        '88142DVM',
        '传统容灾设计服务 - （双活）',
        '1',
        '1',
        '',
        '',
        ''
      ]),
      makeRow(5, [
        ',productTotalPrice.1,partType.1,',
        '',
        '',
        '设备总价',
        '',
        '',
        '',
        '',
        '',
        '2937413.48'
      ])
    ])
    const { hardwareItems, serviceItems } = parseSCTSheet(sheet)
    expect(hardwareItems).toHaveLength(0)
    expect(serviceItems).toHaveLength(1)
    expect(serviceItems[0].name).toContain('灾备服务')
    expect(serviceItems[0].totalAmount).toBe(94188.86)
    expect(serviceItems[0].subItems).toHaveLength(1)
    expect(serviceItems[0].subItems[0].content).toBe('传统容灾设计服务 - （双活）')
  })

  it('汇总行（设备总价）不作为服务子项', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.1,',
        '1',
        '灾备服务(站点内报价)_站点1',
        '',
        '',
        '',
        '',
        '94188.86',
        '94188.86',
        ''
      ]),
      makeRow(30, [
        ',productTotalPrice.1,partType.1,',
        '',
        '',
        '设备总价',
        '',
        '',
        '',
        '',
        '',
        '2937413.48'
      ])
    ])
    const { serviceItems } = parseSCTSheet(sheet)
    expect(serviceItems).toHaveLength(0) // 无明细子项 → 服务项被丢弃
  })

  it('合并单元格：当前行单价为空时向上扫描取起始行', () => {
    const sheet = makeSCTSheet([
      makeRow(3, ['sitecfgid.1,', '1', 'OceanStor Dorado 5000', '', '', '', '', '100', '200', '']),
      makeRow(4, [
        'sitecfgid.1,sbomid3.1,',
        '',
        '88142DVM',
        '88142DVM',
        '描述',
        '1',
        '1',
        '',
        '',
        ''
      ])
    ])
    const { hardwareItems } = parseSCTSheet(sheet)
    expect(hardwareItems).toHaveLength(1)
    expect(hardwareItems[0].unitPrice).toBe(100)
    expect(hardwareItems[0].quantity).toBe(2)
  })

  it('金额含千位分隔符/货币符号也能解析', () => {
    const sheet = makeSCTSheet([
      makeRow(3, [
        'sitecfgid.1,',
        '1',
        'OceanStor Dorado 5000',
        '',
        '',
        '',
        '',
        '¥1,326,920.06',
        '2,653,840.12',
        ''
      ])
    ])
    const { hardwareItems } = parseSCTSheet(sheet)
    expect(hardwareItems[0].unitPrice).toBe(1326920.06)
    expect(hardwareItems[0].quantity).toBe(2)
  })
})

describe('parseHardwareName（恢复的清洗函数）', () => {
  it('站点后缀拆为自定义名，型号保留前两词', () => {
    const r = parseHardwareName('OceanStor Dorado 5000_1 V700R001_站点1')
    expect(r.cleanModel).toBe('OceanStor Dorado')
    expect(r.customName).toBe('站点1')
  })

  it('括号内容被删除', () => {
    const r = parseHardwareName('CloudEngine S8700-6(China)_核心交换机')
    expect(r.cleanModel).toBe('CloudEngine S8700-6')
    expect(r.customName).toBe('核心交换机')
  })

  it('无下划线时 customName 为 null', () => {
    const r = parseHardwareName('OceanStor 5310')
    expect(r.cleanModel).toBe('OceanStor 5310')
    expect(r.customName).toBeNull()
  })
})
