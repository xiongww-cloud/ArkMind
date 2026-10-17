import { hasIcon, iconSize, iconUri, classicAsset } from './iconArt'

/* 图标风格配对目录：设备类严格 1:1（简约↔写实零回退），通用符号风格中立。
   scale = 默认尺寸倍率（基准 76px），两侧同倍率保证风格切换视觉大小一致。
   nas/proxy/ids/wireless_ap 无经典对版 → 不在目录（面板仍可手动拖放）。 */

export interface PairDef {
  id: string
  modern: string // 简约侧 iconArt id
  classic: string // 写实侧 /icons/ 相对路径
  scale: number
}

/* 尺寸档：1.15 大 | 1.0 默认 | 0.85 中小 | 0.8 服务器/存储 | 0.6 末端（AP 等明显缩小） */
const BIG = 1.15
const MID = 0.85
const SERVER = 0.8
const EDGE = 0.6

/* 末端设备：统一内切于 END_W×END_W（长边=END_W，宽度基准的推广——
   横长图标宽56、竖长图标高56，保证同档视觉一致且不超网络设备高度）。 */
export const END_W = 56
const END_DEVICE_IDS = new Set([
  'ap', 'ap_outdoor', 'pc', 'laptop', 'tablet', 'phone',
  'printer', 'ip_camera', 'monitor', 'usb_hub', 'modem', 'data_jack'
])

/* 服务器/存储：统一 SERVER 档（长边基准），比网络设备小、类目内视觉一致 */
const SERVER_IDS = new Set([
  'server', 'web_server', 'database', 'sql', 'mail', 'mainframe',
  'cluster', 'farm', 'storage_array', 'rack', 'rack_empty', 'ups',
  'pdu', 'vm', 'harddrive', 'nas'
])

export function isEndDevice(id: string): boolean {
  return END_DEVICE_IDS.has(id)
}

export function isServerDevice(id: string): boolean {
  return SERVER_IDS.has(id)
}

export function isEndDeviceClassicPath(path: string): boolean {
  const pair = PAIR_BY_CLASSIC.get(path)
  return !!pair && END_DEVICE_IDS.has(pair.id)
}

/** 末端设备尺寸：内切 END_W×END_W（长边=END_W），高等比 */
export function endFit(natW: number, natH: number): { w: number; h: number } {
  const s = END_W / Math.max(natW, natH)
  return { w: Math.max(24, Math.round(natW * s)), h: Math.max(24, Math.round(natH * s)) }
}

export const DEVICE_PAIRS: PairDef[] = [
  /* 路由与交换 */
  { id: 'router', modern: 'router', classic: 'net/Router.png', scale: 1 },
  { id: 'switch', modern: 'switch', classic: 'net/Switch.png', scale: 1 },
  { id: 'switch_l3', modern: 'switch_l3', classic: 'allied/Switch_24_port_L3.svg', scale: 1 },
  { id: 'modular_switch', modern: 'modular_switch', classic: 'allied/Modular_Switch_SBx8106.svg', scale: 1 },
  { id: 'industrial_switch', modern: 'industrial_switch', classic: 'allied/Industrial_Ethernet_IE300.svg', scale: 1 },
  { id: 'fc_switch', modern: 'fc_switch', classic: 'allied/Switch_48_port_L3.svg', scale: 1 },
  { id: 'wireless_router', modern: 'wireless_router', classic: 'net/Wireless_Router.png', scale: 1 },
  { id: 'ap', modern: 'ap', classic: 'allied/Access_Point_Indoor.svg', scale: EDGE },
  { id: 'ap_outdoor', modern: 'ap_outdoor', classic: 'allied/Access_Point_Outdoor.svg', scale: EDGE },
  { id: 'modem', modern: 'modem', classic: 'net/Modem.png', scale: EDGE },
  { id: 'bridge', modern: 'bridge', classic: 'net/Bridge.png', scale: 1 },
  { id: 'repeater', modern: 'repeater', classic: 'net/Repeater.png', scale: 1 },
  { id: 'concentrator', modern: 'concentrator', classic: 'net/Concentrator.png', scale: 1 },
  { id: 'media_converter', modern: 'media_converter', classic: 'allied/Media_Converter_Standalone.svg', scale: MID },
  /* 安全 */
  { id: 'firewall', modern: 'firewall', classic: 'net/Firewall.png', scale: 1 },
  { id: 'firewall2', modern: 'firewall2', classic: 'net/Firewall_02.png', scale: 1 },
  { id: 'vpn_router', modern: 'vpn_router', classic: 'allied/Router_VPN.svg', scale: 1 },
  { id: 'utm', modern: 'utm', classic: 'allied/Router_UTM.svg', scale: 1 },
  { id: 'cert', modern: 'cert', classic: 'net/Certificate.png', scale: MID },
  { id: 'key', modern: 'key', classic: 'ad/key.svg', scale: EDGE },
  { id: 'tunnel', modern: 'tunnel', classic: 'ad/tunnel.svg', scale: MID },
  /* 云与互联网 */
  { id: 'cloud', modern: 'cloud', classic: 'net/Cloud.png', scale: 1 },
  { id: 'cloud_server', modern: 'cloud_server', classic: 'net/Cloud_Server.png', scale: 1 },
  { id: 'cloud_storage', modern: 'cloud_storage', classic: 'net/Cloud_Storage.png', scale: 1 },
  { id: 'internet', modern: 'internet', classic: 'ad/internet_cloud.svg', scale: 1 },
  { id: 'internet_globe', modern: 'internet_globe', classic: 'ad/internet_globe.svg', scale: 1 },
  /* 服务器与存储（统一 SERVER 档） */
  { id: 'server', modern: 'server', classic: 'net/Server.png', scale: SERVER },
  { id: 'web_server', modern: 'web_server', classic: 'ad/web_server.svg', scale: SERVER },
  { id: 'database', modern: 'database', classic: 'net/Database.png', scale: SERVER },
  { id: 'sql', modern: 'sql', classic: 'ad/sql_server.svg', scale: SERVER },
  { id: 'mail', modern: 'mail', classic: 'net/Email.png', scale: SERVER },
  { id: 'mainframe', modern: 'mainframe', classic: 'net/Mainframe.png', scale: SERVER },
  { id: 'cluster', modern: 'cluster', classic: 'ad/cluster_server.svg', scale: SERVER },
  { id: 'farm', modern: 'farm', classic: 'ad/server_farm.svg', scale: SERVER },
  { id: 'storage_array', modern: 'storage_array', classic: 'allied/Datacenter_Server_Storage_Unit_Large.svg', scale: SERVER },
  { id: 'rack', modern: 'rack', classic: 'net/Server_Rack.png', scale: SERVER },
  { id: 'rack_empty', modern: 'rack_empty', classic: 'net/Server_Rack_Empty.png', scale: SERVER },
  { id: 'ups', modern: 'ups', classic: 'net/UPS.png', scale: SERVER },
  { id: 'pdu', modern: 'pdu', classic: 'net/power_distribution_unit.png', scale: SERVER },
  { id: 'vm', modern: 'vm', classic: 'pc/Virtual_Machine.png', scale: SERVER },
  { id: 'harddrive', modern: 'harddrive', classic: 'pc/Harddrive.png', scale: SERVER },
  /* 终端与办公 */
  { id: 'pc', modern: 'pc', classic: 'allied/Personal_Computer.svg', scale: 1 },
  { id: 'workstation', modern: 'workstation', classic: 'pc/Workstation.png', scale: MID },
  { id: 'laptop', modern: 'laptop', classic: 'pc/Laptop.png', scale: MID },
  { id: 'tablet', modern: 'tablet', classic: 'allied/Tablet.svg', scale: MID },
  { id: 'phone', modern: 'phone', classic: 'allied/Smartphone.svg', scale: EDGE },
  { id: 'printer', modern: 'printer', classic: 'pc/Printer.png', scale: MID },
  { id: 'ip_camera', modern: 'ip_camera', classic: 'net/Ip_Camera.png', scale: EDGE },
  { id: 'monitor', modern: 'monitor', classic: 'pc/Monitor_Tower.png', scale: MID },
  { id: 'usb_hub', modern: 'usb_hub', classic: 'pc/USB_Hub.png', scale: EDGE },
  /* 建筑与场地 */
  { id: 'building_lg', modern: 'building_lg', classic: 'allied/Large_Building.svg', scale: BIG },
  { id: 'building_biz', modern: 'building_biz', classic: 'allied/Medium_Business_Building.svg', scale: BIG },
  { id: 'school', modern: 'school', classic: 'allied/School_Building.svg', scale: BIG },
  { id: 'secure_building', modern: 'secure_building', classic: 'allied/Secure_Building.svg', scale: BIG },
  { id: 'home', modern: 'home', classic: 'ad/home.svg', scale: MID },
  { id: 'data_jack', modern: 'data_jack', classic: 'ad/data_jack.svg', scale: EDGE }
]

/* 通用符号：风格中立（两种风格共用简约侧资产） */
export const GENERAL_IDS = [
  'user',
  'users',
  'id-badge-2',
  'star',
  'folder',
  'file-text',
  'bell',
  'clock',
  'search',
  'settings',
  'chart-bar',
  'message-circle',
  'link',
  'shield-check',
  'wifi',
  'device-desktop',
  'map-pin'
]

export type IconStyle = '简约' | '写实'

const PAIR_BY_ID = new Map(DEVICE_PAIRS.map((p) => [p.id, p]))
const PAIR_BY_CLASSIC = new Map(DEVICE_PAIRS.map((p) => [p.classic, p]))
const GENERAL_SET = new Set(GENERAL_IDS)

export function normalizeIconStyle(v: unknown): IconStyle | null {
  if (typeof v !== 'string') return null
  const t = v.trim()
  if (t === '简约' || t === 'simple' || t === 'minimal') return '简约'
  if (t === '写实' || t === 'realistic') return '写实'
  return null
}

/** 目录内可被 YAML 引用的语义 id（两风格一致） */
export function catalogIds(): string[] {
  return [...DEVICE_PAIRS.map((p) => p.id), ...GENERAL_IDS]
}

export function inCatalog(id: string): boolean {
  return PAIR_BY_ID.has(id) || GENERAL_SET.has(id)
}

export interface ResolvedIcon {
  src: string
  w: number
  h: number
}

function fit(size: { w: number; h: number }, scale: number): { w: number; h: number } {
  return { w: Math.round(size.w * scale), h: Math.round(size.h * scale) }
}

/** 按风格解析图标资产；不在目录返回 null */
export function resolveIconForStyle(id: string, style: IconStyle): ResolvedIcon | null {
  if (GENERAL_SET.has(id)) {
    if (!hasIcon(id)) return null
    const s = iconSize(id)
    return { src: iconUri(id), ...fit(s, 1) }
  }
  const pair = PAIR_BY_ID.get(id)
  if (!pair) return null
  const end = END_DEVICE_IDS.has(id)
  if (style === '写实') {
    const asset = classicAsset(pair.classic)
    if (!asset) return null
    if (end) return { src: asset.src, ...endFit(asset.w, asset.h) }
    const long = Math.max(asset.w, asset.h)
    const s = (76 * pair.scale) / long
    return { src: asset.src, w: Math.max(24, Math.round(asset.w * s)), h: Math.max(24, Math.round(asset.h * s)) }
  }
  if (!hasIcon(pair.modern)) return null
  const s = iconSize(pair.modern)
  if (end) return { src: iconUri(pair.modern), ...endFit(s.w, s.h) }
  return { src: iconUri(pair.modern), ...fit(s, pair.scale) }
}

/** 面板拖放用：现代 id 尺寸档（目录外为 1；服务器/存储统一 SERVER 档） */
export function scaleForModern(id: string): number {
  if (SERVER_IDS.has(id)) return SERVER
  return PAIR_BY_ID.get(id)?.scale ?? 1
}

/** 面板拖放用：经典路径尺寸档（未配对为 1；服务器/存储统一 SERVER 档） */
export function scaleForClassicPath(path: string): number {
  const pair = PAIR_BY_CLASSIC.get(path)
  if (pair && SERVER_IDS.has(pair.id)) return SERVER
  return pair?.scale ?? 1
}

/** 纠错建议候选（与风格无关，语义 id 一致） */
export function suggestCatalogId(bad: string): string | null {
  const lower = bad.toLowerCase()
  const ids = catalogIds()
  const exact = ids.find((id) => id.toLowerCase() === lower)
  if (exact) return exact
  let best: string | null = null
  let bestD = 3
  for (const id of ids) {
    const d = levenshtein(lower, id.toLowerCase())
    if (d < bestD) {
      bestD = d
      best = id
    }
  }
  return best
}

function levenshtein(a: string, b: string): number {
  const m = a.length
  const n = b.length
  if (!m) return n
  if (!n) return m
  let prev = Array.from({ length: n + 1 }, (_, i) => i)
  for (let i = 1; i <= m; i++) {
    const cur = [i]
    for (let j = 1; j <= n; j++) {
      cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1))
    }
    prev = cur
  }
  return prev[n]
}

/* src→语义 id 反查（YAML 导出用）：两风格 src 都收进同一张表，惰性构建避免加载期副作用 */
let _iconIdBySrc: Map<string, string> | null = null
function ensureSrcIndex(): Map<string, string> {
  if (_iconIdBySrc) return _iconIdBySrc
  const m = new Map<string, string>()
  for (const id of catalogIds()) {
    for (const style of ['简约', '写实'] as const) {
      const r = resolveIconForStyle(id, style)
      if (r && !m.has(r.src)) m.set(r.src, id)
    }
  }
  _iconIdBySrc = m
  return m
}

/** 资产 src → 语义 id；未知返回 null */
export function iconIdBySrc(src: string): string | null {
  return ensureSrcIndex().get(src) ?? null
}
