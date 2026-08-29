import { defineStore } from 'pinia'
import type {
  DiscussionMessage,
  FolderNode,
  PresentationFolder,
  PresentationItem,
  Slide
} from '@/types/presentation'
import { LAYOUTS, mkSlide, sampleBlock, validateSlide } from '@/presentation/elements'
import { B, W } from '@/presentation/builders'
import {
  finalizeAiSlide,
  getPresentationAiDriver
} from '@/utils/presentationAi'
import { usePresentationHistoryStore } from '@/stores/presentationHistory'

// ─── 工具 ────────────────────────────────────────────────────────
const uid = () => crypto.randomUUID()
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
const now = () => Date.now()
const hoursAgo = (h: number) => Date.now() - h * 3600_000
const daysAgo = (d: number) => Date.now() - d * 86400_000

const STORAGE_KEY = 'arkmind.presentations.v1'
let hydrated = false

// ─── 缩略图刷新防抖定时器（非响应式，仅内部使用） ─────────────────
const thumbTimers = new Map<string, ReturnType<typeof setTimeout>>()

// ─── 内容素材库（各业务线，重组为区块结构） ─────────────────────────
function smartParkDeck(): Slide[] {
  return [
    mkSlide('cover', [
      W(B.heading('智慧园区综合解决方案'), 7),
      W(B.image(), 5),
      B.paragraph('让园区感知、思考、进化 · 客户交流版')
    ]),
    mkSlide('agenda', [
      B.heading('目录'),
      B.numList([
        '项目背景与需求分析',
        '总体架构设计',
        '核心能力亮点',
        '关键指标对比',
        '标杆案例',
        '合作模式与展望'
      ])
    ]),
    mkSlide('content', [
      B.heading('项目背景与需求分析'),
      B.bullets([
        '传统园区面临能耗高、安防弱、运营成本逐年攀升三大痛点',
        '客户希望统一平台打通门禁、安防、能耗、停车等 12 类子系统',
        '监管要求逐步提高，数据合规与等保三级成为硬性门槛',
        '管理层需要一张图掌握园区运行态势，支撑快速决策'
      ])
    ]),
    mkSlide('content', [
      B.heading('总体架构设计'),
      W(
        B.bullets([
          '端：物联网关 + 边缘节点，支持 30+ 协议接入',
          '边：边缘计算节点就地处理视频结构化与联动控制',
          '云：微服务底座 + 数据中台，弹性扩缩容',
          '用：一图统览、移动巡检、智能工单三大应用入口'
        ]),
        6
      ),
      W(B.image(), 6)
    ]),
    mkSlide('stats', [
      B.heading('关键指标对比'),
      B.kpis([
        { value: '-23%', label: '综合能耗', sub: '年省电费约 86 万' },
        { value: '45s', label: '安防响应', sub: '较改造前提速 94%' },
        { value: '18 人', label: '运维人力', sub: '较改造前 -44%' },
        { value: '100%', label: '子系统打通', sub: '12 类系统全接入' }
      ])
    ]),
    mkSlide('quote', [
      B.quote(
        '上线三个月，园区运营报表从每周手工汇总 2 天变成实时自动生成。',
        '—— 某国家级高新区管委会信息中心主任'
      )
    ]),
    mkSlide('content', [
      B.heading('合作模式与实施路径'),
      B.numList([
        '一期（3 个月）：基础平台 + 安防、门禁先行接入',
        '二期（4 个月）：能耗与停车子系统全量上线',
        '三期（持续）：AI 场景训练与增值服务运营',
        '驻场运维 + 远程专家双保障，SLA 99.9%'
      ])
    ]),
    mkSlide('closing', [B.heading('谢谢观看'), B.paragraph('期待与您携手，共建下一代智慧园区')])
  ]
}

function hospitalDeck(): Slide[] {
  return [
    mkSlide('cover', [
      B.heading('三甲医院信息化改造汇报'),
      B.paragraph('以电子病历评级为核心的信息化升级路径')
    ]),
    mkSlide('agenda', [
      B.heading('汇报要点'),
      B.numList(['现状与评级差距', '改造总体蓝图', '分阶段实施计划', '投资与收益测算'])
    ]),
    mkSlide('content', [
      B.heading('现状与评级差距'),
      W(
        B.bullets([
          '当前电子病历评级 3 级，目标两年内达到 5 级',
          '院内 43 套业务系统烟囱式建设，数据互通率不足 40%',
          '门诊高峰期 HIS 响应超时投诉月均 27 起'
        ]),
        7
      ),
      W(
        B.kpis([
          { value: '5 级', label: '目标评级', sub: '两年内' },
          { value: '40%', label: '数据互通', sub: '当前水平' }
        ]),
        5
      )
    ]),
    mkSlide('table', [
      B.heading('分阶段实施计划'),
      B.table(
        ['阶段', '周期', '核心任务', '里程碑'],
        [
          ['一期', '6 个月', '集成平台 + 主数据治理', '互联互通四级甲等'],
          ['二期', '8 个月', '电子病历升级 + CDSS', '电子病历 5 级'],
          ['三期', '6 个月', '智慧服务 + 互联网医院', '智慧服务 3 级']
        ]
      )
    ]),
    mkSlide('quote', [B.quote('信息化不是买设备，而是重塑医疗服务流程。', '—— 项目联合工作组')]),
    mkSlide('closing', [B.heading('感谢聆听'), B.paragraph('欢迎就实施细节深入交流')])
  ]
}

function trainingDeck(): Slide[] {
  return [
    mkSlide('cover', [
      B.heading('新员工产品知识培训'),
      B.paragraph('产品体系 · 核心参数 · 常见问题速查')
    ]),
    mkSlide('content', [
      B.heading('产品体系总览'),
      B.cards([
        { icon: '≡', title: '网络系列', desc: '交换机 / 路由器 / 无线 AP 三条主线' },
        { icon: '▲', title: '安全系列', desc: '防火墙、行为管理、零信任网关' },
        { icon: '▣', title: '智算系列', desc: 'AI 服务器与训推一体机' }
      ])
    ]),
    mkSlide('table', [
      B.heading('主力型号速记卡'),
      B.table(
        ['型号', '定位', '关键参数', '典型场景'],
        [
          ['S6730-H24', '汇聚交换', '24 口万兆', '园区汇聚层'],
          ['AR6140', '出口路由', '4GE+2SFP', '中小企业出口'],
          ['AirEngine 5760', 'Wi-Fi 6 AP', '2.97Gbps', '高密办公']
        ]
      )
    ]),
    mkSlide('quote', [
      B.quote('先懂业务，再谈参数 —— 客户买的从来不是设备本身。', '—— 售前团队内部培训金句')
    ]),
    mkSlide('closing', [B.heading('培训结束'), B.paragraph('考核链接将在企业微信同步发放')])
  ]
}

function networkDeck(): Slide[] {
  return [
    mkSlide('cover', [
      B.heading('数据中心网络产品介绍'),
      B.paragraph('高性能 · 低时延 · 智能运维')
    ]),
    mkSlide('content', [
      B.heading('为什么选择我们的数据中心网络'),
      W(
        B.bullets([
          '单芯片 25.6T 转发能力，支撑 400G 平滑演进',
          '秒级故障收敛，丢包率低于 0.0001%',
          '内置网络数字地图，故障定位从小时级到分钟级'
        ]),
        6
      ),
      W(B.chart('bar', ['P99', '平均'], [28, 16]), 6)
    ]),
    mkSlide('table', [
      B.heading('产品族谱'),
      B.table(
        ['系列', '盒式/框式', '端口密度', '推荐场景'],
        [
          ['CE8850', '盒式', '48×25G + 8×100G', '服务器接入'],
          ['CE16800', '框式', '最大 576×100G', '核心/汇聚'],
          ['DCN 控制器', '软件', '—', 'SDN 集中管控']
        ]
      )
    ]),
    mkSlide('closing', [B.heading('谢谢'), B.paragraph('如需 POC 测试方案请随时联系')])
  ]
}

function medicalDeviceDeck(): Slide[] {
  return [
    mkSlide('cover', [B.heading('智慧医疗设备接入平台'), B.paragraph('设备全生命周期数字化管理')]),
    mkSlide('content', [
      B.heading('平台核心价值'),
      B.bullets([
        '接入 200+ 品牌医疗设备，采集率行业领先',
        '设备利用率可视化，采购决策有据可依',
        '预防性维护提醒，停机时间减少 60%'
      ])
    ]),
    mkSlide('quote', [B.quote('每台设备都在说话，我们负责听懂它。', '—— 产品设计理念')]),
    mkSlide('closing', [B.heading('谢谢观看'), B.paragraph('期待为贵院设备管理注入数字动力')])
  ]
}

// ─── Store ───────────────────────────────────────────────────────
export const usePresentationsStore = defineStore('presentations', {
  state: () => ({
    progressMsg: '',
    folders: <PresentationFolder[]>[
      { id: 'f-client', name: '客户演示', parentId: null, sortOrder: 0 },
      { id: 'f-medical', name: '医疗行业', parentId: 'f-client', sortOrder: 0 },
      { id: 'f-training', name: '内部培训', parentId: null, sortOrder: 1 },
      { id: 'f-product', name: '产品介绍', parentId: null, sortOrder: 2 }
    ],
    presentations: <PresentationItem[]>[
      {
        id: 'p1',
        title: '智慧园区综合解决方案 · 客户交流版',
        theme: 'default',
        folderId: 'f-client',
        status: 'generated',
        generating: false,
        discussion: [],
        slides: smartParkDeck(),
        createdAt: daysAgo(6),
        updatedAt: hoursAgo(5)
      },
      {
        id: 'p2',
        title: '某三甲医院信息化改造汇报',
        theme: 'brand',
        folderId: 'f-medical',
        status: 'generated',
        generating: false,
        discussion: [],
        slides: hospitalDeck(),
        createdAt: daysAgo(9),
        updatedAt: daysAgo(1)
      },
      {
        id: 'p3',
        title: '新员工产品知识培训',
        theme: 'dark',
        folderId: 'f-training',
        status: 'generated',
        generating: false,
        discussion: [],
        slides: trainingDeck(),
        createdAt: daysAgo(14),
        updatedAt: daysAgo(2)
      },
      {
        id: 'p4',
        title: '数据中心网络产品介绍',
        theme: 'default',
        folderId: 'f-product',
        status: 'generated',
        generating: false,
        discussion: [],
        slides: networkDeck(),
        createdAt: daysAgo(3),
        updatedAt: hoursAgo(20)
      },
      {
        id: 'p5',
        title: '政务云方案初步沟通',
        theme: 'default',
        folderId: null,
        status: 'discussing',
        generating: false,
        discussion: [
          {
            id: uid(),
            role: 'user',
            content: '帮我准备一份政务云方案的演示材料，对象是区级政府的评审专家。',
            timestamp: hoursAgo(26)
          },
          {
            id: uid(),
            role: 'assistant',
            content:
              '好的，面向政府评审专家，材料需要突出**安全合规**、**国产化适配**和**运维保障**三条主线。\n\n我的初步建议结构如下：\n\n1. 封面\n2. 政策背景与合规要求\n3. 总体架构（信创栈）\n4. 安全体系建设\n5. 运维服务体系\n6. 同类项目案例\n7. 结束页\n\n您看是否需要调整？另外方便告诉我大概的页数预期吗？',
            timestamp: hoursAgo(26)
          },
          {
            id: uid(),
            role: 'user',
            content: '差不多 10 页以内，把等保三级的要求单独列一页。',
            timestamp: hoursAgo(25)
          },
          {
            id: uid(),
            role: 'assistant',
            content:
              '没问题。我会把**等保三级建设要求**单独成页，覆盖物理、网络、主机、应用、数据五个层面的达标设计。\n\n还需要确认一点：案例部分优先使用本省的政务项目，还是全国范围的标杆案例都可以？确认后随时告诉我"开始生成"。',
            timestamp: hoursAgo(25)
          }
        ],
        slides: [],
        createdAt: daysAgo(2),
        updatedAt: hoursAgo(25)
      },
      {
        id: 'p6',
        title: '年度技术分享大纲',
        theme: 'dark',
        folderId: 'f-training',
        status: 'discussing',
        generating: false,
        discussion: [],
        slides: [],
        createdAt: daysAgo(1),
        updatedAt: daysAgo(1)
      },
      {
        id: 'p7',
        title: '智慧医疗设备接入平台介绍',
        theme: 'default',
        folderId: 'f-medical',
        status: 'generated',
        generating: false,
        discussion: [],
        slides: medicalDeviceDeck(),
        createdAt: daysAgo(20),
        updatedAt: daysAgo(8)
      },
      {
        id: 'p8',
        title: '客户成功案例集锦',
        theme: 'brand',
        folderId: 'f-client',
        status: 'discussing',
        generating: false,
        discussion: [
          {
            id: uid(),
            role: 'user',
            content: '我想做一份客户案例集锦的演示，用于展会循环播放。',
            timestamp: daysAgo(4)
          },
          {
            id: uid(),
            role: 'assistant',
            content:
              '展会循环播放的场景比较特殊：**文字要少、节奏要快、视觉冲击力要强**。\n\n我建议：\n\n- 每个案例压缩到 2 页以内\n- 大字号突出成果数字\n- 整体采用深色主题，在展厅大屏上更显质感\n\n您想收录哪些行业的案例？',
            timestamp: daysAgo(4)
          }
        ],
        slides: [],
        createdAt: daysAgo(4),
        updatedAt: daysAgo(4)
      }
    ]
  }),

  getters: {
    /** 目录树（递归构建层级） */
    folderTree(state): FolderNode[] {
      const build = (parentId: string | null): FolderNode[] =>
        state.folders
          .filter((f) => (f.parentId ?? null) === parentId)
          .sort((a, b) => a.sortOrder - b.sortOrder)
          .map((f) => ({ ...f, children: build(f.id) }))
      return build(null)
    },
    countInFolder(state) {
      return (folderId: string | null | 'all'): number =>
        folderId === 'all'
          ? state.presentations.length
          : state.presentations.filter((p) => p.folderId === folderId).length
    },
    listInFolder(state) {
      return (folderId: string | null | 'all'): PresentationItem[] =>
        state.presentations
          .filter((p) => (folderId === 'all' ? true : p.folderId === folderId))
          .sort((a, b) => b.updatedAt - a.updatedAt)
    },
    getById(state) {
      return (id: string): PresentationItem | undefined =>
        state.presentations.find((p) => p.id === id)
    }
  },

  actions: {
    touch(p: PresentationItem) {
      p.updatedAt = now()
      this.persist()
    },

    // ── 持久化（mock 阶段落 localStorage，为 SQLite 打样） ──
    hydrate() {
      if (hydrated) return
      hydrated = true
      try {
        const raw = localStorage.getItem(STORAGE_KEY)
        if (!raw) return
        const data = JSON.parse(raw)
        if (Array.isArray(data.folders)) this.folders = data.folders
        if (Array.isArray(data.presentations)) {
          this.presentations = data.presentations.map((p: PresentationItem) => ({
            ...p,
            slides: Array.isArray(p.slides) ? p.slides.map((s) => validateSlide(s)) : []
          }))
        }
      } catch (e) {
        console.warn('[presentations] hydrate failed', e)
      }
    },
    persist() {
      try {
        localStorage.setItem(
          STORAGE_KEY,
          JSON.stringify({ folders: this.folders, presentations: this.presentations })
        )
      } catch (e) {
        console.warn('[presentations] persist failed', e)
      }
    },

    // ── 统一的 Deck 修改入口（历史记录 + 落盘） ──
    editDeck(pid: string, label: string, mutate: () => void) {
      const p = this.getById(pid)
      if (!p) return
      const hist = usePresentationHistoryStore()
      const key = `op:${pid}:${uid()}`
      hist.beginSession(key, pid)
      mutate()
      hist.commit(key, label, pid)
      this.touch(p)
    },

    // ── 材料 CRUD ──
    createPresentation(title: string, folderId: string | null): string {
      const id = uid()
      this.presentations.unshift({
        id,
        title,
        theme: 'default',
        folderId,
        status: 'discussing',
        generating: false,
        discussion: [
          {
            id: uid(),
            role: 'assistant',
            content:
              '您好！我是演示材料策划助手 🎯\n\n在正式制作之前，我们先对齐几个关键问题：\n\n1. **这份材料的用途**是什么？（客户交流 / 内部汇报 / 展会展示…）\n2. **听众是谁**？决策层、技术人员还是混合场景？\n3. 有没有**必须涵盖的内容**，比如特定产品、项目或聊天记录里的结论？\n\n您可以随意描述，也可以直接让我先生成一版大纲供讨论。',
            timestamp: now()
          }
        ],
        slides: [],
        createdAt: now(),
        updatedAt: now()
      })
      this.persist()
      return id
    },
    renamePresentation(id: string, title: string) {
      const p = this.getById(id)
      if (!p || !title.trim()) return
      p.title = title.trim()
      this.touch(p)
    },
    deletePresentation(id: string) {
      this.presentations = this.presentations.filter((p) => p.id !== id)
      this.persist()
    },
    movePresentation(id: string, folderId: string | null) {
      const p = this.getById(id)
      if (!p) return
      p.folderId = folderId
      this.touch(p)
    },
    setTheme(id: string, theme: string) {
      this.editDeck(id, '切换主题', () => {
        const p = this.getById(id)
        if (p) p.theme = theme
      })
    },

    // ── 目录管理 ──
    addFolder(name: string, parentId: string | null): string {
      const id = uid()
      const siblings = this.folders.filter((f) => f.parentId === parentId)
      this.folders.push({ id, name, parentId, sortOrder: siblings.length })
      this.persist()
      return id
    },
    renameFolder(id: string, name: string) {
      const f = this.folders.find((x) => x.id === id)
      if (!f || !name.trim()) return
      f.name = name.trim()
      this.persist()
    },
    deleteFolder(id: string) {
      // 子目录上提一级；目录内材料归入未分类
      const parentOfDeleted = this.folders.find((x) => x.id === id)?.parentId ?? null
      for (const f of this.folders) {
        if (f.parentId === id) f.parentId = parentOfDeleted
      }
      this.folders = this.folders.filter((f) => f.id !== id)
      for (const p of this.presentations) {
        if (p.folderId === id) p.folderId = null
      }
      this.persist()
    },

    // ── 策划对话 ──
    pushMessage(id: string, msg: DiscussionMessage) {
      const p = this.getById(id)
      if (!p) return
      p.discussion.push(msg)
      this.touch(p)
    },
    patchMessage(id: string, msgId: string, patch: Partial<DiscussionMessage>) {
      const p = this.getById(id)
      const m = p?.discussion.find((x) => x.id === msgId)
      if (!m) return
      Object.assign(m, patch)
    },

    /** 将工具生成的单页 HTML 应用到演示材料 */
    applyPageHtml(p: PresentationItem, page: number, html: string) {
      const hist = usePresentationHistoryStore()
      const key = `agent:page:${p.id}:${uid()}`
      hist.beginSession(key, p.id)

      if (!p.htmlContent) {
        // 首次生成：构建完整 HTML
        p.htmlContent = buildFullHtml([html], '', p.title)
      } else {
        // 替换或追加页面
        const slides = parseSlides(p.htmlContent)
        if (page >= 1 && page <= slides.length) {
          slides[page - 1].html = html
          p.htmlContent = buildFullHtml(
            slides.map((s) => s.html),
            '',
            p.title
          )
        } else {
          p.htmlContent = buildFullHtml(
            [...slides.map((s) => s.html), html],
            '',
            p.title
          )
        }
      }

      // 更新 slides 缓存
      p.slides = parseSlides(p.htmlContent).map((s, i) => ({
        id: `slide-${i}`,
        layout: 'content',
        blocks: [],
        note: s.title
      }))
      if (p.status !== 'generated') p.status = 'generated'

      hist.commit(key, `AI ${page <= (p.slides?.length ?? 0) ? '修改' : '生成'}第${page}页`, p.id)
      this.touch(p)

      // 异步生成缩略图（不阻塞）
      this.refreshThumbnails(p.id)
    },

    /** 异步刷新缩略图（带防抖，避免多页生成时频繁重建） */
    async refreshThumbnails(pid: string) {
      if (thumbTimers.has(pid)) clearTimeout(thumbTimers.get(pid))
      thumbTimers.set(
        pid,
        setTimeout(async () => {
          thumbTimers.delete(pid)
          const p = this.getById(pid)
          if (!p?.htmlContent) return
          try {
            const { generateThumbnails } = await import('@/utils/htmlSlides')
            p.thumbnails = await generateThumbnails(p.htmlContent)
          } catch (e) {
            console.warn('[refreshThumbnails] failed:', e)
          }
        }, 300)
      )
    },

    // ── 全量生成（真实 AI 产出 Deck JSON；整体记为一步历史） ──
    async startGeneration(pid: string) {
      const p = this.getById(pid)
      if (!p || p.generating) return
      p.generating = true
      const hist = usePresentationHistoryStore()
      const key = `gen:${pid}:${uid()}`
      hist.beginSession(key, pid)
      try {
        const userMsgs = p.discussion.filter((m) => m.role === 'user').map((m) => m.content)
        const plan = [p.title, ...userMsgs.slice(-3)].join('\n')
        const deck = await getPresentationAiDriver().generateDeck(plan, '')
        p.slides = []
        await sleep(600)
        for (const s of deck) {
          p.slides.push(s)
          await sleep(380)
        }
        p.status = 'generated'
        p.generating = false
        hist.commit(key, '生成演示材料', pid)
        this.touch(p)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: `演示材料已完成，共 ${p.slides.length} 页 ✨\n\n每页均由元素库中的版式与区块组合而成。可在右侧「元素」面板或双击画布继续精修。`,
          timestamp: now()
        })
      } catch (e) {
        p.status = 'discussing'
        p.generating = false
        hist.commit(key, '生成演示材料', pid)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: `生成失败：${e instanceof Error ? e.message : String(e)}`,
          timestamp: now()
        })
      }
    },

    /** 取当前页标题文本（供精修/重组保留） */
    slideHeading(slide: Slide): string {
      return (
        ((
          slide.blocks.find((b) => b.type === 'heading')?.data as unknown as Record<string, unknown>
        )?.text as string) || ''
      )
    },

    // ── 按页精修（真实 AI 产出新 slide，整体一步可撤销） ──
    async refineSlide(pid: string, index: number, instruction: string, doneText?: string) {
      const p = this.getById(pid)
      if (!p || !p.slides[index]) return
      await sleep(300)
      const hist = usePresentationHistoryStore()
      const key = `ai:${pid}:${index}:${uid()}`
      hist.beginSession(key, pid)
      try {
        const slide = p.slides[index]
        const next = await getPresentationAiDriver().refineSlide(
          JSON.parse(JSON.stringify(slide)),
          instruction,
          ''
        )
        p.slides[index] = validateSlide(next)
        hist.commit(key, 'AI 精修本页', pid)
        this.touch(p)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: doneText ?? `第 ${index + 1} 页精修完成。`,
          timestamp: now()
        })
      } catch (e) {
        hist.commit(key, 'AI 精修本页', pid)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: `精修失败：${e instanceof Error ? e.message : String(e)}`,
          timestamp: now()
        })
      }
    },

    // ── AI 重组本页（走真实精修：重新设计版式与内容） ──
    async recombineSlide(pid: string, index: number) {
      const p = this.getById(pid)
      if (!p || !p.slides[index]) return
      await this.refineSlide(
        pid,
        index,
        '请重新设计本页版式与内容：选择一种比当前更合适的版式，重构区块，保留标题与核心信息，让页面更有说服力。',
        `AI 已重组第 ${index + 1} 页。`
      )
    },

    // ── AI 整篇修改（真实 AI 返回修改后完整 deck，一步可撤销） ──
    async reviseDeck(pid: string, instruction: string) {
      const p = this.getById(pid)
      if (!p || p.slides.length === 0) return
      console.log('[reviseDeck] pid=%s instruction=%s', pid, instruction.slice(0, 60))
      const hist = usePresentationHistoryStore()
      const key = `rev:${pid}:${uid()}`
      hist.beginSession(key, pid)
      try {
        console.log('[reviseDeck] calling driver.reviseDeck, slides=%d', p.slides.length)
        const deck = await getPresentationAiDriver().reviseDeck(
          JSON.parse(JSON.stringify(p.slides)),
          instruction,
          ''
        )
        console.log('[reviseDeck] success, returned %d slides', deck.length)
        p.slides = deck
        hist.commit(key, 'AI 修改整篇', pid)
        this.touch(p)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: `已按您的要求修改整篇演示材料（共 ${deck.length} 页），未提及的页面尽量保持不变。可用 Ctrl+Z 撤销。`,
          timestamp: now()
        })
      } catch (e) {
        console.error('[reviseDeck] failed:', e)
        hist.commit(key, 'AI 修改整篇', pid)
        this.pushMessage(pid, {
          id: uid(),
          role: 'assistant',
          content: `修改失败：${e instanceof Error ? e.message : String(e)}`,
          timestamp: now()
        })
      }
    },

    // ── 手动装配：应用版式 / 追加区块 / 排序 / 删除（均走历史） ──
    applyLayout(pid: string, index: number, layoutId: string) {
      this.editDeck(pid, '应用版式', () => {
        const p = this.getById(pid)
        const slide = p?.slides[index]
        if (!p || !slide) return
        // 保留版式允许的区块，补一个占位段落保证非空
        const layout = LAYOUTS.find((l) => l.id === layoutId) ?? LAYOUTS[3]
        const kept = slide.blocks.filter((b) => layout.allowedBlocks.includes(b.type))
        if (kept.length === 0) kept.push(sampleBlock('paragraph'))
        p.slides[index] = validateSlide(mkSlide(layout.id, kept, slide.note))
      })
    },
    appendBlock(pid: string, index: number, blockType: string) {
      this.editDeck(pid, '追加区块', () => {
        const p = this.getById(pid)
        const slide = p?.slides[index]
        if (!p || !slide) return
        const layout = LAYOUTS.find((l) => l.id === slide.layout) ?? LAYOUTS[3]
        if (!layout.allowedBlocks.includes(blockType)) {
          // 当前版式不允许 → 切到 content 版式再追加
          slide.layout = 'content'
        }
        slide.blocks.push(sampleBlock(blockType))
        p.slides[index] = validateSlide(slide)
      })
    },
    moveBlock(pid: string, index: number, blockIndex: number, dir: -1 | 1) {
      this.editDeck(pid, '移动区块', () => {
        const p = this.getById(pid)
        const slide = p?.slides[index]
        if (!p || !slide) return
        const to = blockIndex + dir
        if (to < 0 || to >= slide.blocks.length) return
        const arr = [...slide.blocks]
        const [b] = arr.splice(blockIndex, 1)
        arr.splice(to, 0, b)
        slide.blocks = arr
      })
    },
    removeBlock(pid: string, index: number, blockIndex: number) {
      this.editDeck(pid, '删除区块', () => {
        const p = this.getById(pid)
        const slide = p?.slides[index]
        if (!p || !slide) return
        slide.blocks.splice(blockIndex, 1)
        if (slide.blocks.length === 0) slide.blocks.push(sampleBlock('paragraph'))
        p.slides[index] = validateSlide(slide)
      })
    },
    setBlockSpan(pid: string, index: number, blockIndex: number, span: number) {
      this.editDeck(pid, '调整区块宽度', () => {
        const p = this.getById(pid)
        const blk = p?.slides[index]?.blocks[blockIndex]
        if (!p || !blk) return
        blk.span = span
      })
    },
    setBlockScale(pid: string, index: number, blockIndex: number, scale: number) {
      this.editDeck(pid, '调整区块字号', () => {
        const p = this.getById(pid)
        const blk = p?.slides[index]?.blocks[blockIndex]
        if (!p || !blk) return
        blk.scale = scale
      })
    },

    // ── 幻灯片级操作（增删/排序/复制，均走历史） ──
    addSlide(pid: string, afterIndex?: number): string | null {
      const p = this.getById(pid)
      if (!p) return null
      const slide = mkSlide('content', [B.heading('新的一页'), B.paragraph('点击右侧编辑内容')])
      this.editDeck(pid, '新增页面', () => {
        p.slides.splice(afterIndex ?? p.slides.length, 0, slide)
      })
      return slide.id
    },
    duplicateSlide(pid: string, index: number): string | null {
      const p = this.getById(pid)
      const src = p?.slides[index]
      if (!p || !src) return null
      const copy = mkSlide(
        src.layout,
        src.blocks.map((b) => ({ ...b, id: uid() })),
        src.note
      )
      this.editDeck(pid, '复制页面', () => {
        p.slides.splice(index + 1, 0, copy)
      })
      return copy.id
    },
    deleteSlide(pid: string, index: number) {
      const p = this.getById(pid)
      if (!p || p.slides.length <= 1) return
      this.editDeck(pid, '删除页面', () => {
        p.slides.splice(index, 1)
      })
    },
    moveSlide(pid: string, from: number, to: number) {
      const p = this.getById(pid)
      if (!p || from === to) return
      this.editDeck(pid, '移动页面', () => {
        const arr = [...p.slides]
        const [s] = arr.splice(from, 1)
        arr.splice(to, 0, s)
        p.slides = arr
      })
    }
  }
})
