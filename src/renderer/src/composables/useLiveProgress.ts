import { ref, onScopeDispose, watch } from 'vue'

/**
 * "已确认批次数" 的平滑外推显示值。
 * 背景：文档链路 provider 常把大工具参数攒到批末整段下发，confirmed 在批间长时间静止，
 * 观感如卡死。本 composable 按上一批实测速率做保守外推：
 * - 显示值单调不回退（真值向下修正时保持不动，避免跳回观感）
 * - 外推封顶 90s×rate，批次间隔再长也只是"暂停"而非无限吹牛
 * - 真值跳升时立即跟进并以其为新基线；速率取历史实测（5~400 字/s clamp）
 * - 非生成交互态（getEstimable=false，如工具执行/思考中）停在外推暂停、只跟随真值
 */
export function useLiveProgress(
  getTrue: () => number,
  getActive: () => boolean,
  getEstimable: () => boolean,
) {
  const est = ref(0)
  let lastTrue = -1
  let lastTrueAt = Date.now()
  let rate = 20

  const sync = (now: number) => {
    const t = getTrue()
    if (t !== lastTrue) {
      if (lastTrue >= 0 && t > lastTrue) {
        const dt = Math.max(1, (now - lastTrueAt) / 1000)
        rate = Math.max(5, Math.min(400, (t - lastTrue) / dt))
      }
      lastTrue = t
      lastTrueAt = now
    }
    return t
  }

  const iv = setInterval(() => {
    const now = Date.now()
    const t = sync(now)
    if (!getActive()) {
      est.value = 0
      lastTrue = -1
      return
    }
    if (!getEstimable()) {
      est.value = t
      return
    }
    est.value = Math.max(est.value, Math.min(t, est.value), t + rate * Math.min(90, (now - lastTrueAt) / 1000))
  }, 500)

  // 桶源切换（多 agent 轮替）时重置单调记忆，避免带着上一个 agent 的高度
  watch(getTrue, (nv, ov) => { if (nv < (ov ?? 0)) { est.value = nv; lastTrue = -1 } })

  onScopeDispose(() => clearInterval(iv))
  return est
}
