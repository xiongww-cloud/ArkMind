/** 型号内联补全候选（幽灵预览用） */
export interface ModelSuggestion {
  name: string
  id: string
  price?: number
}

/**
 * 产品库前缀匹配（幽灵预览候选）：大小写不敏感、**不做 trim**——
 * 前后有空格即视为未命中（幽灵尾缀按原文长度切片，带空格会对不齐）；
 * 命中首个产品（与旧内联补全的选择一致）。
 */
export function findModelSuggestion(
  typed: string,
  products: { id?: string | number; name?: string; price?: number }[] | null | undefined
): ModelSuggestion | null {
  const value = typed || ''
  if (!value || !Array.isArray(products)) return null
  const lower = value.toLowerCase()
  const match = products.find((p) => (p.name || '').toLowerCase().startsWith(lower))
  if (!match || !match.name) return null
  return { name: match.name, id: String(match.id ?? ''), price: match.price }
}
