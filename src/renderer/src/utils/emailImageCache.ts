/**
 * 邮件渲染缓存（会话内存级）：
 * 1) 懒加载图片 dataURL —— LRU，总上限 60MB，避免重复进入同一邮件时反复后端读取 + base64；
 * 2) 已渲染 HTML（eager 小邮件一次性内联结果）—— 按 emailId，TTL 30 分钟。
 */

const MAX_IMAGE_BYTES = 60 * 1024 * 1024 // 60MB
const RENDER_TTL_MS = 30 * 60 * 1000 // 30 分钟
const RENDER_MAX_ENTRIES = 50

// key: `${emailId}:${token}`
const imageMap = new Map<string, string>()
let imageBytes = 0

function keyOf(emailId: string, token: string): string {
  return `${emailId}:${token}`
}

export function imageCacheGet(emailId: string, token: string): string | undefined {
  const k = keyOf(emailId, token)
  const v = imageMap.get(k)
  if (v !== undefined) {
    // LRU：移到末尾
    imageMap.delete(k)
    imageMap.set(k, v)
  }
  return v
}

export function imageCacheSet(emailId: string, token: string, dataUrl: string): void {
  const k = keyOf(emailId, token)
  const cost = Math.ceil(dataUrl.length * 0.75) // base64 → 字节估算
  if (imageMap.has(k)) {
    imageBytes -= Math.ceil((imageMap.get(k) as string).length * 0.75)
  }
  imageMap.delete(k)
  imageMap.set(k, dataUrl)
  imageBytes += cost

  // LRU 淘汰最旧，直到低于上限
  while (imageBytes > MAX_IMAGE_BYTES && imageMap.size > 1) {
    const oldest = imageMap.keys().next()
    if (oldest.done) break
    const oldKey = oldest.value
    const oldVal = imageMap.get(oldKey) as string
    imageBytes -= Math.ceil(oldVal.length * 0.75)
    imageMap.delete(oldKey)
  }
}

export function imageCacheClearForEmail(emailId: string): void {
  for (const k of Array.from(imageMap.keys())) {
    if (k.startsWith(`${emailId}:`)) {
      const v = imageMap.get(k) as string
      imageBytes -= Math.ceil(v.length * 0.75)
      imageMap.delete(k)
    }
  }
}

interface RenderedCacheEntry {
  html: string
  lazy?: boolean
  ts: number
}

const renderedMap = new Map<string, RenderedCacheEntry>()

export function renderedCacheGet(emailId: string): { html: string; lazy?: boolean } | null {
  const e = renderedMap.get(emailId)
  if (!e) return null
  if (Date.now() - e.ts > RENDER_TTL_MS) {
    renderedMap.delete(emailId)
    return null
  }
  return { html: e.html, lazy: e.lazy }
}

export function renderedCacheSet(emailId: string, html: string, lazy?: boolean): void {
  // 控制条数
  while (renderedMap.size >= RENDER_MAX_ENTRIES) {
    const oldest = renderedMap.keys().next()
    if (oldest.done) break
    renderedMap.delete(oldest.value)
  }
  renderedMap.set(emailId, { html, lazy, ts: Date.now() })
}
