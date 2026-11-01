import { describe, it, expect } from 'vitest'
import { findHistoryParams, listHistoryParams } from '@/utils/resolveParams'

const v = (versionNum: string, items: any[], createdAt = '2026-10-01 10:00:00') => ({
  versionNum,
  createdAt,
  items
})
const dev = (model: string, mergedText: string, productId = 'p-1') => ({
  rowType: 'device',
  model,
  mergedText,
  productId
})
const subnet = (name: string) => ({ rowType: 'subnet', name, mergedText: '', productId: '' })

describe('findHistoryParams — 本清单历史版本参数优先', () => {
  it('命中同型号最近版本的非空参数（新→旧取首个）', () => {
    const history = [
      v('v2.0', [dev('HW-5500', '2、新参数要求', 'p-2')]),
      v('v1.0', [dev('HW-5500', '1、旧参数要求', 'p-1')])
    ]
    const hit = findHistoryParams(history, 'hw-5500')
    expect(hit?.mergedText).toBe('2、新参数要求')
    expect(hit?.productId).toBe('p-2')
    expect(hit?.versionNum).toBe('v2.0')
  })

  it('型号匹配大小写不敏感', () => {
    const history = [v('v1.0', [dev('HW-5500', '1、参数')])]
    expect(findHistoryParams(history, ' hw-5500 ')?.mergedText).toBe('1、参数')
  })

  it('跳过参数为空的行，继续向更早版本找', () => {
    const history = [
      v('v2.0', [dev('HW-5500', '')]),
      v('v1.0', [dev('HW-5500', '1、更早但有参数')])
    ]
    expect(findHistoryParams(history, 'HW-5500')?.mergedText).toBe('1、更早但有参数')
  })

  it('子网行不参与匹配', () => {
    const history = [v('v1.0', [subnet('监控子网'), dev('NVR-8', '1、nvr 参数')])]
    expect(findHistoryParams(history, '监控子网')).toBeNull()
    expect(findHistoryParams(history, 'NVR-8')?.mergedText).toBe('1、nvr 参数')
  })

  it('无历史 / 型号未命中 / 空入参 → null', () => {
    expect(findHistoryParams(null, 'HW-5500')).toBeNull()
    expect(findHistoryParams([], 'HW-5500')).toBeNull()
    expect(findHistoryParams([v('v1.0', [dev('HW-5500', '1、参数')])], 'OTHER')).toBeNull()
    expect(findHistoryParams([v('v1.0', [dev('HW-5500', '1、参数')])], '')).toBeNull()
    expect(findHistoryParams([v('v1.0', null as any)], 'HW-5500')).toBeNull()
  })
})

describe('listHistoryParams — 收集所有版本的命中供点选', () => {
  it('每个含同型号的版本各出一条（新→旧），参数为空的版本跳过', () => {
    const history = [
      v('v3.0', [dev('HW-5500', '3、三版参数', 'p-3')]),
      v('v2.0', [dev('HW-5500', '')]),
      v('v1.0', [dev('HW-5500', '1、一版参数', 'p-1')]),
      v('v0.9', [dev('OTHER', '无关')])
    ]
    const hits = listHistoryParams(history, 'HW-5500')
    expect(hits.map((h) => h.versionNum)).toEqual(['v3.0', 'v1.0'])
    expect(hits[0].mergedText).toBe('3、三版参数')
    expect(hits[1].productId).toBe('p-1')
  })

  it('同版本多行命中只取首个；与 findHistoryParams 首条一致', () => {
    const history = [v('v2.0', [dev('HW-5500', '2a'), dev('HW-5500', '2b')])]
    const hits = listHistoryParams(history, 'hw-5500')
    expect(hits).toHaveLength(1)
    expect(hits[0].mergedText).toBe('2a')
    expect(findHistoryParams(history, 'HW-5500')?.mergedText).toBe(hits[0].mergedText)
  })

  it('空入参 → 空数组', () => {
    expect(listHistoryParams(null, 'HW-5500')).toEqual([])
    expect(listHistoryParams([], 'HW-5500')).toEqual([])
  })
})
