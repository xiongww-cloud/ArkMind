import { describe, it, expect } from 'vitest'
import { findModelSuggestion } from '../utils/modelSuggestion'

describe('findModelSuggestion', () => {
  const products = [
    { id: 'p1', name: 'SCT-1000', price: 100 },
    { id: 'p2', name: 'sct-2000', price: 200 },
    { id: 'p3', name: '显示器-27寸' },
  ]

  it('前缀命中（双方大小写不敏感）', () => {
    expect(findModelSuggestion('sct', products)?.id).toBe('p1')
    expect(findModelSuggestion('SCT-2', products)?.id).toBe('p2')
    expect(findModelSuggestion('SCT-1000', products)?.name).toBe('SCT-1000')
  })

  it('中文产品名同样命中', () => {
    expect(findModelSuggestion('显示', products)?.id).toBe('p3')
  })

  it('不 trim：前后空格均视为未命中（尾缀切片防错位）', () => {
    expect(findModelSuggestion(' sct', products)).toBeNull()
    expect(findModelSuggestion('sct ', products)).toBeNull()
  })

  it('空串/无命中/空列表 → null', () => {
    expect(findModelSuggestion('', products)).toBeNull()
    expect(findModelSuggestion('zzz', products)).toBeNull()
    expect(findModelSuggestion('sct', null)).toBeNull()
    expect(findModelSuggestion('sct', [])).toBeNull()
  })
})
