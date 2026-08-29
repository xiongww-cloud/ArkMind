import { invoke } from '@tauri-apps/api/core'
import {
  BLOCK_SCALES,
  BLOCK_SPANS,
  BLOCKS,
  LAYOUTS,
  layoutAllows,
  uid,
  validateSlide
} from '@/presentation/elements'
import { fitSlide } from '@/presentation/fit'
import type { Slide } from '@/types/presentation'

/** 是否运行在 Tauri 桌面端（浏览器 dev 下为 false，AI 不可用） */
export function isTauri(): boolean {
  return (
    typeof window !== 'undefined' &&
    !!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  )
}

export const AI_UNAVAILABLE_MSG =
  '演示材料 AI 需在桌面端运行。请在「系统设置 → AI 配置」配置 Chat API 后重试。'

/** 把元素注册表序列化为给模型的提示摘要（保持 elements.ts 为单一事实源） */
export function buildRegistrySummary(): string {
  const lines: string[] = []
  lines.push('## 版式（layout）')
  for (const l of LAYOUTS) {
    lines.push(
      `- ${l.id}（${l.name}，${l.shell === 'hero' ? '居中' : '标题+正文'}）：允许区块 ${l.allowedBlocks.join('/')}`
    )
  }
  lines.push('## 区块（block）及 data 字段')
  const fieldDoc: Record<string, string> = {
    kicker: 'text: 眉题',
    heading: 'text: 标题',
    paragraph: 'text: 段落纯文本',
    bullets: 'items: string[]',
    'num-list': 'items: string[]',
    table: 'columns: [{id,width?}], rows: [{cells:[{content}]}]，rows[0]为表头行',
    'kpi-row': 'items: [{value,label,sub?}]',
    'cards-grid': 'items: [{icon,title,desc}]',
    columns: 'left/right: {title, items: string[]}',
    timeline: 'items: [{time,title,desc?}]',
    image: 'src?: 图片（一般留空为占位）',
    chart: 'kind: "bar"|"line", labels: string[], values: number[]',
    quote: 'text, author?',
    cta: 'text, subtext?',
    divider: '（无字段）'
  }
  for (const b of BLOCKS) {
    lines.push(`- ${b.type}（${b.name}）：${fieldDoc[b.type] ?? b.props.join(',')}`)
  }
  lines.push(`## 跨度 span 可选值：${BLOCK_SPANS.map((s) => `${s.value}`).join('/')}`)
  lines.push(`## 字号 scale 可选值：${BLOCK_SCALES.map((s) => `${s.value}`).join('/')}`)
  return lines.join('\n')
}

// ─── 模型输出规整（修复"修改不生效"：标题丢失 / 布局不允许丢块） ──
/** 把模型产出的 slide 原始 JSON 规整为合法 Slide：顶层 heading/title → heading 区块 */
export function normalizeAiSlide(raw: unknown): Slide {
  const s = (raw ?? {}) as {
    layout?: string
    heading?: string
    title?: string
    blocks?: unknown[]
    note?: string
  }
  const layout =
    typeof s.layout === 'string' && LAYOUTS.some((l) => l.id === s.layout) ? s.layout : 'content'
  const heading =
    typeof s.heading === 'string' ? s.heading : typeof s.title === 'string' ? s.title : ''
  const blocks = Array.isArray(s.blocks) ? (s.blocks as Array<Record<string, unknown>>) : []
  const hasHeading = blocks.some((b) => b?.type === 'heading')
  const list =
    heading && !hasHeading ? [{ type: 'heading', data: { text: heading } }, ...blocks] : blocks
  return { id: uid(), layout, blocks: list as never, note: s.note }
}

/** 布局不允许的区块 → 自动升级为 content 版式，避免 validateSlide 静默丢块 */
export function ensureLayoutCompat(slide: Slide): Slide {
  if (slide.blocks.some((b) => !layoutAllows(slide.layout, b.type))) {
    return { ...slide, layout: 'content' }
  }
  return slide
}

/** 完整规整链路：模型原始输出 → 标题规整 → 版面裁剪 → 布局兼容 → 校验 */
export function finalizeAiSlide(raw: unknown): Slide {
  return validateSlide(ensureLayoutCompat(fitSlide(normalizeAiSlide(raw))))
}

// ─── 可注入 AI 驱动（生产默认真实调用；测试可替换为 mock 驱动） ──
export interface PresentationAiDriver {
  generateDeck(plan: string, context: string): Promise<Slide[]>
  reviseDeck(deck: Slide[], instruction: string, context: string): Promise<Slide[]>
  refineSlide(slide: Slide, instruction: string, context: string): Promise<Slide>
}

const realDriver: PresentationAiDriver = {
  async generateDeck(plan, context) {
    if (!isTauri()) throw new Error(AI_UNAVAILABLE_MSG)
    const registry = buildRegistrySummary()
    const r = (await invoke('presentation_generate_deck', {
      data: { plan, context, registry }
    })) as unknown as { success: boolean; slides?: unknown[]; error?: string }
    if (!r?.success) throw new Error(r?.error || '生成失败')
    const slides = (r.slides ?? []).map(finalizeAiSlide)
    if (slides.length < 3) throw new Error('AI 生成结果页数过少，请调整需求后重试')
    return slides
  },
  async reviseDeck(deck, instruction, context) {
    if (!isTauri()) throw new Error(AI_UNAVAILABLE_MSG)
    const registry = buildRegistrySummary()
    const r = (await invoke('presentation_revise_deck', {
      data: { deck: JSON.stringify(deck), instruction, context, registry }
    })) as unknown as { success: boolean; slides?: unknown[]; error?: string }
    if (!r?.success) throw new Error(r?.error || '修改失败')
    const slides = (r.slides ?? []).map(finalizeAiSlide)
    if (slides.length < 3) throw new Error('AI 修改结果页数过少，请调整需求后重试')
    return slides
  },
  async refineSlide(slide, instruction, context) {
    if (!isTauri()) throw new Error(AI_UNAVAILABLE_MSG)
    const registry = buildRegistrySummary()
    const r = (await invoke('presentation_refine_slide', {
      data: { slide: JSON.stringify(slide), instruction, context, registry }
    })) as unknown as { success: boolean; slide?: unknown; error?: string }
    if (!r?.success) throw new Error(r?.error || '精修失败')
    return finalizeAiSlide({ ...((r.slide ?? {}) as object), note: slide.note })
  }
}

let activeDriver: PresentationAiDriver = realDriver

export function setPresentationAiDriver(d: PresentationAiDriver) {
  activeDriver = d
}
export function getPresentationAiDriver(): PresentationAiDriver {
  return activeDriver
}
