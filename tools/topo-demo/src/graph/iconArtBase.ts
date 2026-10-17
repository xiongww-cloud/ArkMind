/* 图标设计系统令牌与几何助手 —— 详见 public/icons/SYSTEM.md */

export const S = '#3F4C63'
export const F = '#DCE7F5'
export const F2 = '#F1F5FB'
export const A = '#3B82F6'
export const AD = '#2563EB'
export const W = '#FFFFFF'

/** 机身/面板 */
export const box = (
  x: number,
  y: number,
  w: number,
  h: number,
  rx = 3,
  fill = F,
  stroke = S,
  sw = 2
) =>
  `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${rx}" fill="${fill}" stroke="${stroke}" stroke-width="${sw}"/>`

/** 线段（round cap） */
export const line = (d: string, stroke = S, w = 2) =>
  `<path d="${d}" fill="none" stroke="${stroke}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/>`

/** 实心三角箭头（沿 +x 方向，配合 line 终点） */
export const triR = (x: number, y: number, s = 3.2, fill = A) =>
  `<path d="M${x} ${y} L${x - s} ${y - s * 0.72} L${x - s} ${y + s * 0.72} Z" fill="${fill}"/>`

/** 实心三角箭头（沿 -x 方向） */
export const triL = (x: number, y: number, s = 3.2, fill = A) =>
  `<path d="M${x} ${y} L${x + s} ${y - s * 0.72} L${x + s} ${y + s * 0.72} Z" fill="${fill}"/>`

/** 实心三角箭头（沿 -y 方向，朝上） */
export const triU = (x: number, y: number, s = 3.2, fill = A) =>
  `<path d="M${x} ${y} L${x - s * 0.72} ${y + s} L${x + s * 0.72} ${y + s} Z" fill="${fill}"/>`

/** 实心三角箭头（沿 +y 方向，朝下） */
export const triD = (x: number, y: number, s = 3.2, fill = A) =>
  `<path d="M${x} ${y} L${x - s * 0.72} ${y - s} L${x + s * 0.72} ${y - s} Z" fill="${fill}"/>`

/** 水平双向箭头（转发母题） */
export const hArrow = (x1: number, x2: number, y: number, stroke = A) =>
  `${line(`M${x1 + 3} ${y} H${x2 - 3}`, stroke)}${triL(x1, y, 3.2, stroke)}${triR(x2, y, 3.2, stroke)}`

/** 垂直双向箭头 */
export const vArrow = (x: number, y1: number, y2: number, stroke = A) =>
  `${line(`M${x} ${y1 + 3} V${y2 - 3}`, stroke)}${triD(x, y1, 3.2, stroke)}${triU(x, y2, 3.2, stroke)}`

/** 端口排：底部一排小方块 */
export const portRow = (
  y: number,
  n: number,
  x0: number,
  step = 4.5,
  w = 3,
  h = 3,
  fill = A
) => {
  let out = ''
  for (let i = 0; i < n; i++) {
    out += `<rect x="${x0 + i * step}" y="${y}" width="${w}" height="${h}" rx="0.8" fill="${fill}"/>`
  }
  return out
}

/** 指示灯 */
export const led = (x: number, y: number, c = A, r = 1.6) => `<circle cx="${x}" cy="${y}" r="${r}" fill="${c}"/>`

/** 无线弧组（双弧 + 端点） */
export const wifi = (cx: number, cy: number, color = A) =>
  line(`M${cx - 5.5} ${cy - 2} A7.8 7.8 0 0 1 ${cx + 5.5} ${cy - 2}`, color, 2) +
  line(`M${cx - 9.5} ${cy - 6.4} A13.5 13.5 0 0 1 ${cx + 9.5} ${cy - 6.4}`, S, 2) +
  led(cx, cy, AD, 1.8)

/** 机箱 + 端口排组合（交换类母题） */
export const switchChassis = (opts?: { x?: number; y?: number; w?: number; h?: number }) => {
  const x = opts?.x ?? 3
  const y = opts?.y ?? 9
  const w = opts?.w ?? 26
  const h = opts?.h ?? 14
  return `${box(x, y, w, h)}${hArrow(x + 5, x + w - 5, y + 4.5)}${portRow(y + h - 6, Math.floor((w - 8) / 4.5), x + 4)}`
}

/** SVG 包装为 data URI（供 <image href> / <img src> 使用）；viewBox 可收紧到内容包围盒 */
export function svgUri(inner: string, viewBox = '0 0 32 32'): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" width="32" height="32">${inner}</svg>`
  return 'data:image/svg+xml,' + encodeURIComponent(svg)
}
