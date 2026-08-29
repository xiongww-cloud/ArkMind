<script setup lang="ts">
import { computed } from 'vue'
import type { BlockInstance, TableCell, TableData } from '@/types/presentation'
import DOMPurify from 'dompurify'
import { docToHtml, parseDoc } from '@/presentation/richText'

const props = defineProps<{
  block: BlockInstance
  hero?: boolean
}>()

const d = computed(() => props.block.data as unknown as Record<string, unknown>)

const str = (key: string, def = '') => {
  const v = d.value[key]
  return typeof v === 'string' && v ? v : def
}
const items = (key: string) => (Array.isArray(d.value[key]) ? (d.value[key] as unknown[]) : [])
/** columns 区块：取一侧分栏数据 */
const col = (side: string) =>
  d.value[side] && typeof d.value[side] === 'object'
    ? (d.value[side] as Record<string, unknown>)
    : undefined
const colTitle = (side: string) => String(col(side)?.title ?? '')
/** columns 区块：取一侧条目的嵌套数组 */
const colItems = (side: string) => {
  const v = col(side)?.items
  return Array.isArray(v) ? (v as unknown[]) : []
}

const headingCls = computed(() => (props.hero ? 'pb-heading pb-heading-hero' : 'pb-heading'))

// 图表（柱状/折线）
const chartKind = computed(() => str('kind', 'bar'))
const chartLabels = computed(() => items('labels').map((x) => String(x)))
const chartValues = computed(() => items('values').map((x) => Number(x) || 0))
const chartMax = computed(() => Math.max(...chartValues.value, 1))
const linePoints = computed(() => {
  const n = chartValues.value.length
  if (n < 2) return ''
  const w = 800
  const h = 300
  const step = w / (n - 1)
  return chartValues.value
    .map((v, i) => `${(i * step + 6).toFixed(1)},${(h - (v / chartMax.value) * h + 10).toFixed(1)}`)
    .join(' ')
})

/** 图片尺寸倍率（0.5 ~ 1.2），默认 1 */
const imgSize = computed(() => {
  const v = d.value.size
  return typeof v === 'number' && v > 0 ? v : 1
})

// ─── 富文本段落 ──────────────────────────────────────────────────
const paraHtml = computed(() => {
  const raw = str('text')
  if (!raw) return ''
  return DOMPurify.sanitize(docToHtml(parseDoc(raw)))
})

// ─── 结构化表格（rows[0] 表头 + 合并单元格） ───────────────────────
const tableModel = computed(() => props.block.data as unknown as TableData)
const colStyle = (c: { width?: number }) =>
  c.width && c.width > 0 ? { width: `${c.width}%` } : undefined
const headerCells = computed(() => tableModel.value.rows[0]?.cells ?? [])
/** 按 rowspan 折叠出每行实际渲染的单元格（跳过被上方跨行占据的列） */
const bodyGrid = computed<Array<Array<{ cell: TableCell; rspan: number }>>>(() => {
  const rows = tableModel.value.rows.slice(1)
  const grid: Array<Array<{ cell: TableCell; rspan: number }>> = []
  const pending = new Map<number, number>()
  for (const row of rows) {
    const line: Array<{ cell: TableCell; rspan: number }> = []
    let c = 0
    for (const cell of row.cells) {
      while ((pending.get(c) ?? 0) > 0) c++
      if (c >= 200) break
      const rspan = cell.rowspan && cell.rowspan > 1 ? cell.rowspan : 1
      const colspan = cell.colspan && cell.colspan > 1 ? cell.colspan : 1
      if (rspan > 1) {
        for (let k = 0; k < colspan; k++)
          pending.set(c + k, Math.max(pending.get(c + k) ?? 0, rspan - 1))
      }
      line.push({ cell, rspan })
      c += colspan
    }
    for (const [col, v] of pending) {
      const nv = v - 1
      if (nv <= 0) pending.delete(col)
      else pending.set(col, nv)
    }
    grid.push(line)
  }
  return grid
})
</script>

<template>
  <!-- 眉题 -->
  <template v-if="block.type === 'kicker'">
    <p class="pb-kicker">{{ str('text') }}</p>
  </template>

  <!-- 标题 -->
  <template v-else-if="block.type === 'heading'">
    <h2 :class="headingCls">{{ str('text') }}</h2>
  </template>

  <!-- 段落 -->
  <template v-else-if="block.type === 'paragraph'">
    <!-- eslint-disable vue/no-v-html -->
    <div class="pb-paragraph" v-html="paraHtml"></div>
    <!-- eslint-enable vue/no-v-html -->
  </template>

  <!-- 要点 -->
  <template v-else-if="block.type === 'bullets'">
    <ul class="pb-bullets">
      <li v-for="(it, i) in items('items')" :key="i" class="pb-bullet">
        <span class="inl-plain">{{ String(it) }}</span>
      </li>
    </ul>
  </template>

  <!-- 编号步骤 -->
  <template v-else-if="block.type === 'num-list'">
    <ol class="pb-numlist">
      <li v-for="(it, i) in items('items')" :key="i" class="pb-num-item">
        <span class="pb-num">{{ String(i + 1).padStart(2, '0') }}</span>
        <span class="inl-plain">{{ String(it) }}</span>
      </li>
    </ol>
  </template>

  <!-- 表格（结构化列 + 行；rows[0] 表头，支持合并） -->
  <template v-else-if="block.type === 'table'">
    <table class="pb-table">
      <colgroup v-if="tableModel.columns.length">
        <col v-for="cl in tableModel.columns" :key="cl.id" :style="colStyle(cl)" />
      </colgroup>
      <thead v-if="headerCells.length">
        <tr>
          <th
            v-for="(c, ci) in headerCells"
            :key="ci"
            :colspan="c.colspan && c.colspan > 1 ? c.colspan : undefined"
            :rowspan="c.rowspan && c.rowspan > 1 ? c.rowspan : undefined"
          >
            {{ c.content }}
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(line, ri) in bodyGrid" :key="ri">
          <td
            v-for="(it, ci) in line"
            :key="ci"
            :rowspan="it.rspan > 1 ? it.rspan : undefined"
            :colspan="it.cell.colspan && it.cell.colspan > 1 ? it.cell.colspan : undefined"
            :class="{ strong: ci === 0 }"
          >
            {{ it.cell.content }}
          </td>
        </tr>
      </tbody>
    </table>
  </template>

  <!-- 数据卡 -->
  <template v-else-if="block.type === 'kpi-row'">
    <div class="pb-kpis">
      <div v-for="(it, i) in items('items')" :key="i" class="pb-kpi">
        <span class="pb-kpi-value">{{ String((it as any)?.value ?? '--') }}</span>
        <span class="pb-kpi-label">{{ String((it as any)?.label ?? '') }}</span>
        <span v-if="(it as any)?.sub" class="pb-kpi-sub">{{ String((it as any)?.sub) }}</span>
      </div>
    </div>
  </template>

  <!-- 卡片网格 -->
  <template v-else-if="block.type === 'cards-grid'">
    <div class="pb-cards">
      <div v-for="(it, i) in items('items')" :key="i" class="pb-card">
        <span class="pb-card-icon">{{ String((it as any)?.icon ?? '◆') }}</span>
        <span class="pb-card-title">{{ String((it as any)?.title ?? '') }}</span>
        <span v-if="(it as any)?.desc" class="pb-card-desc">{{ String((it as any)?.desc) }}</span>
      </div>
    </div>
  </template>

  <!-- 左右分栏（对比） -->
  <template v-else-if="block.type === 'columns'">
    <div class="pb-columns">
      <div v-for="(side, si) in ['left', 'right']" :key="si" class="pb-col">
        <span class="pb-col-title">{{ colTitle(side) }}</span>
        <ul class="pb-bullets pb-col-list">
          <li v-for="(it, i) in colItems(side)" :key="i" class="pb-bullet">
            {{ String(it) }}
          </li>
        </ul>
      </div>
    </div>
  </template>

  <!-- 时间轴 -->
  <template v-else-if="block.type === 'timeline'">
    <div class="pb-timeline">
      <div v-for="(it, i) in items('items')" :key="i" class="pb-tl-item">
        <span class="pb-tl-dot"></span>
        <span class="pb-tl-time">{{ String((it as any)?.time ?? '') }}</span>
        <span class="pb-tl-body">
          <b>{{ String((it as any)?.title ?? '') }}</b>
          <span v-if="(it as any)?.desc" class="pb-tl-desc">{{ String((it as any)?.desc) }}</span>
        </span>
      </div>
    </div>
  </template>

  <!-- 图片（已上传则渲染真实图片，否则占位） -->
  <template v-else-if="block.type === 'image'">
    <div class="pb-image" :style="{ '--pf-img-size': imgSize }">
      <div v-if="str('src')" class="pb-image-real">
        <img :src="str('src')" class="pb-image-img" alt="" />
      </div>
      <div v-else class="pb-image-inner">
        <svg
          class="pb-image-svg"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          stroke-width="1.4"
        >
          <rect x="3" y="5" width="18" height="14" rx="2" />
          <circle cx="9" cy="10" r="1.6" />
          <path d="M4 17l5-4 4 3 4-4 3 3" />
        </svg>
      </div>
    </div>
  </template>

  <!-- 图表 -->
  <template v-else-if="block.type === 'chart'">
    <div class="pb-chart">
      <svg
        v-if="chartKind === 'line'"
        class="pb-chart-line"
        viewBox="0 0 820 320"
        preserveAspectRatio="none"
      >
        <polyline
          :points="linePoints"
          fill="none"
          stroke="var(--pf-accent)"
          stroke-width="5"
          stroke-linejoin="round"
          stroke-linecap="round"
        />
      </svg>
      <div v-else class="pb-bars">
        <div v-for="(v, i) in chartValues" :key="i" class="pb-bar-col">
          <div class="pb-bar" :style="{ height: `${(v / chartMax) * 100}%` }"></div>
          <span class="pb-bar-label">{{ chartLabels[i] ?? '' }}</span>
        </div>
      </div>
    </div>
  </template>

  <!-- 引用 -->
  <template v-else-if="block.type === 'quote'">
    <div class="pb-quote">
      <span class="pb-quote-mark">\u201c</span>
      <p class="pb-quote-text">{{ str('text') }}</p>
      <p v-if="str('author')" class="pb-quote-author">{{ str('author') }}</p>
    </div>
  </template>

  <!-- 行动号召 -->
  <template v-else-if="block.type === 'cta'">
    <div class="pb-cta">
      <span class="pb-cta-text">{{ str('text') }}</span>
      <span v-if="str('subtext')" class="pb-cta-sub">{{ str('subtext') }}</span>
    </div>
  </template>

  <!-- 分隔线 -->
  <template v-else-if="block.type === 'divider'">
    <div class="pb-divider"></div>
  </template>

  <!-- 兜底 -->
  <template v-else>
    <p class="pb-paragraph">{{ str('text', '未支持的内容块') }}</p>
  </template>
</template>

<style>
/* ═══ 区块基础 ═══ */
.pb-txt-hover {
  cursor: text;
}
.pb-txt-hover:hover {
  text-decoration: underline dotted rgba(0, 0, 0, 0.28);
  text-underline-offset: 4px;
}
.inl-plain {
  display: inline;
}
.pb-kicker {
  font-size: calc(20px * var(--pf-fs, 1));
  letter-spacing: 0.35em;
  text-transform: uppercase;
  color: var(--pf-accent, #2563eb);
  margin: 0 0 18px;
}
.pb-heading {
  font-size: calc(42px * var(--pf-fs, 1));
  font-weight: 700;
  margin: 0 0 8px;
  letter-spacing: 0.01em;
}
.pb-heading-hero {
  font-size: calc(64px * var(--pf-fs, 1));
  font-weight: 800;
  letter-spacing: 0.02em;
  margin: 0 0 26px;
  color: var(--pf-accent, #2563eb);
  line-height: 1.22;
}
.pb-paragraph {
  font-size: calc(26px * var(--pf-fs, 1));
  line-height: 1.7;
  color: var(--pf-muted, #86909c);
  margin: 0 0 10px;
}
/* 富文本段落内部排版（v-html 渲染的 Tiptap HTML） */
.pb-paragraph p {
  margin: 0 0 6px;
}
.pb-paragraph p:last-child {
  margin-bottom: 0;
}
.pb-paragraph ul,
.pb-paragraph ol {
  padding-left: 26px;
  margin: 0 0 8px;
}
.pb-paragraph li {
  margin: 3px 0;
}
.pb-paragraph .text-left {
  text-align: left;
}
.pb-paragraph .text-center {
  text-align: center;
}
.pb-paragraph .text-right {
  text-align: right;
}

/* 要点 */
.pb-bullets {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 24px;
}
.pb-bullet {
  position: relative;
  padding-left: 32px;
  font-size: calc(26px * var(--pf-fs, 1));
  line-height: 1.55;
}
.pb-bullet::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 16px;
  width: 11px;
  height: 11px;
  border-radius: 50%;
  background: var(--pf-accent, #2563eb);
}

/* 编号步骤 */
.pb-numlist {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 22px;
}
.pb-num-item {
  display: flex;
  align-items: center;
  gap: 20px;
  font-size: calc(25px * var(--pf-fs, 1));
}
.pb-num {
  font-size: calc(24px * var(--pf-fs, 1));
  font-weight: 800;
  color: var(--pf-accent, #2563eb);
  min-width: 44px;
}

/* 表格 */
.pb-table {
  width: 100%;
  border-collapse: collapse;
  font-size: calc(22px * var(--pf-fs, 1));
}
.pb-table th {
  text-align: left;
  font-weight: 700;
  padding: 14px 18px;
  color: var(--pf-accent, #2563eb);
  border-bottom: 2px solid var(--pf-accent, #2563eb);
}
.pb-table td {
  padding: 13px 18px;
  border-bottom: 1px solid var(--pf-line, #e5e7eb);
}
.pb-table td.strong {
  font-weight: 600;
}

/* 数据卡 */
.pb-kpis {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 28px;
  margin-top: 6px;
}
.pb-kpi {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 26px 28px;
  border-radius: 16px;
  border: 1px solid var(--pf-line, #e5e7eb);
}
.pb-kpi-value {
  font-size: calc(56px * var(--pf-fs, 1));
  font-weight: 800;
  color: var(--pf-accent, #2563eb);
  line-height: 1;
}
.pb-kpi-label {
  margin-top: 12px;
  font-size: calc(20px * var(--pf-fs, 1));
  font-weight: 600;
}
.pb-kpi-sub {
  margin-top: 4px;
  font-size: calc(16px * var(--pf-fs, 1));
  color: var(--pf-muted, #86909c);
}

/* 卡片网格 */
.pb-cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 28px;
  margin-top: 6px;
}
.pb-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 26px 26px;
  border-radius: 16px;
  border: 1px solid var(--pf-line, #e5e7eb);
}
.pb-card-icon {
  width: 46px;
  height: 46px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  font-size: calc(22px * var(--pf-fs, 1));
  color: #fff;
  background: var(--pf-accent, #2563eb);
}
.pb-card-title {
  font-size: calc(25px * var(--pf-fs, 1));
  font-weight: 700;
}
.pb-card-desc {
  font-size: calc(18px * var(--pf-fs, 1));
  line-height: 1.55;
  color: var(--pf-muted, #86909c);
}

/* 左右分栏 */
.pb-columns {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 34px;
  margin-top: 6px;
}
.pb-col {
  padding: 26px 30px;
  border-radius: 16px;
  border: 1px solid var(--pf-line, #e5e7eb);
}
.pb-col-title {
  display: block;
  font-size: calc(27px * var(--pf-fs, 1));
  font-weight: 700;
  margin-bottom: 18px;
}
.pb-col-list {
  gap: 14px;
}
.pb-col-list .pb-bullet {
  font-size: calc(21px * var(--pf-fs, 1));
}

/* 时间轴 */
.pb-timeline {
  display: flex;
  flex-direction: column;
  gap: 26px;
  margin-top: 6px;
}
.pb-tl-item {
  display: flex;
  align-items: flex-start;
  gap: 22px;
  position: relative;
  padding-left: 4px;
}
.pb-tl-item::before {
  content: '';
  position: absolute;
  left: 7px;
  top: 34px;
  bottom: -28px;
  width: 2px;
  background: var(--pf-line, #e5e7eb);
}
.pb-tl-item:last-child::before {
  display: none;
}
.pb-tl-dot {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--pf-accent, #2563eb);
  flex-shrink: 0;
  margin-top: 7px;
}
.pb-tl-time {
  min-width: 90px;
  font-size: calc(22px * var(--pf-fs, 1));
  font-weight: 800;
  color: var(--pf-accent, #2563eb);
}
.pb-tl-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.pb-tl-body b {
  font-size: calc(24px * var(--pf-fs, 1));
}
.pb-tl-desc {
  font-size: calc(18px * var(--pf-fs, 1));
  color: var(--pf-muted, #86909c);
}

/* 图片占位 */
.pb-image-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  height: 300px;
  border-radius: 16px;
  border: 1.5px dashed var(--pf-line, #cbd5e1);
  color: var(--pf-muted, #86909c);
}
.pb-image-svg {
  width: 64px;
  height: 64px;
  opacity: 0.6;
}
/* 已上传图片：自动适配图片比例，宽度受 --pf-img-size（占本列比例）控制 */
.pb-image-real {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: calc(var(--pf-img-size, 1) * 100%);
  max-width: 100%;
  max-height: 560px;
  margin: 0 auto;
  border-radius: 14px;
  overflow: hidden;
  border: 1px solid var(--pf-line, #e5e7eb);
}
.pb-image-img {
  display: block;
  max-width: 100%;
  max-height: 560px;
  height: auto;
}

/* 图表 */
.pb-chart {
  margin-top: 6px;
}
.pb-bars {
  display: flex;
  align-items: flex-end;
  gap: 34px;
  height: 300px;
  padding: 0 10px;
}
.pb-bar-col {
  flex: 1;
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
}
.pb-bar {
  width: 100%;
  max-width: 84px;
  border-radius: 8px 8px 0 0;
  background: linear-gradient(180deg, var(--pf-accent, #2563eb), var(--pf-tint, #eff6ff));
  transition: height 0.3s ease;
}
.pb-bar-label {
  font-size: calc(17px * var(--pf-fs, 1));
  color: var(--pf-muted, #86909c);
}
.pb-chart-line {
  width: 100%;
  height: 300px;
}

/* 引用 */
.pb-quote {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  padding: 0 40px;
}
.pb-quote-mark {
  font-size: calc(120px * var(--pf-fs, 1));
  line-height: 0.6;
  color: var(--pf-accent, #2563eb);
  opacity: 0.35;
  font-family: Georgia, serif;
  margin-bottom: 10px;
}
.pb-quote-text {
  font-size: calc(38px * var(--pf-fs, 1));
  font-weight: 600;
  line-height: 1.6;
  margin: 0 0 26px;
  max-width: 980px;
}
.pb-quote-author {
  font-size: calc(21px * var(--pf-fs, 1));
  color: var(--pf-muted, #86909c);
  margin: 0;
}

/* 行动号召 */
.pb-cta {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  margin-top: 22px;
}
.pb-cta-text {
  font-size: calc(30px * var(--pf-fs, 1));
  font-weight: 700;
  color: var(--pf-accent, #2563eb);
}
.pb-cta-sub {
  font-size: calc(20px * var(--pf-fs, 1));
  color: var(--pf-muted, #86909c);
}

/* 分隔线 */
.pb-divider {
  height: 2px;
  border-radius: 1px;
  background: var(--pf-line, #e5e7eb);
  margin: 8px 0;
}
</style>
