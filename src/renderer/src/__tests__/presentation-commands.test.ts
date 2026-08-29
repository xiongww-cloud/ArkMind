import { describe, it, expect, beforeEach } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { usePresentationsStore } from '@/stores/presentations'
import { usePresentationHistoryStore } from '@/stores/presentationHistory'
import { usePresentationEdit } from '@/composables/usePresentationEdit'
import { mkSlide, sampleBlock, uid, validateSlide } from '@/presentation/elements'
import { setPresentationAiDriver } from '@/utils/presentationAi'
import { applyRefineRules } from '@/presentation/refineRules'

/** 测试用 mock AI 驱动：复用关键词精修规则 */
function useMockAiDriver() {
  setPresentationAiDriver({
    generateDeck: async () => [],
    reviseDeck: async (deck) => deck,
    refineSlide: async (slide, instruction) => {
      const clone = validateSlide(JSON.parse(JSON.stringify(slide)))
      applyRefineRules(clone, instruction)
      return clone
    }
  })
}

function seed() {
  setActivePinia(createPinia())
  const store = usePresentationsStore()
  store.presentations = [
    {
      id: 'd1',
      title: '演示',
      theme: 'default',
      folderId: null,
      status: 'generated',
      discussion: [],
      slides: [mkSlide('content', [sampleBlock('heading'), sampleBlock('bullets')])],
      createdAt: 0,
      updatedAt: 0
    }
  ]
  return store
}

const blk = (pid: string, idx: number, bi: number) =>
  usePresentationsStore().getById(pid)!.slides[idx].blocks[bi]

describe('presentation command/undo', () => {
  beforeEach(() => {
    localStorage.clear()
    useMockAiDriver()
  })

  it('setBlockSpan is undoable and redoable', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    store.setBlockSpan('d1', 0, 0, 6)
    expect(blk('d1', 0, 0).span).toBe(6)
    history.undo()
    expect(blk('d1', 0, 0).span).toBe(12)
    history.redo()
    expect(blk('d1', 0, 0).span).toBe(6)
  })

  it('removeBlock is undoable', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    const before = store.getById('d1')!.slides[0].blocks.length
    store.removeBlock('d1', 0, 1)
    expect(store.getById('d1')!.slides[0].blocks.length).toBeLessThan(before)
    history.undo()
    expect(store.getById('d1')!.slides[0].blocks.length).toBe(before)
  })

  it('typing bursts merge into a single undo step', () => {
    seed()
    const history = usePresentationHistoryStore()
    const edit = usePresentationEdit(
      () => 'd1',
      () => 0
    )
    const heading = blk('d1', 0, 0)
    const data = heading.data as unknown as Record<string, unknown>
    edit.run(`${heading.id}:text`, '编辑文本', () => {
      data.text = '页面标题A'
    })
    edit.run(`${heading.id}:text`, '编辑文本', () => {
      data.text = '页面标题AB'
    })
    edit.run(`${heading.id}:text`, '编辑文本', () => {
      data.text = '页面标题ABC'
    })
    expect(history.undoStack.length).toBe(1)
    history.undo()
    expect((blk('d1', 0, 0).data as unknown as Record<string, unknown>).text).toBe('页面标题')
  })

  it('applyLayout is undoable', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    store.applyLayout('d1', 0, 'stats')
    expect(store.getById('d1')!.slides[0].layout).toBe('stats')
    history.undo()
    expect(store.getById('d1')!.slides[0].layout).toBe('content')
  })

  it('theme switch is undoable', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    store.setTheme('d1', 'dark')
    expect(store.getById('d1')!.theme).toBe('dark')
    history.undo()
    expect(store.getById('d1')!.theme).toBe('default')
  })

  it('refineSlide is a single undo step (AI 操作可撤销)', async () => {
    const { vi } = await import('vitest')
    vi.useFakeTimers()
    try {
      const store = seed()
      const history = usePresentationHistoryStore()
      const promise = store.refineSlide('d1', 0, '改成表格')
      await vi.advanceTimersByTimeAsync(1200)
      await promise
      expect(store.getById('d1')!.slides[0].layout).toBe('table')
      expect(history.undoStack.length).toBe(1)
      history.undo()
      expect(store.getById('d1')!.slides[0].layout).toBe('content')
    } finally {
      vi.useRealTimers()
    }
  })

  it('duplicateSlide and deleteSlide are undoable', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    const n = store.getById('d1')!.slides.length
    store.duplicateSlide('d1', 0)
    expect(store.getById('d1')!.slides.length).toBe(n + 1)
    history.undo()
    expect(store.getById('d1')!.slides.length).toBe(n)
    history.redo()
    expect(store.getById('d1')!.slides.length).toBe(n + 1)
    store.deleteSlide('d1', 0)
    expect(store.getById('d1')!.slides.length).toBe(n)
    history.undo()
    expect(store.getById('d1')!.slides.length).toBe(n + 1)
  })

  it('refineSlide with AI unavailable reports error and records no undo step', async () => {
    // 模拟真实驱动在浏览器/未配置时的行为
    setPresentationAiDriver({
      generateDeck: async () => {
        throw new Error('请先配置 Chat API 密钥')
      },
      reviseDeck: async () => {
        throw new Error('请先配置 Chat API 密钥')
      },
      refineSlide: async () => {
        throw new Error('请先配置 Chat API 密钥')
      }
    })
    const store = seed()
    const history = usePresentationHistoryStore()
    const layoutBefore = store.getById('d1')!.slides[0].layout
    await store.refineSlide('d1', 0, '改成表格')
    expect(store.getById('d1')!.slides[0].layout).toBe(layoutBefore)
    expect(history.undoStack.length).toBe(0)
    const last = store.getById('d1')!.discussion[store.getById('d1')!.discussion.length - 1]
    expect(last.role).toBe('assistant')
    expect(last.content).toContain('精修失败')
  })

  it('undo across different presentations is isolated', () => {
    const store = seed()
    const history = usePresentationHistoryStore()
    store.presentations.push({
      id: 'd2',
      title: '二',
      theme: 'default',
      folderId: null,
      status: 'generated',
      discussion: [],
      slides: [mkSlide('content', [sampleBlock('heading')])],
      createdAt: 0,
      updatedAt: 0
    })
    store.setBlockSpan('d1', 0, 0, 6)
    store.setBlockSpan('d2', 0, 0, 4)
    history.undo()
    expect(blk('d2', 0, 0).span).toBe(12)
    expect(blk('d1', 0, 0).span).toBe(6)
    history.undo()
    expect(blk('d1', 0, 0).span).toBe(12)
  })

  it('blocks always carry an id', () => {
    const store = seed()
    expect(uid).toBeTypeOf('function')
    expect(
      store.getById('d1')!.slides[0].blocks.every((b) => typeof b.id === 'string' && b.id)
    ).toBe(true)
  })
})
