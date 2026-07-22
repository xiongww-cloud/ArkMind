export const STATUS_OPTIONS = ['Lead', 'SolutionDesign', 'PoC', 'Bidding', 'Won', 'Ordered', 'Lost', 'OnHold'] as const

export const STATUS_LABELS: Record<string, string> = {
  Lead: '线索', SolutionDesign: '方案报价', PoC: '测试', Bidding: '投标',
  Won: '中标', Ordered: '下单', Lost: '丢单', OnHold: '搁置'
}

export const STATUS_DOT_COLORS: Record<string, string> = {
  Lead: 'bg-blue-300', SolutionDesign: 'bg-cyan-300', PoC: 'bg-purple-300',
  Bidding: 'bg-orange-300', Won: 'bg-emerald-300', Ordered: 'bg-indigo-300',
  Lost: 'bg-red-300', OnHold: 'bg-gray-400'
}

export const STATUS_BADGE_COLORS: Record<string, string> = {
  Lead: 'bg-slate-100 text-slate-600', SolutionDesign: 'bg-cyan-50 text-cyan-600',
  PoC: 'bg-blue-50 text-blue-600', Bidding: 'bg-amber-50 text-amber-700',
  Won: 'bg-emerald-50 text-emerald-600', Ordered: 'bg-violet-50 text-violet-600',
  Lost: 'bg-red-50 text-red-500', OnHold: 'bg-gray-100 text-gray-500'
}

export function statusBadgeClass(status: string) {
  return `badge badge-${(status || 'Lead').toLowerCase()}`
}

export function statusDotClass(status: string) {
  return STATUS_DOT_COLORS[status] || 'bg-gray-400'
}

export function statusLabel(status: string) {
  return STATUS_LABELS[status] || status
}
