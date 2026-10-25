import { describe, it, expect } from 'vitest'
import {
  parseParams,
  serializeParams,
  normalizeParams,
  lintParams,
  fixParams,
  paramsEquivalent,
  stripControlContent,
  PROOF_KEYWORDS
} from '../utils/paramText'

describe('parseParams', () => {
  it('解析规范格式', () => {
    const rows = parseParams('1、吞吐量：≥1Tbps\n2、端口：48×GE')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toMatchObject({ num: 1, mark: '', indicator: '吞吐量', requirement: '≥1Tbps' })
    expect(rows[1]).toMatchObject({ num: 2, indicator: '端口', requirement: '48×GE' })
  })

  it('兼容各种旧序号写法', () => {
    const rows = parseParams('1. 冷备：1+1\n2) 冗余：双机\n（3）电源：双路\n4．散热：风冷\n5。质保：3年')
    expect(rows.map((r) => r.num)).toEqual([1, 2, 3, 4, 5])
    expect(rows[0].indicator).toBe('冷备')
    expect(rows[2].indicator).toBe('电源')
  })

  it('保留 ▲/★ 标记', () => {
    const rows = parseParams('▲1、核心指标：必答\n★2、废标项：无')
    expect(rows[0].mark).toBe('▲')
    expect(rows[0].indicator).toBe('核心指标')
    expect(rows[1].mark).toBe('★')
  })

  it('无序号行保留为 raw 行不丢数据', () => {
    const rows = parseParams('这是一行没有序号的内容\n1、正常：参数')
    expect(rows[0].raw).toBe('这是一行没有序号的内容')
    expect(rows[0].num).toBeNull()
    expect(rows[1].indicator).toBe('正常')
  })

  it('半角冒号切分并标记 halfColon', () => {
    const rows = parseParams('1、端口: 48个')
    expect(rows[0].indicator).toBe('端口')
    expect(rows[0].requirement).toBe('48个')
    expect(rows[0].halfColon).toBe(true)
  })

  it('无冒号整行作为 raw', () => {
    const rows = parseParams('1、整行内容没有冒号')
    expect(rows[0].raw).toBe('整行内容没有冒号')
    expect(rows[0].halfColon).toBe(false)
  })

  it('空行丢弃', () => {
    expect(parseParams('1、a：b\n\n\n2、c：d\n')).toHaveLength(2)
    expect(parseParams('')).toHaveLength(0)
    expect(parseParams('   \n  ')).toHaveLength(0)
  })
})

describe('serializeParams / normalizeParams', () => {
  it('序列化为规范格式并连续编号（保持行序，不按旧序号重排）', () => {
    const rows = parseParams('3、b：2\n1、a：1')
    expect(serializeParams(rows)).toBe('1、b：2\n2、a：1')
  })

  it('归一分隔符与冒号', () => {
    expect(normalizeParams('1. a: 1\n2）b：2')).toBe('1、a：1\n2、b：2')
  })

  it('保留标记前缀', () => {
    expect(normalizeParams('▲1. a: 1')).toBe('▲1、a：1')
  })

  it('幂等：normalize 两次等于一次', () => {
    const t = '1. a: 1\n2）b；c\n3、裸行'
    expect(normalizeParams(normalizeParams(t))).toBe(normalizeParams(t))
  })

  it('无序号行补序号', () => {
    expect(normalizeParams('裸行内容')).toBe('1、裸行内容')
  })
})

describe('paramsEquivalent', () => {
  it('仅格式差异视为一致', () => {
    expect(paramsEquivalent('1、a：1\n2、b：2', '1. a: 1\n2）b：2')).toBe(true)
  })
  it('内容差异视为不一致', () => {
    expect(paramsEquivalent('1、a：1', '1、a：2')).toBe(false)
  })
  it('空文本等价', () => {
    expect(paramsEquivalent('', '')).toBe(true)
    expect(paramsEquivalent('', '1、a：1')).toBe(false)
  })
})

describe('lintParams', () => {
  it('规范文本无问题', () => {
    expect(lintParams('1、吞吐量：≥1Tbps；\n2、端口：48个GE。')).toEqual([])
  })

  it('序号乱序 / 重号', () => {
    const issues = lintParams('2、a：1\n1、b：2')
    expect(issues.some((i) => i.rule === 'seq')).toBe(true)
    expect(lintParams('1、a：1\n1、b：2').some((i) => i.rule === 'seq')).toBe(true)
  })

  it('分隔符不规范', () => {
    const issues = lintParams('1. a：1\n2）b：2')
    expect(issues.filter((i) => i.rule === 'sep')).toHaveLength(2)
  })

  it('缺序号', () => {
    expect(lintParams('裸行\n1、a：1').some((i) => i.rule === 'sep' && i.line === 1)).toBe(true)
  })

  it('半角冒号', () => {
    const issues = lintParams('1、a: 1')
    expect(issues.filter((i) => i.rule === 'colon')).toHaveLength(1)
  })

  it('行尾空格与连续空格', () => {
    const issues = lintParams('1、a：1  \n2、b：  x')
    expect(issues.some((i) => i.rule === 'space')).toBe(true)
  })

  it('全半角逗号/分号混用', () => {
    const issues = lintParams('1、a：x,y，z\n2、b：p;q；r')
    expect(issues.filter((i) => i.rule === 'punct')).toHaveLength(2)
  })

  it('纯全角不报混用', () => {
    expect(lintParams('1、a：x，y\n2、b：p；r').filter((i) => i.rule === 'punct')).toHaveLength(0)
  })

  it('空文本无问题', () => {
    expect(lintParams('')).toEqual([])
    expect(lintParams('   ')).toEqual([])
  })

  it('行尾标点：非末行缺「；」、末行缺「。」', () => {
    const issues = lintParams('1、a：1\n2、b：2\n3、c：3')
    const ending = issues.filter((i) => i.rule === 'ending')
    expect(ending).toHaveLength(3)
    expect(ending[0].message).toContain('；')
    expect(ending[2].message).toContain('。')
  })

  it('行尾标点规范文本通过', () => {
    expect(lintParams('1、a：1；\n2、b：2。').filter((i) => i.rule === 'ending')).toHaveLength(0)
  })

  it('末行以「；」结尾视为不规范', () => {
    expect(lintParams('1、a：1；\n2、b：2；').filter((i) => i.rule === 'ending')).toHaveLength(1)
  })
})

describe('fixParams', () => {
  it('一次修复全部五类问题（含行尾标点）', () => {
    const messy = '2. a: x,y，z \n4）b：p'
    const { text, fixes } = fixParams(messy)
    expect(text).toBe('1、a：x，y，z；\n2、b：p。')
    expect(fixes.seq).toBe(1)
    expect(fixes.sep).toBe(2)
    expect(fixes.colon).toBe(1)
    expect(fixes.space).toBe(1)
    expect(fixes.punct).toBe(1)
    expect(fixes.ending).toBe(2)
    // 修复后 lint 干净
    expect(lintParams(text)).toEqual([])
  })

  it('逗号按多数派统一（半角居多取半角）+ 行尾补「。」', () => {
    const { text } = fixParams('1、a：x,y,z，w')
    expect(text).toBe('1、a：x,y,z,w。')
  })

  it('已是规范文本时零修复且不变', () => {
    const clean = '1、a：1；\n2、b：2。'
    const { text, fixes } = fixParams(clean)
    expect(text).toBe(clean)
    expect(fixes).toEqual({ seq: 0, sep: 0, colon: 0, space: 0, punct: 0, ending: 0 })
  })

  it('修复幂等', () => {
    const once = fixParams('2. a: 1 \n3、b；c').text
    expect(fixParams(once).text).toBe(once)
  })

  it('空文本原样返回', () => {
    expect(fixParams('')).toEqual({ text: '', fixes: { seq: 0, sep: 0, colon: 0, space: 0, punct: 0, ending: 0 } })
  })
})

describe('stripControlContent', () => {
  it('剥除行首 ★/▲ 符号但保留行', () => {
    const r = stripControlContent('★1、质保：三年\n▲2、端口：48GE')
    expect(r.text).toBe('1、质保：三年；\n2、端口：48GE。')
    expect(r.marksStripped).toBe(2)
    expect(r.rowsRemoved).toBe(0)
  })

  it('扫除内嵌 ★/▲ 字符', () => {
    const r = stripControlContent('1、要求：支持A★功能与▲特性')
    expect(r.text).toBe('1、要求：支持A功能与特性。')
    expect(r.marksStripped).toBe(2)
  })

  it('按标点切段删除证明材料段并修复标点', () => {
    // 含"盖章证明"的段连同前导分隔符一起删除，剩余段重接后行尾补「。」
    const r = stripControlContent('1、质保：原厂三年，提供盖章证明；全国联保')
    expect(r.text).toBe('1、质保：原厂三年，提供盖章证明；全国联保。'.replace('，提供盖章证明；', '，'))
    expect(r.segmentsRemoved).toBe(1)
  })

  it('多个关键词段各自删除，序号重排', () => {
    const r = stripControlContent(
      '1、质保：三年，提供CMA检测报告\n2、服务：7x24，附现场截图\n3、端口：48GE'
    )
    expect(r.rowsRemoved).toBe(0)
    expect(r.segmentsRemoved).toBe(2)
    expect(r.text).not.toContain('报告')
    expect(r.text).not.toContain('截图')
    expect(r.text).toContain('48GE')
    // 重排后仍连续
    expect(r.text).toMatch(/^1、质保/)
    expect(r.text).toMatch(/\n2、服务/)
    expect(r.text).toMatch(/\n3、端口/)
  })

  it('指标名命中关键词整行删除', () => {
    const r = stripControlContent('1、盖章证明：原件\n2、端口：48GE')
    expect(r.rowsRemoved).toBe(1)
    expect(r.text).toBe('1、端口：48GE。')
  })

  it('要求被清空的行整行删除', () => {
    const r = stripControlContent('1、质保：提供盖章证明\n2、端口：48GE')
    expect(r.rowsRemoved).toBe(1)
    expect(r.text).toBe('1、端口：48GE。')
  })

  it('空文本与无控标文本', () => {
    expect(stripControlContent('').text).toBe('')
    const r = stripControlContent('1、端口：48GE')
    expect(r.text).toBe('1、端口：48GE。') // 仅行尾归一
    expect(r.marksStripped + r.segmentsRemoved + r.rowsRemoved).toBe(0)
  })

  it('关键词表恰为四词', () => {
    expect(PROOF_KEYWORDS).toEqual(['截图', '证明', '报告', '盖章'])
  })
})
