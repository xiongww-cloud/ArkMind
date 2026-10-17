import type { LineStyle } from './setup'

/* 链路类型预设 v2 —— 编码体系：
   色相=速率档+介质（光纤 blue/gray/slate，铜缆 green/lime/pink，40G+ black）
   线宽=速率强化；线型=介质与角色（实线=实物链路，虚线=逻辑管控，双线=级联/堆叠）
   FC 存储网络独立成族（同色不同宽）；SAN 数据面按物理速率选型。
   AP 等末端设备也通过有线介质（网线/光电复合）接入，无「无线」链路类型。
*/

export interface LinkTypePreset {
  color: string // 色板名或 #hex
  style: LineStyle
  width: number
  double?: boolean // 级联/堆叠双线
  oval?: boolean // 双线中点椭圆（堆叠专用）
  desc: string
}

/* FC 族统一色（不在12色板，hex 直供） */
const FC_COLOR = '#0f766e'

/* 12 色板（与画布一致）：名 → hex */
export const COLOR_NAME: Record<string, string> = {
  gray: '#4b5563',
  blue: '#1677ff',
  cyan: '#0891b2',
  green: '#389e0d',
  lime: '#7cb305',
  amber: '#ca8a04',
  orange: '#ea580c',
  red: '#d4380d',
  pink: '#db2777',
  purple: '#722ed1',
  slate: '#64748b',
  black: '#111827'
}

export const HEX_RE = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/

/** 解析色板名或 hex → 失败返回 null */
export function resolveColor(v: string): string | null {
  if (v in COLOR_NAME) return COLOR_NAME[v]
  if (HEX_RE.test(v)) return v
  return null
}

/** 色相 → 浅色 tint（同色系填充/边框，alpha 合成与主题无关） */
export function tint(hexOrName: string, alpha: number): string | null {
  const hex = resolveColor(hexOrName)
  if (!hex) return null
  let h = hex.slice(1)
  if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2]
  const r = parseInt(h.slice(0, 2), 16)
  const g = parseInt(h.slice(2, 4), 16)
  const b = parseInt(h.slice(4, 6), 16)
  return `rgba(${r},${g},${b},${alpha})`
}

export const BUILTIN_LINK_TYPES: Record<string, LinkTypePreset> = {
  /* 数据面：速率定档，光纤/铜缆分色（fiber=blue/gray/slate，copper=green/lime/pink） */
  超高速: { color: 'black', style: 'solid', width: 3, desc: '40GE/100GE' },
  万兆光纤: { color: 'blue', style: 'solid', width: 2.5, desc: '10GE 光纤' },
  万兆网线: { color: 'green', style: 'solid', width: 2.5, desc: '10GE 铜缆' },
  千兆光纤: { color: 'gray', style: 'solid', width: 2, desc: 'GE 光纤' },
  千兆网线: { color: 'lime', style: 'solid', width: 2, desc: 'GE 铜缆' },
  百兆光纤: { color: 'slate', style: 'solid', width: 1.5, desc: '100M 光纤' },
  百兆网线: { color: 'pink', style: 'solid', width: 1.5, desc: '100M 铜缆' },
  /* FC 存储网络：同色不同宽表示 8/16/32/64Gb */
  'FC 8Gb': { color: FC_COLOR, style: 'solid', width: 1.5, desc: 'FC 8Gb' },
  'FC 16Gb': { color: FC_COLOR, style: 'solid', width: 2, desc: 'FC 16Gb' },
  'FC 32Gb': { color: FC_COLOR, style: 'solid', width: 2.5, desc: 'FC 32Gb' },
  'FC 64Gb': { color: FC_COLOR, style: 'solid', width: 3, desc: 'FC 64Gb' },
  /* 叠加与角色 */
  级联: { color: 'gray', style: 'solid', width: 4, double: true, desc: '交换机级联（双线）' },
  堆叠: { color: 'purple', style: 'solid', width: 3, double: true, oval: true, desc: '堆叠' },
  聚合: { color: 'purple', style: 'solid', width: 2, desc: 'LACP 链路聚合' },
  VPN隧道: { color: 'purple', style: 'dashed', width: 2, desc: 'VPN/隧道（逻辑）' },
  /* 管控与冗余 */
  HA: { color: 'red', style: 'dashed', width: 2, desc: 'HA' },
  管理: { color: 'cyan', style: 'dashed', width: 1, desc: 'OOB 管理网' },
  备用: { color: 'amber', style: 'dashed', width: 1.5, desc: '备份/降级链路' },
  /* 广域与外部 */
  互联: { color: 'orange', style: 'solid', width: 2.5, desc: '站点/ISP 互联' },
  专线: { color: 'orange', style: 'solid', width: 2, desc: '租用专线/MPLS（实物）' }
}

export const LINK_TYPE_ALIASES: Record<string, string> = {
  心跳: 'HA',
  OOB: '管理',
  MPLS: '专线',
  lapc: '聚合'
}

export function builtinTypeNames(): string[] {
  return Object.keys(BUILTIN_LINK_TYPES)
}

/* YAML `linkTypes:` 表：仅允许新增自定义类型，内置类型名不可覆盖。
   返回 {merged, errors} — errors 带调用方提供的行号。 */
export function mergeLinkTypes(
  userTable: unknown,
  lineOf: (index: number) => number | null
): { merged: Record<string, LinkTypePreset>; errors: string[] } {
  const merged: Record<string, LinkTypePreset> = { ...BUILTIN_LINK_TYPES }
  const errors: string[] = []
  if (userTable === undefined || userTable === null) return { merged, errors }
  if (typeof userTable !== 'object' || Array.isArray(userTable)) {
    errors.push(`linkTypes 必须是映射对象`)
    return { merged, errors }
  }
  const ALLOWED = new Set(['color', 'style', 'width', 'double', 'oval', 'desc'])
  Object.entries(userTable as Record<string, unknown>).forEach(([name, val], i) => {
    const line = lineOf(i)
    const prefix = line !== null ? `第 ${line} 行：` : ''
    if (name in BUILTIN_LINK_TYPES) {
      errors.push(`${prefix}linkTypes.${name} 是内置类型，不允许覆盖（可改用自定义名称）`)
      return
    }
    if (typeof val !== 'object' || val === null || Array.isArray(val)) {
      errors.push(`${prefix}linkTypes.${name} 必须是对象`)
      return
    }
    const base = { color: 'gray', style: 'solid' as LineStyle, width: 2, desc: `自定义类型 ${name}` }
    const next: LinkTypePreset = { ...base }
    for (const [k, v] of Object.entries(val as Record<string, unknown>)) {
      if (!ALLOWED.has(k)) {
        errors.push(`${prefix}linkTypes.${name}.${k} 不是可识别字段（可写 color/style/width/double/oval/desc）`)
        continue
      }
      if (k === 'color') {
        if (typeof v !== 'string' || !resolveColor(v)) {
          errors.push(`${prefix}linkTypes.${name}.color 非法："${String(v)}"（色板名 ${Object.keys(COLOR_NAME).join('/')} 或 #hex）`)
          continue
        }
        next.color = String(v)
      } else if (k === 'style') {
        if (!['solid', 'dashed', 'dotted'].includes(String(v))) {
          errors.push(`${prefix}linkTypes.${name}.style 非法："${String(v)}"（solid|dashed|dotted）`)
          continue
        }
        next.style = String(v) as LineStyle
      } else if (k === 'width') {
        const n = Number(v)
        if (!isFinite(n) || n <= 0 || n > 8) {
          errors.push(`${prefix}linkTypes.${name}.width 非法（0~8 数值）`)
          continue
        }
        next.width = n
      } else if (k === 'double') {
        next.double = Boolean(v)
      } else if (k === 'oval') {
        next.oval = Boolean(v)
      } else {
        next.desc = String(v)
      }
    }
    merged[name] = next
  })
  return { merged, errors }
}

/* 别名解析 + 合并表查询 */
export function lookupLinkType(
  name: string,
  merged: Record<string, LinkTypePreset>
): { preset: LinkTypePreset | null; canonical: string } {
  const alias = LINK_TYPE_ALIASES[name]
  const key = alias ?? name
  const preset = merged[key] ?? null
  return { preset, canonical: preset ? key : name }
}

/* 进行中的合并表（YAML 解析时刷新；Inspector 链路类型下拉读取） */
let activeMerged: Record<string, LinkTypePreset> = { ...BUILTIN_LINK_TYPES }

export function setActiveMergedTypes(m: Record<string, LinkTypePreset>) {
  activeMerged = m
}

export function activeTypeNames(): string[] {
  return Object.keys(activeMerged)
}

export function getActivePreset(name: string): LinkTypePreset | null {
  return activeMerged[name] ?? null
}
