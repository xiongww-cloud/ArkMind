import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import PresentationBlock from '@/components/presentation/PresentationBlock.vue'

function headingBlock(editingField: string | null) {
  return mount(PresentationBlock, {
    props: {
      block: { type: 'heading', data: { text: '测试标题' }, span: 12, scale: 1 },
      editEnabled: true,
      editingField
    } as never
  })
}

describe('heading render', () => {
  it('static: single h2 with one text', () => {
    const w = headingBlock(null)
    const html = w.html()
    expect(html.match(/测试标题/g)?.length ?? 0).toBe(1)
    expect(w.find('h2').exists()).toBe(true)
    expect(w.find('textarea').exists()).toBe(false)
  })

  it('static: no fallback paragraph duplicate for heading', () => {
    const w = headingBlock(null)
    const paras = w.findAll('p')
    // 只有 h2 渲染标题，不应有兜底 <p class="pb-paragraph"> 重复标题
    expect(w.find('.pb-paragraph').exists()).toBe(false)
    expect(paras.length).toBe(0)
  })
})
