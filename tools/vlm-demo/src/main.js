import * as pdfjs from 'pdfjs-dist'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import './style.css'

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.min.mjs',
  import.meta.url
).href

// ═══════════ 设置（localStorage 持久化；API Key 仅存本机浏览器） ═══════════
const SETTINGS_KEY = 'vlm-demo-settings-v1'
const DEFAULTS = {
  // 任意 OpenAI 兼容地址都行：http(s) 地址自动经本地 /p 代理转发（免 CORS）
  baseUrl: 'https://api.siliconflow.cn/v1/chat/completions',
  apiKey: '',
  model: 'PaddlePaddle/PaddleOCR-VL-1.5',
  temperature: 0.1,
  maxTokens: 8192,
  extraBody: '', // 额外请求参数 JSON（如 {"enable_thinking": false}），合并进请求体
  vlmScale: 1.5, // VLM 输入图渲染倍率
  imgFormat: 'jpeg', // jpeg（省流量）| png（文字更锐）
  prompt: [
    '你是高精度文档 OCR 引擎。请将图片中的全部内容完整转录为 Markdown：',
    '1. 严格按阅读顺序输出，保持原文措辞，不要解释、不要编造图中不存在的内容',
    '2. 标题用 # 表示层级；列表用 - 或 1.；正文段落直接换行',
    '3. 表格转换为 Markdown 管道表（| 与 |---|），合并单元格拆分或在单元格内注明',
    '4. 图片/图表位置输出占位：![图：简要描述]',
    '5. 只输出 Markdown 正文，不要任何前后说明文字'
  ].join('\n')
}
// API 平台预设（一键切换 Base URL + 默认模型；Key 不动）
const API_PRESETS = {
  siliconflow: {
    baseUrl: 'https://api.siliconflow.cn/v1/chat/completions',
    model: 'PaddlePaddle/PaddleOCR-VL-1.5'
  },
  aliyun: {
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions',
    model: 'qwen3.8-omni-flash'
  },
  custom: null // 自定义：不动当前配置
}
function detectApiPreset(u) {
  if (u.includes('siliconflow') || u.startsWith('/sf')) return 'siliconflow'
  if (u.includes('aliyuncs.com')) return 'aliyun'
  return 'custom'
}
// 把完整 API 地址转成本地代理路径：https://host/v1/x → /p/host/v1/x
function resolveEndpoint(fullUrl) {
  if (/^https?:\/\//i.test(fullUrl)) {
    const u = new URL(fullUrl)
    return '/p/' + u.host + u.pathname + u.search
  }
  return fullUrl // 本地代理路径（如 /sf/v1/...）直接用
}
// 提示词预设（A/B 对照用）
const PROMPT_PRESETS = {
  detailed: DEFAULTS.prompt,
  minimal: '请把图片中的所有文字内容原样转录为 Markdown，不要添加任何说明。',
  qwenvl: 'qwenvl markdown',
  // OCR 转录器专用（主程序回移候选）：全文照录、列对齐硬约束；删压缩/去重/精炼类改写指令
  ocrTranscribe: [
    '你是文档 OCR 转录引擎。把图片中的全部内容原样转录为 Markdown：保持原文措辞与顺序，禁止解释、总结、改写或补充。',
    '',
    '1. 标题用 # 表示层级；列表用 - 或 1.',
    '2. 表格用 Markdown 管道表：第一行是表头，它决定总列数 N；之后每一行的单元格数必须恰好等于 N——不允许多格、不允许少格；原表横向/纵向合并的单元格，在每个被合并的位置重复输出相同的值（禁止「同上」，禁止留空）',
    '3. 数值、型号、编号、单位完整保留（256 GB、2U，数字与单位间空格）；看不清写 [?]，禁止猜测',
    '4. 跳过页码、水印、页眉页脚；双栏先第一栏后第二栏',
    '5. 只输出转录结果，不要任何说明。'
  ].join('\n'),
  // 表格策略 C：Markdown 载体 + 列数强制对齐（OCR 模型天然吐 markdown，遵循率最高）
  mdExpand: [
    '你是文档 OCR 引擎。将图片中的全部内容完整转录为 Markdown：',
    '',
    '正文规则：',
    '1. 按阅读顺序输出，保持原文措辞，不要解释、不要编造图中不存在的内容',
    '2. 标题用 # 表示层级；列表用 - 或 1.；段落直接换行',
    '',
    '表格规则（必须严格遵守）：',
    '3. 表格用 Markdown 管道表输出（| 与 |---|），第一行是表头，它决定总列数 N',
    '4. 之后每一行的单元格数量必须恰好等于 N：不允许多格、不允许少格',
    '5. 原表横向合并的单元格：在每个被合并的位置重复输出相同的值',
    '6. 原表纵向跨行合并的单元格：在它跨越的每一行都重复输出该值（Markdown 无法表达跨行合并，展开属预期）',
    '7. 单元格内只放纯文本，不要嵌套标签',
    '',
    '只输出转录结果，不要任何前后说明文字。'
  ].join('\n'),
  // 表格策略 A：列数强制对齐，合并处复制值（下游解析永不列错位）
  htmlExpand: [
    '你是文档 OCR 引擎。将图片中的全部内容完整转录：正文用 Markdown，表格用 HTML。',
    '',
    '正文规则：',
    '1. 按阅读顺序输出，保持原文措辞，不要解释、不要编造图中不存在的内容',
    '2. 标题用 # 表示层级；列表用 - 或 1.；段落直接换行',
    '',
    '表格规则（必须严格遵守）：',
    '3. 表格输出为 <table><tr><td>…</td></tr></table>，第一行 <tr> 是表头，它决定总列数 N',
    '4. 之后每一行的 <td> 数量必须恰好等于 N：不允许少格、不允许多格',
    '5. 原表横向合并的单元格：在每个被合并的位置重复输出相同的值（禁止使用 colspan）',
    '6. 原表纵向跨行合并的单元格：在它跨越的每一行都重复输出该值（禁止使用 rowspan）',
    '7. 单元格内只放纯文本，不要嵌套任何标签',
    '',
    '只输出转录结果，不要任何前后说明文字。'
  ].join('\n'),
  // 表格策略 B：如实保留合并结构（colspan/rowspan），用于对照
  htmlMerge: [
    '你是文档 OCR 引擎。将图片中的全部内容完整转录：正文用 Markdown，表格用 HTML。',
    '',
    '正文规则：',
    '1. 按阅读顺序输出，保持原文措辞，不要解释、不要编造图中不存在的内容',
    '2. 标题用 # 表示层级；列表用 - 或 1.；段落直接换行',
    '',
    '表格规则：',
    '3. 表格输出为 <table>，如实反映原表结构：横向合并用 colspan，纵向跨行合并用 rowspan',
    '4. 每一行的总列数（各 td 的 colspan 之和）必须与表头列数一致',
    '5. 单元格内只放纯文本，不要嵌套任何标签',
    '',
    '只输出转录结果，不要任何前后说明文字。'
  ].join('\n')
}

function loadSettings() {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY)
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) }
  } catch { /* ignore */ }
  return { ...DEFAULTS }
}
function saveSettings() {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings))
}
const settings = loadSettings()

// ═══════════ 运行状态 ═══════════
/** @type {{kind:'pdf'|'img', index:number, label:string, thumb:string|null, img:string|null, status:'idle'|'running'|'done'|'error', ms:number, error:string, md:string}[]} */
let pages = []
let running = false
let stopFlag = false
let activeCtl = null
let fileName = ''

// 进度显示
let currentLabel = '' // 正在识别的页标签
let runStartedAt = 0 // 本轮批量起始时刻
let ticker = null // 运行中每 500ms 刷新用时
const PAGE_TIMEOUT_MS = 120000 // 单页 VLM 超时 2 分钟
let uidSeq = 0 // 页面唯一 id（PDF 与图片混导时 index 会撞）

// 运行日志（定位"卡在哪一步"）
const logLines = []
function log(msg) {
  const t = new Date().toTimeString().slice(0, 8)
  logLines.push(`${t} ${msg}`)
  if (logLines.length > 300) logLines.shift()
  const el = document.getElementById('log')
  if (el) {
    el.textContent = logLines.join('\n')
    el.scrollTop = el.scrollHeight
  }
  console.log('[vlm-demo]', msg)
}

// ═══════════ VLM 调用 ═══════════
async function callVlm(imgDataUrl, promptOverride) {
  const prompt = promptOverride ?? settings.prompt
  // 合并额外请求参数（放开配置：enable_thinking、top_p 随便加）
  const bodyObj = {
    model: settings.model,
    messages: [
      {
        role: 'user',
        content: [
          // detail=high：避免被降采样成 448×448 导致文字看不清（SiliconFlow 多模态文档参数）
          { type: 'image_url', image_url: { url: imgDataUrl, detail: 'high' } },
          { type: 'text', text: prompt }
        ]
      }
    ],
    temperature: Number(settings.temperature) || 0,
    max_tokens: Number(settings.maxTokens) || 8192,
    stream: false
  }
  const extraRaw = String(settings.extraBody || '').trim()
  if (extraRaw) {
    let extra
    try {
      extra = JSON.parse(extraRaw)
    } catch {
      throw new Error('「额外请求参数」不是合法 JSON：' + extraRaw.slice(0, 100))
    }
    Object.assign(bodyObj, extra)
  }
  const ctl = new AbortController()
  activeCtl = ctl
  const res = await fetch(resolveEndpoint(settings.baseUrl), {
    method: 'POST',
    signal: ctl.signal,
    headers: {
      'Content-Type': 'application/json',
      ...(settings.apiKey ? { Authorization: `Bearer ${settings.apiKey}` } : {})
    },
    body: JSON.stringify(bodyObj)
  })
  if (!res.ok) {
    const t = await res.text().catch(() => '')
    log(`VLM HTTP ${res.status}：${t.slice(0, 200)}`)
    throw new Error(`HTTP ${res.status} ${t.slice(0, 400)}`)
  }
  const data = await res.json()
  log(`VLM HTTP ${res.status} 收到响应`)
  const content = data?.choices?.[0]?.message?.content
  if (typeof content !== 'string') throw new Error('响应结构异常：' + JSON.stringify(data).slice(0, 300))
  return { content, raw: JSON.stringify(data, null, 2).slice(0, 60000) }
}

// ═══════════ 模型列表拉取（任意 OpenAI 兼容 /models） ═══════════
async function fetchModels() {
  const base = String(settings.baseUrl).replace(/\/chat\/completions\/?$/, '')
  const url = resolveEndpoint(base + '/models')
  log('拉取模型列表：' + url)
  const r = await fetch(url, {
    headers: { ...(settings.apiKey ? { Authorization: `Bearer ${settings.apiKey}` } : {}) }
  })
  if (!r.ok) {
    const t = await r.text().catch(() => '')
    throw new Error(`HTTP ${r.status} ${t.slice(0, 200)}`)
  }
  const d = await r.json()
  const ids = (d?.data || []).map((m) => m?.id).filter(Boolean).sort()
  const dl = document.getElementById('model-datalist')
  if (dl) dl.innerHTML = ids.map((id) => `<option value="${escapeHtml(id)}"></option>`).join('')
  log(`拉取到 ${ids.length} 个模型（已填入模型输入框的候选下拉）`)
  return ids
}

// ═══════════ 一键体检：模型存在性 + 图片通路（决定性测试） ═══════════
async function runDiagnostics() {
  if (!settings.apiKey.trim()) {
    alert('请先填写 API Key')
    return
  }
  const report = []
  log('── 体检开始 ──')

  // ① 模型是否在平台列表
  try {
    const ids = await fetchModels()
    const exists = ids.includes(settings.model)
    const fuzzy = ids.filter((s) => /paddle|ocr/i.test(s))
    report.push(exists
      ? `① 模型存在：${settings.model} ✓（列表共 ${ids.length} 个）`
      : `① 模型【不在】列表！${settings.model}\n   含 OCR/Paddle 的可选：${fuzzy.join(', ') || '（无）'}\n   列表共 ${ids.length} 个模型`)
  } catch (e) {
    report.push('① 模型列表查询失败：' + (e?.message || e))
  }

  // ② 已知文字小图 → 模型能否读出来（图片通路决定性测试）
  const MARK = 'ARK-TEST-1234'
  const c = document.createElement('canvas')
  c.width = 480
  c.height = 160
  const ctx = c.getContext('2d')
  ctx.fillStyle = '#fff'
  ctx.fillRect(0, 0, 480, 160)
  ctx.fillStyle = '#000'
  ctx.font = 'bold 48px monospace'
  ctx.fillText(MARK, 30, 100)
  const img = c.toDataURL('image/jpeg', 0.95)
  const TEST_PROMPT = '请原样转录图片中的文字，只输出文字本身。'

  const tryVariant = async (name, contentBuilder) => {
    try {
      log(`体检[${name}] 发送内置测试图（${MARK}）…`)
      const ctl = new AbortController()
      activeCtl = ctl
      const res = await fetch(resolveEndpoint(settings.baseUrl), {
        method: 'POST',
        signal: ctl.signal,
        headers: {
          'Content-Type': 'application/json',
          ...(settings.apiKey ? { Authorization: `Bearer ${settings.apiKey}` } : {})
        },
        body: JSON.stringify({
          model: settings.model,
          messages: [{ role: 'user', content: contentBuilder(TEST_PROMPT) }],
          temperature: 0,
          max_tokens: 512,
          stream: false
        })
      })
      if (!res.ok) throw new Error(`HTTP ${res.status} ${(await res.text().catch(() => '')).slice(0, 200)}`)
      const data = await res.json()
      const content = data?.choices?.[0]?.message?.content
      const hit = typeof content === 'string' && content.includes(MARK)
      log(`体检[${name}] ${hit ? '✓ 读出' : '✗ 未读出'}：${String(content || '').slice(0, 120)}`)
      return { hit, content: String(content || '') }
    } catch (e) {
      log(`体检[${name}] 失败：${e?.message || e}`)
      return { hit: false, content: '调用失败：' + (e?.message || e) }
    }
  }

  // 变体 A：图在前 + detail=high（当前正式调用格式）
  const a = await tryVariant('A 图在前+detail', (t) => [
    { type: 'image_url', image_url: { url: img, detail: 'high' } },
    { type: 'text', text: t }
  ])
  let hit = a.hit
  let diagNote = ''
  if (!hit) {
    // 变体 B：文本在前 + 无 detail（官方 curl 示例顺序）
    const b = await tryVariant('B 文本在前无detail', (t) => [
      { type: 'text', text: t },
      { type: 'image_url', image_url: { url: img } }
    ])
    hit = b.hit
    diagNote = `\n   变体A(图前+detail)：${a.hit ? '✓' : '✗'} ${a.content.slice(0, 80)}\n   变体B(文本前无detail)：${b.hit ? '✓' : '✗'} ${b.content.slice(0, 80)}`
  }
  report.push(hit
    ? `② 图片通路正常 ✓ 模型读出了 "${MARK}"`
    : `② 图片通路【异常】✗ 两种格式模型都没读出测试文字。${diagNote}`)
  if (!hit) report.push('   → 模型没收到图片/不支持图片输入，只会拿文本提示词作答（与你看到的"复述提示词"完全一致）')

  const text = report.join('\n\n')
  log('── 体检结果 ──\n' + text)
  alert('体检结果\n\n' + text)
}

// ═══════════ PDF 渲染 ═══════════
async function renderPageImage(pdf, pageNum, scale, format = 'jpeg') {
  const page = await pdf.getPage(pageNum)
  const viewport = page.getViewport({ scale })
  const canvas = document.createElement('canvas')
  canvas.width = Math.floor(viewport.width)
  canvas.height = Math.floor(viewport.height)
  const ctx = canvas.getContext('2d')
  ctx.fillStyle = '#ffffff' // 防透明底渲染成白底
  ctx.fillRect(0, 0, canvas.width, canvas.height)
  await page.render({ canvasContext: ctx, viewport }).promise
  const dataUrl = format === 'png'
    ? canvas.toDataURL('image/png')
    : canvas.toDataURL('image/jpeg', 0.95)
  // 诊断：采样统计非白像素占比（判断渲染是否空白）
  let ink = 0
  let sampled = 0
  try {
    const step = 16
    const img = ctx.getImageData(0, 0, canvas.width, canvas.height).data
    for (let y = 0; y < canvas.height; y += step) {
      for (let x = 0; x < canvas.width; x += step) {
        const i = (y * canvas.width + x) * 4
        sampled++
        if (img[i] < 245 || img[i + 1] < 245 || img[i + 2] < 245) ink++
      }
    }
  } catch { /* 采样失败不阻断 */ }
  return {
    dataUrl,
    w: canvas.width,
    h: canvas.height,
    ink: sampled ? Math.round((ink / sampled) * 100) : -1,
    bytes: dataUrl.length
  }
}

async function renderThumb(p) {
  if (!pdfDoc || p.thumb) return
  const page = await pdfDoc.getPage(p.index)
  const w = page.getViewport({ scale: 1 }).width
  const scale = Math.min(160 / w, 1)
  p.thumb = (await renderPageImage(pdfDoc, p.index, Math.max(scale, 0.1), 'jpeg')).dataUrl
}

let pdfDoc = null

async function importPdf(file) {
  fileName = file.name
  const buf = await file.arrayBuffer()
  pdfDoc = await pdfjs.getDocument({ data: buf }).promise
  pages = []
  for (let i = 1; i <= pdfDoc.numPages; i++) {
    pages.push({ kind: 'pdf', index: i, uid: ++uidSeq, label: `第 ${i} 页`, thumb: null, img: null, status: 'idle', ms: 0, error: '', md: '' })
  }
  render()
  // 缩略图限流生成
  let next = 0
  const workers = Array.from({ length: 4 }, async () => {
    while (next < pages.length && !stopFlag) {
      const p = pages[next++]
      try { await renderThumb(p); renderPageCard(p) } catch { /* 单页缩略图失败不阻断 */ }
    }
  })
  await Promise.all(workers)
  log(`已导入 ${fileName}，${pages.length} 页`)
  render()
}

async function importImage(file) {
  const dataUrl = await new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => resolve(r.result)
    r.onerror = reject
    r.readAsDataURL(file)
  })
  pages.push({
    kind: 'img',
    index: pages.length + 1,
    uid: ++uidSeq,
    label: `图片 ${file.name}`,
    thumb: dataUrl,
    img: dataUrl,
    status: 'idle',
    ms: 0,
    error: '',
    md: ''
  })
  render()
}

// ═══════════ 识别 ═══════════
async function runOne(p) {
  if (p.status === 'running') return
  p.status = 'running'
  p.error = ''
  currentLabel = p.label
  if (!ticker) {
    // 单页直接识别：自行起计时与刷新（批量时由 runAll 统一管理）
    runStartedAt = performance.now()
    ticker = setInterval(renderHeaderStats, 500)
  }
  renderPageCard(p)
  renderHeaderStats()
  document.getElementById(`pc-${p.uid}`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  const t0 = performance.now()
  let timedOut = false
  const timer = setTimeout(() => {
    timedOut = true
    activeCtl?.abort()
  }, PAGE_TIMEOUT_MS)
  try {
    if (!p.img) {
      if (p.kind !== 'pdf') throw new Error('缺少图片数据')
      const r = await renderPageImage(pdfDoc, p.index, Number(settings.vlmScale) || 1.5, settings.imgFormat)
      p.img = r.dataUrl
      p.imgInfo = r
      log(`${p.label} 图就绪 ${r.w}×${r.h} ${(r.bytes / 1024 / 1024).toFixed(2)}MB 非白${r.ink}%`)
    }
    log(`${p.label} VLM 请求发出…`)
    const { content, raw } = await callVlm(p.img)
    log(`${p.label} VLM 响应完成`)
    p.md = content
    p.raw = raw
    p.status = 'done'
  } catch (e) {
    if (timedOut) {
      p.status = 'error'
      p.error = `超时（>${PAGE_TIMEOUT_MS / 1000}s），已中止本页`
    } else if (e?.name === 'AbortError') {
      p.status = 'idle'
      log(`${p.label} 被中止`)
    } else {
      p.status = 'error'
      p.error = String(e?.message || e)
    }
    log(`${p.label} 失败：${p.error}`)
  } finally {
    clearTimeout(timer)
  }
  p.ms = Math.round(performance.now() - t0)
  renderPageCard(p)
  renderHeaderStats()
  if (!running && ticker) {
    // 单页模式收尾
    currentLabel = ''
    clearInterval(ticker)
    ticker = null
    renderHeaderStats()
  }
}

async function runAll() {
  if (running) return
  if (!settings.apiKey.trim()) {
    alert('请先在左侧填写 API Key')
    return
  }
  running = true
  stopFlag = false
  activeCtl = null
  runStartedAt = performance.now()
  currentLabel = ''
  ticker = setInterval(renderHeaderStats, 500)
  renderHeaderStats()
  log(`批量识别开始，共 ${pages.filter((p) => p.status !== 'done').length} 页待识别`)
  try {
    for (const p of pages) {
      if (stopFlag) {
        log('收到停止指令，中断批量')
        break
      }
      if (p.status === 'done') continue
      try {
        await runOne(p)
      } catch (e) {
        // 单页内部异常绝不中断批量
        p.status = 'error'
        p.error = '内部异常：' + String(e?.message || e)
        renderPageCard(p)
        log(`${p.label} 内部异常：${e?.stack || e}`)
      }
    }
  } finally {
    running = false
    activeCtl = null
    currentLabel = ''
    clearInterval(ticker)
    ticker = null
    renderHeaderStats()
    const done = pages.filter((p) => p.status === 'done').length
    log(`批量结束：完成 ${done}/${pages.length}`)
  }
}

function stopRun() {
  stopFlag = true
  activeCtl?.abort()
}

// ═══════════ 导出 ═══════════
function exportMarkdown() {
  const done = pages.filter((p) => p.status === 'done' && p.md)
  if (!done.length) {
    alert('还没有已识别的页面')
    return
  }
  const text = done.map((p) => `<!-- ${p.label} -->\n\n${p.md}`).join('\n\n---\n\n')
  const blob = new Blob([text], { type: 'text/markdown;charset=utf-8' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = (fileName.replace(/\.pdf$/i, '') || 'vlm') + '.md'
  a.click()
  URL.revokeObjectURL(a.href)
}

// ═══════════ 渲染 ═══════════
const mdRender = (md) => DOMPurify.sanitize(marked.parse(md || '', { async: false }))

function statusBadge(p) {
  if (p.status === 'running') return '<span class="badge running">识别中…</span>'
  if (p.status === 'done') return `<span class="badge done">${p.ms} ms</span>`
  if (p.status === 'error') return '<span class="badge error">失败</span>'
  return '<span class="badge idle">待识别</span>'
}

function pageCardHtml(p) {
  const info = p.imgInfo
    ? `<span class="imginfo" title="VLM 输入图：宽×高、体积、非白像素占比（过低说明渲染空白）">${p.imgInfo.w}×${p.imgInfo.h} · ${(p.imgInfo.bytes / 1024 / 1024).toFixed(2)}MB · 非白 ${p.imgInfo.ink}%</span>`
    : ''
  let result = ''
  if (p.status === 'done') {
    result = `<div class="result"><div class="tabs">
      <button data-act="preview" data-u="${p.uid}" class="active">预览</button>
      <button data-act="raw" data-u="${p.uid}">源码</button>
      <button data-act="resp" data-u="${p.uid}">原始响应</button>
      <button data-act="input" data-u="${p.uid}">输入图</button>
    </div><div class="md" id="md-${p.uid}">${mdRender(p.md)}</div></div>`
  } else if (p.status === 'error') {
    result = `<div class="result err">${escapeHtml(p.error)}</div>`
  }
  return `
    <div class="page-card" id="pc-${p.uid}">
      <div class="page-head">
        ${p.thumb ? `<img class="thumb" data-act="input" data-u="${p.uid}" src="${p.thumb}" title="点击查看送入 VLM 的图" />` : '<div class="thumb placeholder"></div>'}
        <div class="page-meta">
          <span class="page-label">${escapeHtml(p.label)}</span>
          ${statusBadge(p)}
          ${info}
        </div>
        <div class="page-actions">
          <button data-act="run" data-u="${p.uid}" ${running ? 'disabled' : ''}>${p.status === 'done' ? '重试' : '识别'}</button>
        </div>
      </div>
      ${result}
    </div>`
}

function escapeHtml(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function renderHeaderStats() {
  const el = document.getElementById('stats')
  if (!el) return
  const total = pages.length
  const done = pages.filter((p) => p.status === 'done').length
  const failed = pages.filter((p) => p.status === 'error').length
  const totalMs = pages.filter((p) => p.status === 'done').reduce((s, p) => s + p.ms, 0)
  let text = ''
  if (total) {
    text = `进度 ${done}/${total} 页`
    if (failed) text += ` · 失败 ${failed}`
    if (done) text += ` · 总耗时 ${(totalMs / 1000).toFixed(1)}s · 均 ${Math.round(totalMs / done)}ms/页`
    if (running) {
      const elapsed = ((performance.now() - runStartedAt) / 1000).toFixed(0)
      text += ` · 本轮已用 ${elapsed}s`
      if (currentLabel) text += ` · 当前：${currentLabel}`
    }
  }
  el.textContent = text

  // 顶部进度条（已完成占比；运行中无已完成时给流动感）
  const bar = document.getElementById('bar')
  if (bar) {
    const pct = total ? Math.round((done / total) * 100) : 0
    bar.style.width = pct + '%'
    bar.classList.toggle('indeterminate', running && pct === 0)
  }

  const stopBtn = document.getElementById('btn-stop')
  const runBtn = document.getElementById('btn-run-all')
  if (stopBtn) stopBtn.disabled = !running
  if (runBtn) runBtn.disabled = running
}

function renderPageCard(p) {
  const old = document.getElementById(`pc-${p.uid}`)
  if (!old) return render()
  const tmp = document.createElement('div')
  tmp.innerHTML = pageCardHtml(p)
  old.replaceWith(tmp.firstElementChild)
  bindPageActions()
}

function bindPageActions() {
  document.querySelectorAll('[data-act="run"]').forEach((btn) => {
    btn.onclick = () => {
      const p = pages.find((x) => x.uid === Number(btn.dataset.u))
      if (p) runOne(p)
    }
  })
  // 预览/源码/原始响应/输入图 切换（缩略图点击也走 input）
  document.querySelectorAll('[data-act="preview"],[data-act="raw"],[data-act="resp"],[data-act="input"]').forEach((el) => {
    el.onclick = () => {
      const p = pages.find((x) => x.uid === Number(el.dataset.u))
      if (!p) return
      if (el.dataset.act === 'input') return showInputImage(p)
      const box = document.getElementById(`md-${p.uid}`)
      if (!box) return
      if (el.dataset.act === 'preview') box.innerHTML = mdRender(p.md)
      else if (el.dataset.act === 'raw') box.innerHTML = `<pre class="raw">${escapeHtml(p.md)}</pre>`
      else box.innerHTML = `<pre class="raw">${escapeHtml(p.raw || '（无原始响应）')}</pre>`
      // 高亮当前 tab（仅 result 内的 tabs 按钮）
      el.parentElement?.querySelectorAll('button').forEach((b) => b.classList.toggle('active', b === el))
    }
  })
}

async function showInputImage(p) {
  if (!p.img) {
    if (p.kind === 'pdf' && pdfDoc) {
      const r = await renderPageImage(pdfDoc, p.index, Number(settings.vlmScale) || 1.5, settings.imgFormat)
      p.img = r.dataUrl
      p.imgInfo = r
      renderPageCard(p)
    } else {
      alert('该页尚无输入图（图片导入的页面应已有）')
      return
    }
  }
  const lb = document.getElementById('lightbox')
  lb.querySelector('img').src = p.img
  lb.querySelector('.lb-cap').textContent =
    `${p.label} · 输入图 ${p.imgInfo ? `${p.imgInfo.w}×${p.imgInfo.h} · ${(p.imgInfo.bytes / 1024 / 1024).toFixed(2)}MB · 非白 ${p.imgInfo.ink}%` : ''} — 这就是发给 VLM 的原图`
  lb.classList.add('show')
}

function render() {
  const list = document.getElementById('pageList')
  if (!pages.length) {
    list.innerHTML = '<div class="empty">导入 PDF 或图片后开始</div>'
  } else {
    list.innerHTML = pages.map(pageCardHtml).join('')
    bindPageActions()
  }
  renderHeaderStats()
}

// ═══════════ 设置绑定 ═══════════
function bindSettings() {
  const map = {
    'set-baseUrl': 'baseUrl',
    'set-apiKey': 'apiKey',
    'set-model': 'model',
    'set-temperature': 'temperature',
    'set-maxTokens': 'maxTokens',
    'set-extraBody': 'extraBody',
    'set-vlmScale': 'vlmScale',
    'set-imgFormat': 'imgFormat',
    'set-prompt': 'prompt'
  }
  for (const [id, key] of Object.entries(map)) {
    const el = document.getElementById(id)
    if (!el) continue
    el.value = settings[key]
    el.addEventListener('input', () => {
      settings[key] = el.value
      saveSettings()
    })
    el.addEventListener('change', () => {
      settings[key] = el.value
      saveSettings()
    })
  }
  // API 平台预设：一键切换 Base URL + 默认模型
  const presetSel = document.getElementById('set-apiPreset')
  presetSel.value = detectApiPreset(settings.baseUrl)
  presetSel.onchange = () => {
    const p = API_PRESETS[presetSel.value]
    if (!p) return // 自定义：不动
    settings.baseUrl = p.baseUrl
    settings.model = p.model
    saveSettings()
    bindSettingsValues()
    presetSel.value = presetSel.value // 保持
    log(`已切换预设：${presetSel.value} → ${p.baseUrl} / ${p.model}`)
  }
  // 拉取模型列表
  document.getElementById('btn-fetch-models').onclick = async () => {
    const btn = document.getElementById('btn-fetch-models')
    btn.disabled = true
    btn.textContent = '拉取中…'
    try {
      const ids = await fetchModels()
      btn.textContent = `拉取模型列表（${ids.length}）`
    } catch (e) {
      alert('拉取失败：' + (e?.message || e))
      btn.textContent = '拉取模型列表'
    } finally {
      btn.disabled = false
    }
  }
  document.getElementById('btn-diag').onclick = runDiagnostics
  document.getElementById('btn-reset-settings').onclick = () => {
    if (!confirm('恢复默认设置？（API Key 不会被清除）')) return
    const key = settings.apiKey
    Object.assign(settings, DEFAULTS, { apiKey: key })
    saveSettings()
    bindSettingsValues()
    document.getElementById('set-apiPreset').value = detectApiPreset(settings.baseUrl)
  }
}
function bindSettingsValues() {
  const map = {
    'set-baseUrl': 'baseUrl',
    'set-apiKey': 'apiKey',
    'set-model': 'model',
    'set-temperature': 'temperature',
    'set-maxTokens': 'maxTokens',
    'set-extraBody': 'extraBody',
    'set-vlmScale': 'vlmScale',
    'set-imgFormat': 'imgFormat',
    'set-prompt': 'prompt'
  }
  for (const [id, key] of Object.entries(map)) {
    const el = document.getElementById(id)
    if (el) el.value = settings[key]
  }
}

// ═══════════ 模板 ═══════════
document.getElementById('app').innerHTML = `
<div class="layout">
  <aside class="side">
    <h1>VLM PDF 解析试验台</h1>
    <details open>
      <summary>VLM 设置</summary>
      <div class="form">
        <label>API 预设
          <select id="set-apiPreset">
            <option value="siliconflow">SiliconFlow</option>
            <option value="aliyun">阿里百炼 DashScope</option>
            <option value="custom">自定义（不改动）</option>
          </select>
        </label>
        <label>Base URL<span class="hint">（任意 OpenAI 兼容地址，自动经本地 /p 代理，无 CORS 限制）</span>
          <input id="set-baseUrl" type="text" spellcheck="false" /></label>
        <label>API Key
          <input id="set-apiKey" type="password" placeholder="sk-…" spellcheck="false" /></label>
        <label>模型
          <input id="set-model" type="text" list="model-datalist" spellcheck="false" /></label>
        <datalist id="model-datalist"></datalist>
        <button id="btn-fetch-models" class="ghost">拉取模型列表（用当前 Key）</button>
        <div class="row2">
          <label>温度<input id="set-temperature" type="number" step="0.1" min="0" max="2" /></label>
          <label>max_tokens<input id="set-maxTokens" type="number" step="256" min="256" /></label>
        </div>
        <label>额外请求参数 JSON<span class="hint">（可选，合并进请求体，如 {"enable_thinking": false}）</span>
          <textarea id="set-extraBody" rows="2" spellcheck="false" placeholder='{"enable_thinking": false}'></textarea></label>
        <div class="row2">
          <label>图倍率<input id="set-vlmScale" type="number" step="0.1" min="0.5" max="4" /></label>
          <label>图片格式
            <select id="set-imgFormat"><option value="jpeg">jpeg</option><option value="png">png</option></select>
          </label>
        </div>
        <label>提示词预设
          <select id="set-preset">
            <option value="detailed">详细规则（默认）</option>
            <option value="minimal">极简转录（A/B 对照）</option>
            <option value="qwenvl">qwenvl markdown（Qwen 文档解析）</option>
            <option value="ocrTranscribe">OCR 转录·回移候选（主程序新版）</option>
            <option value="mdExpand">Markdown 列对齐·合并复制值（OCR 模型推荐）</option>
            <option value="htmlExpand">HTML 列对齐·合并复制值（表格推荐）</option>
            <option value="htmlMerge">HTML 保留合并·colspan/rowspan（对照）</option>
          </select>
        </label>
        <label>提示词<textarea id="set-prompt" rows="10" spellcheck="false"></textarea></label>
        <div class="row2">
          <button id="btn-reset-settings" class="ghost">恢复默认（保留 Key）</button>
          <button id="btn-diag" class="ghost">一键体检</button>
        </div>
        <p class="hint">一键体检：查模型是否在平台列表 + 用内置已知文字测试图验证图片通路（定位"复述提示词"是模型问题还是图片问题）。</p>
        <p class="hint">设置自动保存在本机浏览器 localStorage；API Key 不会离开本机（除调用所填接口）。</p>
      </div>
    </details>
    <details open>
      <summary>运行日志</summary>
      <pre id="log" class="logbox">—</pre>
    </details>
    <details open>
      <summary>导入</summary>
      <div class="form">
        <label class="file-btn">选择 PDF<input id="file-pdf" type="file" accept="application/pdf" /></label>
        <label class="file-btn ghost">选择图片（单图测试）<input id="file-img" type="file" accept="image/*" /></label>
        <p class="hint">PDF 逐页渲染后送 VLM；图片直接作为一页测试。</p>
      </div>
    </details>
  </aside>
  <main class="main">
    <header class="topbar">
      <span id="fileName" class="fname">未导入</span>
      <span id="stats" class="stats"></span>
      <div class="top-actions">
        <button id="btn-run-all" class="primary">全部识别</button>
        <button id="btn-stop" class="ghost" disabled>停止</button>
        <button id="btn-export" class="ghost">导出 Markdown</button>
      </div>
    </header>
    <div class="progress"><div class="bar" id="bar"></div></div>
    <div id="pageList" class="pages"></div>
  </main>
</div>
<div id="lightbox" class="lightbox"><div class="lb-inner"><img alt="" /><div class="lb-cap"></div></div></div>
`

// ═══════════ 事件绑定 ═══════════
bindSettings()

document.getElementById('file-pdf').onchange = async (e) => {
  const f = e.target.files?.[0]
  e.target.value = ''
  if (!f) return
  stopFlag = false
  try {
    await importPdf(f)
    document.getElementById('fileName').textContent = fileName
  } catch (err) {
    alert('PDF 打开失败：' + (err?.message || err))
  }
}
document.getElementById('file-img').onchange = async (e) => {
  const f = e.target.files?.[0]
  e.target.value = ''
  if (f) await importImage(f)
}
document.getElementById('btn-run-all').onclick = runAll
document.getElementById('btn-stop').onclick = stopRun
document.getElementById('btn-export').onclick = exportMarkdown

// 提示词预设切换
document.getElementById('set-preset').onchange = (e) => {
  const preset = PROMPT_PRESETS[e.target.value]
  if (!preset) return
  settings.prompt = preset
  saveSettings()
  document.getElementById('set-prompt').value = preset
}

// 灯箱关闭
document.getElementById('lightbox').onclick = (e) => {
  if (e.target.id === 'lightbox' || e.target.closest('.lb-cap')) {
    document.getElementById('lightbox').classList.remove('show')
  }
}
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') document.getElementById('lightbox').classList.remove('show')
})

// 未捕获异常也进日志（排查"卡住"必备）
window.addEventListener('unhandledrejection', (e) => {
  log('未捕获异常：' + String(e.reason?.stack || e.reason || e))
})
window.addEventListener('error', (e) => {
  log('脚本错误：' + e.message)
})

log('就绪。导入 PDF 后点「全部识别」；卡住时看本日志最后几行。')
render()
