/**
 * htmlSlides.ts — HTML 原生演示材料的解析/序列化/注入工具
 *
 * 每个演示材料是一个完整的 HTML 文件，每页用 <section class="slide"> 分隔。
 * 设计尺寸：1920×1080（Full HD），渲染时通过 CSS transform: scale() 自适应缩放。
 */

export const SLIDE_W = 1920
export const SLIDE_H = 1080

export interface SlidePage {
  index: number
  html: string
  title: string
}

/** 全局主题 CSS（注入到 iframe 的 <style> 中） */
export const LIGHT_THEME_CSS = `
:root {
  --accent: #2563eb;
  --accent-light: #3b82f6;
  --bg: #ffffff;
  --bg-alt: #f9fafb;
  --text: #111827;
  --muted: #6b7280;
  --border: #e5e7eb;
  --card: #ffffff;
  --font: 'Inter', 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
  --slide-w: ${SLIDE_W}px;
  --slide-h: ${SLIDE_H}px;
}
`

export const DARK_THEME_CSS = `
:root {
  --accent: #3b82f6;
  --accent-light: #60a5fa;
  --bg: #0f172a;
  --bg-alt: #1e293b;
  --text: #f1f5f9;
  --muted: #94a3b8;
  --border: #334155;
  --card: #1e293b;
  --font: 'Inter', 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
  --slide-w: ${SLIDE_W}px;
  --slide-h: ${SLIDE_H}px;
}
`

const BASE_CSS = `
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body {
  font-family: var(--font); color: var(--text); background: var(--bg);
  width: ${SLIDE_W}px; height: ${SLIDE_H}px; overflow: hidden;
}
.slide {
  width: ${SLIDE_W}px;
  height: ${SLIDE_H}px;
  position: relative;
  overflow: hidden;
  background: var(--bg);
  font-family: var(--font);
}
img { max-width: 100%; height: auto; }
`

/** 默认主题 CSS：当 AI 未提供主题时，确保 CSS 变量有值可解析 */
const DEFAULT_THEME_CSS = `
:root {
  --accent: #2563eb;
  --accent-light: #3b82f6;
  --bg: #ffffff;
  --bg-alt: #f9fafb;
  --text: #111827;
  --muted: #6b7280;
  --border: #e5e7eb;
  --card: #ffffff;
  --font: 'Inter', 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
  --slide-w: ${SLIDE_W}px;
  --slide-h: ${SLIDE_H}px;
}
`

/** 注入到 iframe 的脚本（导航 + 点击事件通信） */
const IFRAME_SCRIPT = `
(function() {
  var slides = document.querySelectorAll('.slide');
  var currentIndex = 0;

  function goTo(index) {
    if (index < 0 || index >= slides.length) return;
    slides.forEach(function(s, i) { s.style.display = i === index ? 'block' : 'none'; });
    currentIndex = index;
    window.parent.postMessage({ type: 'SLIDE_CHANGED', index: index, total: slides.length }, '*');
  }

  // 初始显示第一页
  goTo(0);

  // 监听父窗口指令
  window.addEventListener('message', function(e) {
    var data = e.data;
    if (!data) return;
    switch (data.type) {
      case 'NAVIGATE_TO': goTo(data.index); break;
      case 'GET_SLIDES_INFO':
        window.parent.postMessage({
          type: 'SLIDES_INFO',
          total: slides.length,
          titles: Array.from(slides).map(function(s) {
            var h = s.querySelector('h1, h2, h3');
            return h ? h.textContent.trim() : '';
          })
        }, '*');
        break;
    }
  });

  // 点击元素 → 通知父窗口
  document.addEventListener('click', function(e) {
    e.preventDefault();
    var el = e.target;
    window.parent.postMessage({
      type: 'ELEMENT_CLICKED',
      tagName: el.tagName,
      id: el.id || '',
      className: el.className || '',
      text: (el.textContent || '').slice(0, 200),
      rect: el.getBoundingClientRect(),
      outerHTML: el.outerHTML.slice(0, 500)
    }, '*');
  }, true);
})();
`

/** 从完整 HTML 中解析各页 */
export function parseSlides(html: string): SlidePage[] {
  if (!html || !html.trim()) return []

  const parser = new DOMParser()
  const doc = parser.parseFromString(html, 'text/html')
  const sections = doc.querySelectorAll('.slide')

  if (sections.length === 0) {
    // 兼容：尝试 div.slide
    const divs = doc.querySelectorAll('div.slide')
    if (divs.length > 0) {
      return Array.from(divs).map((el, i) => ({
        index: i,
        html: el.outerHTML,
        title: extractTitle(el)
      }))
    }
  }

  return Array.from(sections).map((el, i) => ({
    index: i,
    html: el.outerHTML,
    title: extractTitle(el)
  }))
}

/** 从元素中提取标题 */
function extractTitle(el: Element): string {
  const heading = el.querySelector('h1, h2, h3')
  if (heading) return heading.textContent?.trim() ?? ''
  // fallback：取前50个字符
  return (el.textContent?.trim().slice(0, 50) ?? '') || '无标题'
}

/** 构建完整的 HTML 文件内容 */
export function buildFullHtml(slidesHtml: string[], themeCss: string, title?: string): string {
  // 始终包含默认主题 CSS（提供 :root 变量），AI 可覆盖但不会缺失
  const mergedCss = `${DEFAULT_THEME_CSS}\n${themeCss}`
  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <title>${title ?? '演示材料'}</title>
  <style>
${mergedCss}
${BASE_CSS}
  </style>
</head>
<body>
${slidesHtml.map((html, i) => `  <!-- 第${i + 1}页 -->
  ${html}`).join('\n\n')}

  <script>
${IFRAME_SCRIPT}
  </script>
</body>
</html>`
}

/** 构建单页的 srcdoc（用于只渲染一页） */
export function buildSinglePageSrcdoc(pageHtml: string, themeCss: string): string {
  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <style>
${themeCss}
${BASE_CSS}
.slide { display: block !important; }
  </style>
</head>
<body>
  ${pageHtml}

  <script>
${IFRAME_SCRIPT}
  </script>
</body>
</html>`
}

/** 替换完整 HTML 中的某一页 */
export function replaceSlide(fullHtml: string, pageIndex: number, newSlideHtml: string): string {
  const slides = parseSlides(fullHtml)
  if (pageIndex < 0 || pageIndex >= slides.length) return fullHtml

  slides[pageIndex].html = newSlideHtml

  // 重建完整 HTML（保留 <head> 中的样式）
  const parser = new DOMParser()
  const doc = parser.parseFromString(fullHtml, 'text/html')
  const sections = doc.querySelectorAll('.slide')

  if (sections[pageIndex]) {
    const wrapper = doc.createElement('div')
    wrapper.innerHTML = newSlideHtml
    const newEl = wrapper.firstElementChild
    if (newEl) {
      sections[pageIndex].replaceWith(newEl)
    }
  }

  return '<!DOCTYPE html>\n' + doc.documentElement.outerHTML
}

/** 在完整 HTML 的指定位置插入一页 */
export function insertSlide(fullHtml: string, afterIndex: number, newSlideHtml: string): string {
  const parser = new DOMParser()
  const doc = parser.parseFromString(fullHtml, 'text/html')
  const sections = doc.querySelectorAll('.slide')

  if (afterIndex >= 0 && afterIndex < sections.length) {
    const wrapper = doc.createElement('div')
    wrapper.innerHTML = newSlideHtml
    const newEl = wrapper.firstElementChild
    if (newEl) {
      sections[afterIndex].after(newEl)
    }
  } else {
    // 追加到 body 末尾
    const wrapper = doc.createElement('div')
    wrapper.innerHTML = newSlideHtml
    const newEl = wrapper.firstElementChild
    if (newEl) {
      doc.body.appendChild(newEl)
    }
  }

  return '<!DOCTYPE html>\n' + doc.documentElement.outerHTML
}

/** 删除完整 HTML 中的某一页 */
export function deleteSlide(fullHtml: string, pageIndex: number): string {
  const parser = new DOMParser()
  const doc = parser.parseFromString(fullHtml, 'text/html')
  const sections = doc.querySelectorAll('.slide')

  if (pageIndex >= 0 && pageIndex < sections.length) {
    sections[pageIndex].remove()
  }

  return '<!DOCTYPE html>\n' + doc.documentElement.outerHTML
}

/** 用隐藏 div + html2canvas 为每页生成缩略图（base64 data URL） */
export async function generateThumbnails(html: string): Promise<string[]> {
  if (!html || !html.trim()) return []

  const { default: html2canvas } = await import('html2canvas')
  const slides = parseSlides(html)
  const thumbnails: string[] = []
  const thumbW = 288
  const thumbH = 162

  // 提取完整 HTML 中的 <style>（含 :root 变量 + BASE_CSS）
  const styleMatch = html.match(/<style>([\s\S]*?)<\/style>/i)
  const styleCss = styleMatch ? styleMatch[1] : `${DEFAULT_THEME_CSS}\n${BASE_CSS}`

  // 创建一个隐藏容器，按设计尺寸渲染每页
  const container = document.createElement('div')
  container.style.cssText = `position:fixed;left:-9999px;top:-9999px;z-index:-1;overflow:hidden;`
  document.body.appendChild(container)

  try {
    for (const slide of slides) {
      const wrapper = document.createElement('div')
      wrapper.style.cssText = `width:${SLIDE_W}px;height:${SLIDE_H}px;overflow:hidden;position:relative;`
      // 注入完整样式（含 :root 变量），确保 var(--accent) 等可解析
      wrapper.innerHTML = `<style>${styleCss}</style>${slide.html}`
      container.appendChild(wrapper)

      try {
        const canvas = await html2canvas(wrapper, {
          width: SLIDE_W,
          height: SLIDE_H,
          scale: thumbW / SLIDE_W,
          useCORS: true,
          logging: false,
          backgroundColor: null
        })
        thumbnails.push(canvas.toDataURL('image/png'))
      } catch {
        thumbnails.push('')
      }

      container.removeChild(wrapper)
    }
  } finally {
    document.body.removeChild(container)
  }

  return thumbnails
}
