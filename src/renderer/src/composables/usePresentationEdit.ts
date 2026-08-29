import { usePresentationHistoryStore } from '@/stores/presentationHistory'

/**
 * 组件内对 Deck 的修改统一入口：
 *   run(mergeKey, label, mutate)
 * 在 mutate 前后捕获整篇快照并写入历史，支持 Ctrl+Z 撤销与打字合并。
 *
 * 注意：mutate 内直接修改响应式 store（保持 WYSIWYG 实时更新），
 * 历史层负责记录 before/after 快照，撤销时整篇回滚。
 */
export function usePresentationEdit(getPid: () => string, getIndex?: () => number) {
  const history = usePresentationHistoryStore()

  function run(key: string, label: string, mutate: () => void) {
    const pid = getPid()
    if (!pid) return
    const scope = getIndex ? `${key}@${getIndex()}` : key
    history.beginSession(scope, pid)
    mutate()
    history.commit(scope, label, pid)
  }

  return { run }
}
