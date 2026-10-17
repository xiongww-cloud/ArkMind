import { svgUri } from './iconArtBase'
import { ICONS_A } from './iconArtA'
import { ICONS_B } from './iconArtB'

/* 自绘图标：收紧 viewBox 到内容包围盒 + 按纵横比定节点尺寸，
   使节点盒边框 == 图形可视边缘，连线贴死图标无空白 */

const RAW_ICONS: Record<string, string> = { ...ICONS_A, ...ICONS_B }

const STABLER_IDS = [
  'user', 'users', 'id-badge-2', 'shield-check', 'map-pin', 'star',
  'folder', 'file-text', 'bell', 'clock', 'search', 'settings', 'chart-bar',
  'message-circle', 'link', 'wifi', 'device-desktop'
]

const TARGET = 76
const STROKE_PAD = 1.0

const uriCache = new Map<string, string>()
const sizeCache = new Map<string, { w: number; h: number }>()

function measure(inner: string): { vb: string; w: number; h: number } | null {
  try {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
    svg.setAttribute('viewBox', '0 0 32 32')
    svg.innerHTML = inner
    svg.setAttribute('style', 'position:absolute;left:-9999px;width:32px;height:32px;visibility:hidden')
    document.body.appendChild(svg)
    const bb = svg.getBBox()
    svg.remove()
    if (!bb || !isFinite(bb.width) || !isFinite(bb.height) || bb.width <= 0 || bb.height <= 0) return null
    const x = bb.x - STROKE_PAD
    const y = bb.y - STROKE_PAD
    const w = bb.width + STROKE_PAD * 2
    const h = bb.height + STROKE_PAD * 2
    const long = Math.max(w, h)
    const scale = TARGET / long
    return {
      vb: `${x.toFixed(2)} ${y.toFixed(2)} ${w.toFixed(2)} ${h.toFixed(2)}`,
      w: Math.round(w * scale),
      h: Math.round(h * scale)
    }
  } catch {
    return null
  }
}

function register(id: string, inner: string) {
  const m = measure(inner)
  if (m) {
    uriCache.set(id, svgUri(inner, m.vb))
    sizeCache.set(id, { w: m.w, h: m.h })
  } else {
    uriCache.set(id, svgUri(inner))
    sizeCache.set(id, { w: TARGET, h: TARGET })
  }
}

// 自绘图标同步注册（模块执行时 DOM 已就绪：module script defer 语义）
Object.entries(RAW_ICONS).forEach(([id, inner]) => register(id, inner))

// Tabler 图标：fetch 源码后同样收紧（异步，initIcons 内完成）
async function initTabler() {
  await Promise.all(
    STABLER_IDS.map(async (name) => {
      try {
        const res = await fetch(`/icons/tabler/${name}.svg`)
        if (!res.ok) return
        let text = await res.text()
        // 归一：currentColor -> 描边色
        text = text.replace(/currentColor/g, '#3F4C63')
        // 剥掉根 <svg>（tabler 带 width="1em" 嵌套渲染会被裁切），只保留内容 + viewBox
        const vbMatch = text.match(/viewBox="([^"]+)"/)
        const innerMatch = text.match(/<svg[^>]*>([\s\S]*)<\/svg>/)
        if (innerMatch) {
          text = innerMatch[1]
          if (vbMatch) {
            // 内容坐标系仍是原 viewBox（多为 0 0 24 24），测量/渲染由 register/svgUri 接管
            text = `<g>${text}</g>`
          }
        }
        register(name, text)
      } catch {
        /* ignore */
      }
    })
  )
}

/* ---------- 经典（写实）资产：PNG alpha 裁剪清单 + SVG viewBox 收紧 ---------- */

interface ClassicAsset {
  src: string
  w: number
  h: number
}

const classicRegistry = new Map<string, ClassicAsset>()

const CLASSIC_SVGS = [
  'ad/cluster_server.svg',
  'ad/data_jack.svg',
  'ad/database_server.svg',
  'ad/firewall.svg',
  'ad/generic_node.svg',
  'ad/generic_server.svg',
  'ad/home.svg',
  'ad/internet_cloud.svg',
  'ad/internet_globe.svg',
  'ad/key.svg',
  'ad/mainframe_host.svg',
  'ad/phone.svg',
  'ad/printer.svg',
  'ad/router.svg',
  'ad/security.svg',
  'ad/server_farm.svg',
  'ad/sql_server.svg',
  'ad/switch.svg',
  'ad/tunnel.svg',
  'ad/user.svg',
  'ad/users.svg',
  'ad/web_server.svg',
  'allied/Access_Point_Indoor.svg',
  'allied/Access_Point_Outdoor.svg',
  'allied/Datacenter_Server_Rack.svg',
  'allied/Datacenter_Server_Rack_ToR.svg',
  'allied/Datacenter_Server_Storage_Unit_Large.svg',
  'allied/Industrial_Ethernet_IE300.svg',
  'allied/Large_Building.svg',
  'allied/Media_Converter_Standalone.svg',
  'allied/Medium_Business_Building.svg',
  'allied/Modular_Switch_SBx8106.svg',
  'allied/Personal_Computer.svg',
  'allied/Router_UTM.svg',
  'allied/Router_VPN.svg',
  'allied/School_Building.svg',
  'allied/Secure_Building.svg',
  'allied/Server_Desktop.svg',
  'allied/Smartphone.svg',
  'allied/Switch_24_port_L3.svg',
  'allied/Switch_48_port_L3.svg',
  'allied/Tablet.svg'
]

const SVG_NS = 'http://www.w3.org/2000/svg'

function tightenSvg(text: string): ClassicAsset | null {
  try {
    const doc = new DOMParser().parseFromString(text, 'image/svg+xml')
    const root = doc.documentElement
    if (!root || root.nodeName.toLowerCase() !== 'svg') return null
    const vb = root.getAttribute('viewBox') || '0 0 24 24'
    const inner = Array.from(root.childNodes)
      .map((n) => new XMLSerializer().serializeToString(n))
      .join('')
    const meas = document.createElementNS(SVG_NS, 'svg')
    meas.setAttribute('viewBox', vb)
    meas.innerHTML = inner
    meas.setAttribute('style', 'position:absolute;left:-9999px;width:32px;height:32px;visibility:hidden')
    document.body.appendChild(meas)
    const bb = meas.getBBox()
    meas.remove()
    if (!bb || !isFinite(bb.width) || !isFinite(bb.height) || bb.width <= 0 || bb.height <= 0) return null
    const pad = 0.6
    const tightVb = `${bb.x - pad} ${bb.y - pad} ${bb.width + pad * 2} ${bb.height + pad * 2}`
    const tight = `<svg xmlns="${SVG_NS}" viewBox="${tightVb}">${inner}</svg>`
    return {
      src: 'data:image/svg+xml,' + encodeURIComponent(tight),
      w: bb.width,
      h: bb.height
    }
  } catch {
    return null
  }
}

async function initClassic(): Promise<void> {
  /* PNG：alpha 裁剪后的尺寸清单（scripts 预处理产出） */
  try {
    const res = await fetch('/icons/classic-manifest.json')
    if (res.ok) {
      const manifest = (await res.json()) as Record<string, { w: number; h: number }>
      Object.entries(manifest).forEach(([path, dim]) => {
        if (!path.endsWith('.png')) return
        classicRegistry.set(path, { src: `/icons/${path}`, w: dim.w, h: dim.h })
      })
    }
  } catch {
    /* ignore */
  }
  /* SVG：运行时收紧 viewBox → data URI */
  await Promise.all(
    CLASSIC_SVGS.map(async (path) => {
      try {
        const res = await fetch(`/icons/${path}`)
        if (!res.ok) return
        const text = await res.text()
        const asset = tightenSvg(text)
        if (asset) classicRegistry.set(path, asset)
      } catch {
        /* ignore */
      }
    })
  )
}

let inited: Promise<void> | null = null

export function initIcons(): Promise<void> {
  if (!inited) inited = initTabler().then(() => initClassic())
  return inited
}

/** 经典资产（写实）：收紧后的 src + 自然尺寸（纵横比适配用） */
export function classicAsset(path: string): ClassicAsset | null {
  return classicRegistry.get(path) ?? null
}

export function iconUri(id: string): string {
  return uriCache.get(id) || ''
}

export function iconSize(id: string): { w: number; h: number } {
  return sizeCache.get(id) || { w: TARGET, h: TARGET }
}

export function hasIcon(id: string): boolean {
  return uriCache.has(id)
}

/** 全部已注册图标 id（自绘 + tabler；供 YAML 校验的纠错建议） */
export function allIconIds(): string[] {
  return [...uriCache.keys()]
}
