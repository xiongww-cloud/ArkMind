import { formatDateTime } from './time'

// ─── Date/Time Formatters ─────────────────────────────────────────

/** Format date with relative prefix (今天/明天) */
export function fmtDate(d?: string): string {
  if (!d) return '无'
  try {
    const date = new Date(d)
    if (isNaN(date.getTime())) return '无'
    const today = new Date(); today.setHours(0, 0, 0, 0)
    const tomorrow = new Date(today); tomorrow.setDate(tomorrow.getDate() + 1)
    const prefix = date.getTime() === today.getTime() ? '今天' : date.getTime() === tomorrow.getTime() ? '明天' : ''
    const datePart = `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}`
    const timePart = d.includes('T') ? ` ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}` : ''
    const full = datePart + timePart
    return prefix ? `${prefix} · ${full}` : full
  } catch { return '无' }
}

/** Format date with Chinese characters (年月日) and relative prefix */
export function fmtDateCn(d?: string): string {
  if (!d) return ''
  try {
    const date = new Date(d)
    if (isNaN(date.getTime())) return ''
    const today = new Date(); today.setHours(0, 0, 0, 0)
    const tomorrow = new Date(today); tomorrow.setDate(tomorrow.getDate() + 1)
    const prefix = date.getTime() === today.getTime() ? '今天' : date.getTime() === tomorrow.getTime() ? '明天' : ''
    const datePart = `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日`
    const timePart = d.includes('T') ? ` ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}` : ''
    const full = datePart + timePart
    return prefix ? `${prefix} · ${full}` : full
  } catch { return '' }
}

/** Relative time (刚刚, 5分钟前, 3小时前, 2天前) */
export function timeAgo(ts: string): string {
  if (!ts) return ''
  try {
    const norm = ts.includes('T') ? ts : ts.replace(' ', 'T')
    const diff = Date.now() - new Date(norm).getTime()
    if (isNaN(diff)) return ts.slice(0, 10)
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return '刚刚'
    if (mins < 60) return `${mins}分钟前`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}小时前`
    const days = Math.floor(hrs / 24)
    if (days < 30) return `${days}天前`
    return ts.slice(0, 10)
  } catch { return ts.slice(0, 10) }
}

/** Short time ago for notes (刚刚, 5 分钟前, 3 小时前, 2 天前) */
export function timeAgoShort(ts: string): string {
  if (!ts) return ''
  try {
    const d = ts.includes('T') ? new Date(ts) : new Date(ts.replace(' ', 'T'))
    const diff = Date.now() - d.getTime()
    if (isNaN(diff)) return ts.slice(0, 16)
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return '刚刚'
    if (mins < 60) return `${mins} 分钟前`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs} 小时前`
    const days = Math.floor(hrs / 24)
    if (days < 7) return `${days} 天前`
    return ts.slice(0, 16)
  } catch { return ts.slice(0, 16) }
}

// ─── Number Formatters ────────────────────────────────────────────

/** Format budget in 万/亿 */
export function formatBudgetWan(v: number | string | undefined): string {
  if (!v || v === 0) return '-'
  const num = typeof v === 'string' ? Number(v) : v
  if (num >= 10000) return `${(num / 10000).toFixed(0)} 亿`
  if (num >= 1) return `¥${num.toLocaleString()} 万`
  return ''
}

// ─── File Formatters ──────────────────────────────────────────────

/** Format bytes to human-readable size */
export function formatSize(bytes: number | undefined): string {
  if (!bytes || bytes === 0) return '--'
  if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(1) + ' GB'
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB'
  if (bytes >= 1024) return (bytes / 1024).toFixed(0) + ' KB'
  return bytes + ' B'
}

const FILE_TYPE_COLORS: Record<string, string> = {
  pdf: 'bg-red-50 text-red-600',
  doc: 'bg-blue-50 text-blue-600', docx: 'bg-blue-50 text-blue-600',
  xls: 'bg-emerald-50 text-emerald-600', xlsx: 'bg-emerald-50 text-emerald-600', csv: 'bg-emerald-50 text-emerald-600',
  ppt: 'bg-orange-50 text-orange-600', pptx: 'bg-orange-50 text-orange-600',
  png: 'bg-purple-50 text-purple-600', jpg: 'bg-purple-50 text-purple-600', jpeg: 'bg-purple-50 text-purple-600',
  gif: 'bg-purple-50 text-purple-600', svg: 'bg-purple-50 text-purple-600', webp: 'bg-purple-50 text-purple-600',
  zip: 'bg-slate-50 text-slate-600', rar: 'bg-slate-50 text-slate-600', '7z': 'bg-slate-50 text-slate-600',
  txt: 'bg-gray-50 text-gray-600', md: 'bg-gray-50 text-gray-600',
}

/** Get Tailwind classes for file type badge */
export function fileTypeColor(ext?: string): string {
  const key = ext?.toLowerCase() || ''
  return FILE_TYPE_COLORS[key] || 'bg-slate-50 text-slate-500'
}

/** Get uppercase file extension label */
export function fileTypeLabel(ext?: string): string {
  return ext?.toUpperCase() || '--'
}

// ─── File Name Resolution ─────────────────────────────────────────

/** Resolve display name from file object with various possible fields */
export function fileDisplayName(f: any): string {
  return f.custom_name || f.global_name || f.displayName || f.name || f.lib_gn || f.lib_cn || f.proj_gn || f.proj_cn || f.prod_gn || f.prod_cn || f.mapping_name || '未命名'
}
