/**
 * 清单历史版本参数优先的匹配链。
 *
 * 投标场景里参数是在项目里被调过的（删控标、改指标），这些调整只活在
 * solution_items 快照里；产品库 full_specs 是通用/过期的。因此同型号
 * 回填参数时：本清单历史版本（新→旧）优先，产品库兜底。
 */

export interface HistoryParamHit {
  /** 参数多行文本（solution item 的 mergedText / description） */
  mergedText: string
  /** 命中行所属历史版本的型号 productId（可能为空串） */
  productId: string
  /** 命中行所属版本号（展示用，如 v1.2） */
  versionNum?: string
  /** 命中行所属版本创建时间（展示用） */
  createdAt?: string | null
}

interface VersionLike {
  versionNum?: string
  createdAt?: string | null
  items?: any[] | null
}

/**
 * 在清单历史版本（新→旧顺序）中收集同型号的所有参数命中，每个版本取首个非空行。
 * 拉取弹窗用它把每个版本的参数逐一展示给用户点选。
 */
export function listHistoryParams(
  history: VersionLike[] | null | undefined,
  model: string
): HistoryParamHit[] {
  const m = (model || '').trim().toLowerCase()
  if (!m || !Array.isArray(history)) return []
  const out: HistoryParamHit[] = []
  for (const v of history) {
    const items = Array.isArray(v?.items) ? v.items : []
    for (const it of items) {
      if (!it || it.rowType === 'subnet') continue
      const im = (it.model || '').trim().toLowerCase()
      if (!im || im !== m) continue
      const text = it.mergedText || ''
      if (!text.trim()) continue
      out.push({
        mergedText: text,
        productId: it.productId || '',
        versionNum: v.versionNum,
        createdAt: v.createdAt ?? null
      })
      break // 每个版本只取首个命中行
    }
  }
  return out
}

/** 最近一次命中（自动回填用）：listHistoryParams 的首条 */
export function findHistoryParams(
  history: VersionLike[] | null | undefined,
  model: string
): HistoryParamHit | null {
  return listHistoryParams(history, model)[0] || null
}
