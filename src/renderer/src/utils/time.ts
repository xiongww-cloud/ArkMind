/**
 * 全链路时区统一格式化工具
 * 原则：SQLite 存储 UTC，前端显示 Local
 */

const pad = (n: number) => n.toString().padStart(2, '0')

/** 时间标准化：带时区标记的（Z/±HH:MM）保持原样让 JS 正确转换；无时区标记的 SQLite localtime 按本地时间解析 */
function normalizeLocal(raw: string): string {
  let s = raw.trim()
  if (/[-+]\d{2}:\d{2}$/.test(s) || s.endsWith('Z')) {
    // 已有时区 → JS new Date() 自动转换
    return s
  }
  // SQLite datetime('now','localtime') 无时区 → 按本地时间解析
  return s.replace(' ', 'T')
}

/** 完整日期时间：2026-06-07 19:55（本地时区） */
export const formatDateTime = (isoString: string | null | undefined): string => {
  if (!isoString) return ''
  const date = new Date(normalizeLocal(isoString))
  if (isNaN(date.getTime())) return ''
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** 短日期：2026/06/07（本地时区） */
export const formatDate = (isoString: string | null | undefined): string => {
  if (!isoString) return ''
  const date = new Date(normalizeLocal(isoString))
  if (isNaN(date.getTime())) return ''
  return `${date.getFullYear()}/${pad(date.getMonth() + 1)}/${pad(date.getDate())}`
}

/** 相对时间：刚刚 / N 分钟前 / N 小时前 / 超过 24h 切为标准格式 */
export const formatRelativeTime = (isoString: string | null | undefined): string => {
  if (!isoString) return ''
  const date = new Date(normalizeLocal(isoString))
  if (isNaN(date.getTime())) return ''
  const now = Date.now()
  const diff = now - date.getTime()
  const minutes = Math.floor(diff / 60000)
  if (minutes < 1) return '刚刚'
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  return formatDateTime(isoString)
}
