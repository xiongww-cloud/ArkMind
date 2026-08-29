import type { Slide } from '@/types/presentation'
import { sampleBlock } from '@/presentation/elements'
import { B } from '@/presentation/builders'

export interface RefineResult {
  applied: string
  /** 若需要切换全局主题（如"深色"）则返回目标主题 id */
  theme?: string
}

/** 取单页标题文本 */
function slideHeading(slide: Slide): string {
  return (
    ((slide.blocks.find((b) => b.type === 'heading')?.data as unknown as Record<string, unknown>)
      ?.text as string) || ''
  )
}

/**
 * 关键词精修规则（供测试 mock 驱动 / 无 AI 场景复用）。
 * 原地修改 slide，返回执行说明（可能附带 theme）。
 * 注意：这是旧 mock 逻辑，正式环境一律走真实 AI。
 */
export function applyRefineRules(slide: Slide, instruction: string): RefineResult {
  const heading = slideHeading(slide)
  const ins = instruction.toLowerCase()
  let applied = ''
  let theme: string | undefined

  if (/深色|暗色|dark/.test(ins)) {
    theme = 'dark'
    applied = '全局主题已切换为深色'
  } else if (/表格|table/.test(ins)) {
    slide.layout = 'table'
    slide.blocks = [
      ...slide.blocks.filter((b) => b.type === 'heading' || b.type === 'kicker'),
      B.table(
        ['要点', '说明'],
        [
          ['A', '内容A'],
          ['B', '内容B'],
          ['C', '内容C']
        ]
      )
    ]
    applied = '本页已切换为「表格页」版式'
  } else if (/对比|左右|columns|分栏/.test(ins)) {
    slide.layout = 'content'
    slide.blocks = [
      ...slide.blocks.filter((b) => b.type !== 'columns'),
      B.columns(
        { title: '方案A', items: ['优势一', '优势二'] },
        { title: '方案B', items: ['优势一', '优势二'] }
      )
    ]
    applied = '已在本页加入左右对比分栏'
  } else if (/时间轴|timeline|阶段|步骤/.test(ins)) {
    slide.layout = 'timeline'
    slide.blocks = [
      ...slide.blocks.filter((b) => b.type === 'heading' || b.type === 'kicker'),
      B.timeline([
        { time: '一期', title: '基础建设', desc: '平台部署与核心模块上线' },
        { time: '二期', title: '深化应用', desc: '业务系统全面接入' },
        { time: '三期', title: '持续优化', desc: '数据运营与增值服务' }
      ])
    ]
    applied = '本页已切换为「时间轴页」版式'
  } else if (/数据|指标|kpi|数字/.test(ins)) {
    slide.layout = 'stats'
    slide.blocks = [
      ...slide.blocks.filter((b) => b.type === 'heading' || b.type === 'kicker'),
      B.kpis([
        { value: '99.9%', label: '可用性', sub: 'SLA 保障' },
        { value: '45s', label: '平均响应', sub: '峰值亦保持' },
        { value: '-44%', label: '成本下降', sub: '同比' }
      ])
    ]
    applied = '本页已切换为「数据页」版式'
  } else if (/卡片|分类|cards/.test(ins)) {
    slide.layout = 'content'
    slide.blocks = [
      ...slide.blocks.filter((b) => b.type !== 'cards-grid'),
      B.cards([
        { icon: '◆', title: '能力一', desc: '能力说明文字' },
        { icon: '◆', title: '能力二', desc: '能力说明文字' },
        { icon: '◆', title: '能力三', desc: '能力说明文字' }
      ])
    ]
    applied = '已在本页加入卡片网格'
  } else if (/图表|chart/.test(ins)) {
    slide.blocks = [...slide.blocks, B.chart('bar', ['Q1', 'Q2', 'Q3', 'Q4'], [40, 65, 80, 92])]
    applied = '已在本页加入柱状图'
  } else if (/图片|示意图|架构图/.test(ins)) {
    slide.blocks = [...slide.blocks, B.image()]
    applied = '已在本页加入图片占位'
  } else if (/左右|并排|两列|分栏并排/.test(ins)) {
    const body = slide.blocks.filter((b) => b.type !== 'heading' && b.type !== 'kicker')
    const targets = body.slice(-2)
    if (targets.length >= 2) {
      slide.blocks = slide.blocks.map((b) => (targets.includes(b) ? { ...b, span: 6 } : b))
      applied = '已将本页最后两个区块设为半宽，左右并排'
    } else {
      slide.blocks = slide.blocks.map((b) =>
        b.type !== 'heading' && b.type !== 'kicker' ? { ...b, span: 6 } : b
      )
      applied = '已将正文区块设为半宽，可再追加一个区块形成左右并排'
    }
  } else if (/半宽|一半/.test(ins)) {
    const idx = slide.blocks.findIndex((b) => b.type !== 'heading' && b.type !== 'kicker')
    if (idx > -1) {
      slide.blocks[idx] = { ...slide.blocks[idx], span: 6 }
      applied = '已将该区块设为半宽'
    } else {
      applied = '未找到可调整的正文区块'
    }
  } else if (
    /字号|字体|大一点|大些|小一点|小些|紧凑一点|紧凑些|字太小|字太大|放大|缩小/.test(ins)
  ) {
    let target = slide.blocks[0]
    const idx = slide.blocks.findIndex((b) => b.type !== 'heading' && b.type !== 'kicker')
    if (idx > -1) target = slide.blocks[idx]
    if (/大一点|大些|字太小|放大|宽松/.test(ins)) {
      target.scale = 1.15
      applied = '已将该区块字号调整为「宽松」'
    } else if (/小一点|小些|紧凑|字太大|缩小/.test(ins)) {
      target.scale = 0.85
      applied = '已将该区块字号调整为「紧凑」'
    } else {
      target.scale = 1
      applied = '已将该区块字号恢复为「标准」'
    }
    slide.blocks = slide.blocks.map((b) => (b === target ? target : b))
  } else if (/全宽|加宽/.test(ins)) {
    slide.blocks = slide.blocks.map((b) => ({ ...b, span: 12 }))
    applied = '已将所有区块恢复为全宽'
  } else if (/精简|简洁|删减|少点/.test(ins)) {
    for (const b of slide.blocks) {
      const items = (b.data as unknown as { items?: unknown[] }).items
      if (
        (b.type === 'bullets' || b.type === 'num-list') &&
        Array.isArray(items) &&
        items.length > 3
      ) {
        ;(b.data as unknown as { items: string[] }).items = items.slice(0, 3) as string[]
      }
    }
    applied = '已精简本页内容至 3 条以内'
  } else if (/口语|通俗|生动/.test(ins)) {
    for (const b of slide.blocks) {
      const items = (b.data as unknown as { items?: unknown[] }).items
      if (b.type === 'bullets' && Array.isArray(items)) {
        ;(b.data as unknown as { items: string[] }).items = items.map(
          (x) => String(x).replace(/。$/, '') + '，效果一目了然'
        )
      }
    }
    applied = '文案风格已调整得更接地气'
  } else {
    slide.blocks = slide.blocks.length ? [...slide.blocks].reverse() : [sampleBlock('paragraph')]
    applied = '已按您的指令重排本页内容'
  }

  // 标题补全（切版式后若没有 heading 则补一个）
  if (!slideHeading(slide) && heading) {
    slide.blocks = [B.heading(heading), ...slide.blocks]
  }
  return { applied, theme }
}
