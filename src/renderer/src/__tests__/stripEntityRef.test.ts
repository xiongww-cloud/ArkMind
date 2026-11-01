import { describe, it, expect } from 'vitest'
import { stripEntityRef } from '@/utils/inAppLink'

describe('stripEntityRef — 深链 EntityRef 前缀归一', () => {
  it('TYPE-uuid → 裸 uuid', () => {
    expect(stripEntityRef('PRODUCT-d2da25cf-b005-4792-8082-348b49fc557d')).toBe(
      'd2da25cf-b005-4792-8082-348b49fc557d'
    )
    expect(stripEntityRef('EQUIPMENT_LIST-12345678-abcd-4ef0-1234-56789abcdef0')).toBe(
      '12345678-abcd-4ef0-1234-56789abcdef0'
    )
  })

  it('裸 uuid / 非 EntityRef 原样保留', () => {
    const bare = 'd2da25cf-b005-4792-8082-348b49fc557d'
    expect(stripEntityRef(bare)).toBe(bare)
    expect(stripEntityRef('note-1')).toBe('note-1') // 后段非 uuid，不剥
    expect(stripEntityRef('产品-abc')).toBe('产品-abc')
    expect(stripEntityRef('')).toBe('')
  })
})
