<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { parseMarkdownTable } from '@/utils/uiBlocks'
import { renderMdInline } from '@/composables/useAgentChat'

/**
 * 对话内 markdown 表格 → 用户直编（仅在父级批注态下激活，无独立编辑入口）。
 * 编辑动作即变更记录（格改/删行/删列/改名/增列/增行，锚点插入），由批注栏统一收集 getRevision()
 * 组装成【答复修订】消息交 AI 应用——前端零写路径。只读态沿用全局 .markdown-body 表格样式 + 单元格行内 markdown。
 */
const props = defineProps<{ md: string; forced?: boolean }>()

const table = computed(() => parseMarkdownTable(props.md))
const editing = computed(() => !!props.forced)

const inlineHtml = ref<Record<string, string>>({})
watch(table, async (t) => {
  if (!t) return
  for (let c = 0; c < t.headers.length; c++) inlineHtml.value[`h:${c}`] = await renderMdInline(t.headers[c])
  for (let r = 0; r < t.rows.length; r++)
    for (let c = 0; c < t.headers.length; c++) {
      const raw = t.rows[r][c]
      if (raw) inlineHtml.value[`${r}:${c}`] = await renderMdInline(raw)
    }
}, { immediate: true })

type Anchor = number | 'end'
const cellEdits = ref<Record<string, string>>({})        // `${r}:${c}`(原坐标) -> 新值
const deletedRows = ref<number[]>([])
const deletedCols = ref<number[]>([])
const headerEdits = ref<Record<number, string>>({})
const addedCols = ref<{ header: string; vals: Record<string, string>; anchor: Anchor; dead: boolean }[]>([])
const addedRows = ref<{ cells: Record<string, string>; anchor: Anchor; dead: boolean }[]>([])

const baseCols = computed(() => table.value ? table.value.headers.map((_, c) => c).filter(c => !deletedCols.value.includes(c)) : [])
const liveRowIdx = computed(() => table.value ? table.value.rows.map((_, r) => r).filter(r => !deletedRows.value.includes(r)) : [])

type ColView = { k: 'b'; c: number } | { k: 'c'; i: number }
type RowView = { k: 'b'; r: number } | { k: 'a'; i: number }
const colKey = (v: ColView) => v.k === 'b' ? `b${v.c}` : `c${v.i}`
const rowKey = (v: RowView) => v.k === 'b' ? `b${v.r}` : `a${v.i}`
const colViews = computed<ColView[]>(() => {
  const out: ColView[] = []
  for (const c of baseCols.value) {
    out.push({ k: 'b', c })
    addedCols.value.forEach((ac, i) => { if (!ac.dead && ac.anchor === c) out.push({ k: 'c', i }) })
  }
  addedCols.value.forEach((ac, i) => { if (!ac.dead && ac.anchor === 'end') out.push({ k: 'c', i }) })
  return out
})
const rowViews = computed<RowView[]>(() => {
  const out: RowView[] = []
  for (const r of liveRowIdx.value) {
    out.push({ k: 'b', r })
    addedRows.value.forEach((ar, i) => { if (!ar.dead && ar.anchor === r) out.push({ k: 'a', i }) })
  }
  addedRows.value.forEach((ar, i) => { if (!ar.dead && ar.anchor === 'end') out.push({ k: 'a', i }) })
  return out
})

const colName = (c: number) => headerEdits.value[c] ?? table.value!.headers[c]
const curCell = (r: number, c: number) => cellEdits.value[`${r}:${c}`] ?? table.value!.rows[r][c]
const setCell = (r: number, c: number, v: string) => {
  const k = `${r}:${c}`
  if (v === table.value!.rows[r][c]) delete cellEdits.value[k]
  else cellEdits.value[k] = v
}
function delRow(r: number) {
  if (deletedRows.value.includes(r)) return
  deletedRows.value.push(r)
  for (const k of Object.keys(cellEdits.value)) if (k.startsWith(`${r}:`)) delete cellEdits.value[k]
}
function delCol(c: number) {
  if (deletedCols.value.includes(c)) return
  deletedCols.value.push(c)
  for (const k of Object.keys(cellEdits.value)) if (k.endsWith(`:${c}`)) delete cellEdits.value[k]
}
function insertColAt(anchor: Anchor) { addedCols.value.push({ header: '', vals: {}, anchor, dead: false }) }
function insertRowAt(anchor: Anchor) { addedRows.value.push({ cells: {}, anchor, dead: false }) }

const liveAddedCols = computed(() => addedCols.value.map((ac, i) => ({ ac, i })).filter(x => !x.ac.dead))
const liveAddedRows = computed(() => addedRows.value.map((ar, i) => ({ ar, i })).filter(x => !x.ar.dead))
const changeCount = computed(() =>
  Object.keys(cellEdits.value).length + deletedRows.value.length + deletedCols.value.length
  + Object.keys(headerEdits.value).length + liveAddedCols.value.length + liveAddedRows.value.length)

/** 行标识：首存活列当前值（编辑后），无值退回原行号 */
function rowLabel(r: number): string {
  const first = baseCols.value[0]
  const v = first === undefined ? '' : curCell(r, first)
  return v ? `「${v}」` : `第${r + 1}行`
}
function anchorColLabel(a: Anchor): string {
  return a === 'end' || deletedCols.value.includes(a) ? '' : `位于「${colName(a)}」列之后，`
}
function anchorRowDesc(a: Anchor): string {
  return a === 'end' || deletedRows.value.includes(a) ? '在表末' : `在${rowLabel(a)}之后`}

function buildRevision(): { diff: string; headerLabel: string } {
  const t = table.value!
  const lines: string[] = []
  for (const c of baseCols.value) if (headerEdits.value[c] && headerEdits.value[c] !== t.headers[c])
    lines.push(`- 列「${t.headers[c]}」更名为「${headerEdits.value[c]}」`)
  for (const c of [...deletedCols.value].sort((a, b) => a - b))
    lines.push(`- 删除列「${headerEdits.value[c] ?? t.headers[c]}」`)
  let anon = 0
  for (const { ac } of liveAddedCols.value) {
    const vals = rowViews.value.map(v => ac.vals[rowKey(v)] ?? '')
    const header = ac.header.trim()
    if (!header && !vals.some(v => v.trim())) { anon++; continue }
    const name = header || `新列${++anon}`
    lines.push(`- 新增列「${name}」（${anchorColLabel(ac.anchor) || '置于表末，'}自上而下各行值）：${vals.join('；')}`)
  }
  for (const r of [...deletedRows.value].sort((a, b) => a - b))
    lines.push(`- 删除${rowLabel(r)}行（原：${t.rows[r].join(' | ')}）`)
  for (const [k, v] of Object.entries(cellEdits.value)) {
    const [r, c] = k.split(':').map(Number)
    if (deletedRows.value.includes(r) || deletedCols.value.includes(c)) continue
    lines.push(`- ${rowLabel(r)}的「${colName(c)}」格：${t.rows[r][c] || '（空）'} → ${v || '（空）'}`)
  }
  for (const { ar } of liveAddedRows.value) {
    const cells = colViews.value.map(cv => ar.cells[colKey(cv)] ?? '')
    if (!cells.some(v => v.trim())) continue
    lines.push(`- ${anchorRowDesc(ar.anchor)}新增行：${cells.join(' | ')}`)
  }
  return { diff: lines.join('\n'), headerLabel: baseCols.value.map(c => colName(c)).join('|') }
}

function resetEdits() {
  cellEdits.value = {}; deletedRows.value = []; deletedCols.value = []; headerEdits.value = {}
  addedCols.value = []; addedRows.value = []
}
// 退出批注态即丢弃未提交编辑（与批注 chips 丢弃语义一致）
watch(editing, (on) => { if (!on) resetEdits() })

defineExpose({
  hasChanges: () => changeCount.value > 0,
  getRevision: buildRevision,
})
</script>

<template>
  <!-- 非表格/解析失败：防御性原样输出 -->
  <pre v-if="!table" class="text-[11px] whitespace-pre-wrap text-surface-600">{{ md }}</pre>

  <!-- 只读态：全局 markdown-body 表格观感（编辑能力仅批注态提供） -->
  <div v-else-if="!editing" class="markdown-body">
    <table>
      <thead><tr><th v-for="(h, c) in table.headers" :key="c"><span v-if="inlineHtml['h:' + c]" v-html="inlineHtml['h:' + c]" />{{ inlineHtml['h:' + c] ? '' : h }}</th></tr></thead>
      <tbody><tr v-for="(row, r) in table.rows" :key="r"><td v-for="(cell, c) in row" :key="c"><span v-if="inlineHtml[r + ':' + c]" v-html="inlineHtml[r + ':' + c]" />{{ inlineHtml[r + ':' + c] ? '' : cell }}</td></tr></tbody>
    </table>
  </div>

  <!-- 编辑态（批注模式内）：操作按钮悬停显形 -->
  <div v-else class="md-table-root group/tbl overflow-x-auto border border-brand-200 rounded-lg bg-white p-1.5">
    <table class="text-[11px] border-collapse">
      <thead>
        <tr>
          <th v-for="cv in colViews" :key="colKey(cv)" class="border border-surface-200 bg-surface-50 p-0.5 align-top">
            <input v-if="cv.k === 'b'" :value="colName(cv.c)" @input="headerEdits[cv.c] = ($event.target as HTMLInputElement).value"
              class="min-w-[52px] w-full px-1 py-0.5 text-[11px] font-medium bg-transparent outline-none border-b border-transparent focus:border-brand-400" />
            <input v-else v-model="addedCols[cv.i].header" placeholder="新列名"
              class="min-w-[52px] w-full px-1 py-0.5 text-[11px] font-medium bg-transparent outline-none border-b border-brand-300 focus:border-brand-400" />
            <div class="flex items-center justify-end gap-1 px-0.5 opacity-0 group-hover/tbl:opacity-100 transition-opacity">
              <button type="button" @click="insertColAt(cv.k === 'b' ? cv.c : addedCols[cv.i].anchor)"
                class="text-[10px] text-surface-400 hover:text-brand-600 leading-none" title="在此列右侧插入新列">＋</button>
              <button v-if="cv.k === 'b'" type="button" @click="delCol(cv.c)"
                class="text-[10px] text-surface-400 hover:text-red-500 leading-none px-0.5" title="删除此列">×</button>
              <button v-else type="button" @click="addedCols[cv.i].dead = true"
                class="text-[10px] text-surface-400 hover:text-red-500 leading-none px-0.5" title="删除新增列">×</button>
            </div>
          </th>
          <th class="w-12 border-none align-middle"><button type="button" @click="insertColAt('end')" class="text-[10px] text-brand-600 hover:underline whitespace-nowrap px-1 opacity-0 group-hover/tbl:opacity-100 transition-opacity">+ 列</button></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="rv in rowViews" :key="rowKey(rv)">
          <td v-for="cv in colViews" :key="colKey(cv)" class="border border-surface-200 p-0"
            :class="cv.k === 'c' ? 'bg-brand-50/30' : (rv.k === 'a' ? 'bg-emerald-50/30' : '')">
            <input v-if="rv.k === 'b' && cv.k === 'b'" :value="curCell(rv.r, cv.c)" @input="setCell(rv.r, cv.c, ($event.target as HTMLInputElement).value)"
              class="w-full min-w-[46px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
            <input v-else-if="rv.k === 'b' && cv.k === 'c'" :value="addedCols[cv.i].vals[rowKey(rv)] ?? ''" @input="addedCols[cv.i].vals[rowKey(rv)] = ($event.target as HTMLInputElement).value"
              class="w-full min-w-[46px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
            <input v-else-if="rv.k === 'a' && cv.k === 'b'" :value="addedRows[rv.i].cells[colKey(cv)] ?? ''" @input="addedRows[rv.i].cells[colKey(cv)] = ($event.target as HTMLInputElement).value"
              class="w-full min-w-[46px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
            <input v-else :value="addedRows[(rv as { k: 'a'; i: number }).i].cells[colKey(cv)] ?? ''" @input="addedRows[(rv as { k: 'a'; i: number }).i].cells[colKey(cv)] = ($event.target as HTMLInputElement).value"
              class="w-full min-w-[46px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
          </td>
          <td class="border-none whitespace-nowrap pl-1 opacity-0 group-hover/tbl:opacity-100 transition-opacity">
            <button v-if="rv.k === 'b'" type="button" @click="insertRowAt(rv.r)" class="text-[10px] text-surface-400 hover:text-brand-600 leading-none px-0.5" title="在此行下方插入新行">＋</button>
            <button v-if="rv.k === 'b'" type="button" @click="delRow(rv.r)" class="text-[10px] text-surface-400 hover:text-red-500 leading-none px-0.5" title="删除此行">×</button>
            <button v-else type="button" @click="addedRows[rv.i].dead = true" class="text-[10px] text-surface-400 hover:text-red-500 leading-none px-0.5" title="删除新增行">×</button>
          </td>
        </tr>
      </tbody>
    </table>
    <div class="flex items-center justify-between px-1 pt-1">
      <button type="button" @click="insertRowAt('end')" class="text-[10px] text-brand-600 hover:underline opacity-0 group-hover/tbl:opacity-100 transition-opacity">+ 行</button>
      <span v-if="changeCount" class="text-[10px] text-surface-400 opacity-0 group-hover/tbl:opacity-100 transition-opacity">已修改 {{ changeCount }} 处</span>
    </div>
  </div>
</template>
