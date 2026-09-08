import { emailInlineAsset } from '@/api/email-client'
import { imageCacheGet, imageCacheSet } from '@/utils/emailImageCache'

const CONCURRENCY = 3
const VISIBLE_MARGIN = 300

/**
 * 为邮件正文容器里的懒加载占位图（img[data-emimg]）启用分批/视口懒加载，并带上加载动画：
 * - 每张占位图被包进 <span class="emimg-box">，内含居中圆环 spinner；
 * - 占位阶段：宽高比 + 容器宽自适应，从一开始就铺满宽度不跳动；
 * - 图片进入可视区（含 300px 预加载边距）才拉取；加载完成 box 加 emimg-loaded（淡入、隐 spinner）；
 * - 小图加载后还原原始小尺寸；加载失败 box 加 emimg-error（保留灰底）。
 * 返回的 cancel 可在卸载/切换邮件时终止后续加载。
 */
export function lazyLoadEmailImages(
  container: HTMLElement | null | undefined,
  emailId: string
): () => void {
  const el = container
  if (!el) return () => {}
  const imgs = Array.from(el.querySelectorAll<HTMLImageElement>('img[data-emimg]'))
  if (imgs.length === 0) return () => {}
  console.info(`[emailLazy] start lazy load, ${imgs.length} images`)

  let cancelled = false
  const done = new WeakSet<HTMLImageElement>()
  const scheduled = new WeakSet<HTMLImageElement>()
  const queue: HTMLImageElement[] = []
  let draining = false

  function boxOf(img: HTMLImageElement): HTMLElement | null {
    const p = img.parentElement
    return p && p.classList.contains('emimg-box') ? p : null
  }

  // 把每个占位图包进 .emimg-box（内插 spinner），占位宽高比迁移到 box
  const wrapped = imgs.filter((img) => {
    const w = parseFloat(img.getAttribute('width') || '')
    const h = parseFloat(img.getAttribute('height') || '')
    if (boxOf(img)) {
      img.style.width = '100%'
      img.style.height = 'auto'
      return true
    }
    const box = document.createElement('span')
    box.className = 'emimg-box'
    if (Number.isFinite(w) && Number.isFinite(h) && w > 0 && h > 0) {
      box.style.aspectRatio = `${w} / ${h}`
    } else {
      box.style.minHeight = '120px'
    }
    const spinner = document.createElement('span')
    spinner.className = 'emimg-spinner'
    box.appendChild(spinner)
    img.parentNode?.insertBefore(box, img)
    box.appendChild(img)
    img.style.width = '100%'
    img.style.height = 'auto'
    return true
  })

  function markLoaded(img: HTMLImageElement) {
    const box = boxOf(img)
    if (box && !box.classList.contains('emimg-loaded')) {
      box.classList.add('emimg-loaded')
    }
    if (!done.has(img)) {
      done.add(img)
      img.removeAttribute('data-emimg')
    }
  }

  function markError(img: HTMLImageElement) {
    const box = boxOf(img)
    if (box) box.classList.add('emimg-error')
    if (!done.has(img)) {
      done.add(img)
      img.removeAttribute('data-emimg')
    }
  }

  // 解码后：大图保持铺满容器，小图还原原始小尺寸（box 跟随收缩）
  function finalizeSize(img: HTMLImageElement) {
    if (cancelled) return
    try {
      const containerW = el.clientWidth
      const natW = img.naturalWidth
      const box = boxOf(img)
      if (natW > 0 && natW <= containerW) {
        img.style.width = ''
        img.style.height = ''
        if (box) {
          box.style.aspectRatio = ''
          box.style.minHeight = ''
          // 下一帧按图片实际宽度收缩 box
          requestAnimationFrame(() => {
            if (img.offsetWidth > 0) box.style.width = `${img.offsetWidth}px`
          })
        }
      }
    } catch {
      /* ignore */
    }
  }

  // 应用 dataURL：命中缓存与刚拉取共用同一套 onload/淡入逻辑
  function applyDataUrl(img: HTMLImageElement, dataUrl: string) {
    if (cancelled) return
    img.onload = () => {
      finalizeSize(img)
      markLoaded(img)
    }
    img.onerror = () => markError(img)
    img.src = dataUrl
    img.removeAttribute('srcset')
    if (img.complete && img.naturalWidth > 0) {
      finalizeSize(img)
      markLoaded(img)
    }
  }

  function fetchOne(img: HTMLImageElement) {
    const token = img.getAttribute('data-emimg')
    if (!token) {
      markLoaded(img)
      return Promise.resolve()
    }
    // 命中会话缓存：直接出图，不再走后端
    const cached = imageCacheGet(emailId, token)
    if (cached) {
      applyDataUrl(img, cached)
      return Promise.resolve()
    }
    return emailInlineAsset(emailId, token)
      .then((r) => {
        if (cancelled) return
        if (r?.success && r.dataUrl) {
          imageCacheSet(emailId, token, r.dataUrl)
          applyDataUrl(img, r.dataUrl)
        } else {
          console.warn('[emailLazy] 解析失败, 保持占位:', token, r)
          markError(img)
        }
      })
      .catch((e) => {
        console.error('[emailLazy] 加载图片失败:', token, e)
        markError(img)
      })
  }

  function schedule(img: HTMLImageElement) {
    if (done.has(img) || scheduled.has(img)) return
    scheduled.add(img)
    queue.push(img)
    if (!draining) {
      draining = true
      void drain()
    }
  }

  async function drain() {
    while (queue.length > 0 && !cancelled) {
      const batch = queue.splice(0, CONCURRENCY)
      await Promise.all(batch.map(fetchOne))
    }
    draining = false
    if (queue.length > 0 && !cancelled) {
      draining = true
      void drain()
    }
  }

  function isVisible(img: HTMLImageElement): boolean {
    const r = img.getBoundingClientRect()
    const vw = window.innerWidth || document.documentElement.clientWidth
    const vh = window.innerHeight || document.documentElement.clientHeight
    if (r.width === 0 && r.height === 0) return true
    return (
      r.bottom >= -VISIBLE_MARGIN &&
      r.top <= vh + VISIBLE_MARGIN &&
      r.right >= -VISIBLE_MARGIN &&
      r.left <= vw + VISIBLE_MARGIN
    )
  }

  function scan() {
    if (cancelled) return
    for (const img of wrapped) {
      if (!done.has(img) && !scheduled.has(img) && isVisible(img)) schedule(img)
    }
  }

  let io: IntersectionObserver | null = null
  if (typeof IntersectionObserver === 'function') {
    io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            const img = e.target as HTMLImageElement
            io?.unobserve(img)
            if (!done.has(img) && !scheduled.has(img)) schedule(img)
          }
        }
      },
      { root: null, rootMargin: `${VISIBLE_MARGIN}px 0px`, threshold: 0 }
    )
    wrapped.forEach((img) => io?.observe(img))
  }

  const scrollers: Array<Element | Window> = [window]
  let node: HTMLElement | null = el
  while (node) {
    const st = getComputedStyle(node)
    if (node.scrollHeight > node.clientHeight && /(auto|scroll|overlay)/i.test(st.overflowY || '')) {
      scrollers.push(node)
      break
    }
    node = node.parentElement
  }
  let raf = 0
  const onScroll = () => {
    if (raf) cancelAnimationFrame(raf)
    raf = requestAnimationFrame(() => {
      raf = 0
      scan()
    })
  }
  scrollers.forEach((s) => s.addEventListener('scroll', onScroll, { passive: true }))

  scan()
  const safetyTimer = window.setTimeout(() => scan(), 800)

  return () => {
    cancelled = true
    io?.disconnect()
    scrollers.forEach((s) => s.removeEventListener('scroll', onScroll))
    window.clearTimeout(safetyTimer)
  }
}
