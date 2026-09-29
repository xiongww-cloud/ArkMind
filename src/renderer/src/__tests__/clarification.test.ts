import { describe, it, expect } from 'vitest'
import { parseClarificationMetadata, buildClarificationReply, mergeClarificationAnswer } from '../composables/useAgentChat'
import type { ClarificationData } from '../composables/useAgentChat'

const data: ClarificationData = {
  reason: '为了精准编制超融合参数',
  questions: [
    { question_id: 'q1', prompt: '业务场景偏好？', type: 'single_choice', options: ['医院核心HIS/PACS', '通用办公虚拟化'] },
    { question_id: 'q2', prompt: '信创与架构约束？', type: 'multiple_choice', options: ['纯国产信创', 'X86'] },
    { question_id: 'q3', prompt: '部署模式', type: 'text', options: [] },
  ],
}

describe('parseClarificationMetadata', () => {
  it('从 metadata JSON 恢复结构化追问', () => {
    const meta = JSON.stringify({ documents: [{ file_id: 'F1' }], clarification: data })
    const got = parseClarificationMetadata(meta)
    expect(got).toBeTruthy()
    expect(got!.reason).toBe('为了精准编制超融合参数')
    expect(got!.questions).toHaveLength(3)
    expect(got!.questions[1].type).toBe('multiple_choice')
  })

  it('无 clarification 键 / 非法 JSON / 空 questions → undefined', () => {
    expect(parseClarificationMetadata(JSON.stringify({ documents: [] }))).toBeUndefined()
    expect(parseClarificationMetadata('not-json')).toBeUndefined()
    expect(parseClarificationMetadata(undefined)).toBeUndefined()
    expect(parseClarificationMetadata(JSON.stringify({ clarification: { reason: 'r', questions: [] } }))).toBeUndefined()
  })

  it('剔除空 prompt 问题、未知 type 降级 text', () => {
    const meta = JSON.stringify({
      clarification: {
        reason: 'r',
        questions: [
          { question_id: 'a', prompt: '', type: 'text', options: [] },
          { question_id: 'b', prompt: 'B？', type: 'weird', options: null },
        ],
      },
    })
    const got = parseClarificationMetadata(meta)!
    expect(got.questions).toHaveLength(1)
    expect(got.questions[0].question_id).toBe('b')
    expect(got.questions[0].type).toBe('text')
    expect(got.questions[0].options).toEqual([])
  })
})

describe('buildClarificationReply', () => {
  it('合成点选答案为格式化用户消息', () => {
    const text = buildClarificationReply(data, {
      q1: '医院核心HIS/PACS',
      q2: ['纯国产信创', 'X86'],
      q3: '  ',
    })
    expect(text.startsWith('已确认补充信息：')).toBe(true)
    expect(text).toContain('1.业务场景偏好：医院核心HIS/PACS')
    expect(text).toContain('2.信创与架构约束：纯国产信创、X86')
    expect(text).not.toContain('3.')
    expect(text.endsWith('请继续执行。')).toBe(true)
  })

  it('全部未答 → 默认假设话术（与 prompt 默认假设原则呼应）', () => {
    const text = buildClarificationReply(data, {})
    expect(text).toContain('默认假设')
  })

  it('问号尾巴被清理', () => {
    const text = buildClarificationReply(data, { q3: '一体机交付' })
    expect(text).toContain('3.部署模式：一体机交付')
    expect(text).not.toContain('部署模式？')
  })
})

describe('mergeClarificationAnswer', () => {
  const single = data.questions[0]
  const multi = data.questions[1]
  const free = data.questions[2]

  it('单选：自定义覆盖已选项；空自定义回退选择', () => {
    expect(mergeClarificationAnswer(single, '医院核心HIS/PACS', ' 混合场景 ')).toBe('混合场景')
    expect(mergeClarificationAnswer(single, '医院核心HIS/PACS', '  ')).toBe('医院核心HIS/PACS')
    expect(mergeClarificationAnswer(single, '', '')).toBe('')
  })

  it('多选：自定义作为附加项并入；空自定义保持原列表', () => {
    expect(mergeClarificationAnswer(multi, ['纯国产信创'], '含少量X86存量'))
      .toEqual(['纯国产信创', '含少量X86存量'])
    expect(mergeClarificationAnswer(multi, [], '  ')).toEqual([])
  })

  it('text 题：自定义即答案（与点选通道兼容）', () => {
    expect(mergeClarificationAnswer(free, undefined, '原厂一体机')).toBe('原厂一体机')
    expect(mergeClarificationAnswer(free, '旧值', '')).toBe('旧值')
  })

  it('自定义参与合成答复文本', () => {
    const merged: Record<string, string | string[]> = {
      q1: mergeClarificationAnswer(single, '医院核心HIS/PACS', 'HIS+影像双活'),
      q2: mergeClarificationAnswer(multi, ['纯国产信创'], '保留X86出口'),
    }
    const text = buildClarificationReply(data, merged)
    expect(text).toContain('1.业务场景偏好：HIS+影像双活')
    expect(text).toContain('2.信创与架构约束：纯国产信创、保留X86出口')
  })
})
