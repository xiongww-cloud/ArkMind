<script setup lang="ts">
import { computed, ref } from 'vue'
import { parseMarkdownTable } from '@/utils/uiBlocks'

/**
 * 对话内 markdown 表格 → 用户直编。操作日志式：编辑动作本身即变更记录（格改/删行/删列/改名/增列/增行），
 * 提交 = 变更说明消息（不含全表），AI 逐项机械应用。删行作废该行改格，删列作废该列改格。
 */
const props = defineProps<{ md: string }>()
const emit = defineEmits<{ submit: [text: string] }>()

const table = computed(() => parseMarkdownTable(props.md))
const editing = ref(false)

const cellEdits = ref<Record<string, string>>({})      // `${r}:${c}`(原坐标) -> 新值
const deletedRows = ref<number[]>([])                   // 原行号
const deletedCols = ref<number[]>([])                   // 原列号
const headerEdits = ref<Record<number, string>>({})    // 原列号 -> 新列名
const addedCols = ref<{ header: string; values: string[] }[]>([]) // values 按可见行序
const addedRows = ref<string[][]>([])                   // 按可见列序

const baseCols = computed(() => table.value ? table.value.headers.map((_, c) => c).filter(c => !deletedCols.value.includes(c)) : [])
const colName = (c: number) => headerEdits.value[c] ?? table.value!.headers[c]
const curCell = (r: number, c: number) => cellEdits.value[`${r}:${c}`] ?? table.value!.rows[r][c]
const setCell = (r: number, c: number, v: string) => {
  const k = `${r}:${c}`
  if (v === table.value!.rows[r][c]) delete cellEdits.value[k]
  else cellEdits.value[k] = v
}
const delRow = (r: number) => {
  if (deletedRows.value.includes(r)) return
  const pos = liveRowIdx.value.indexOf(r)
  deletedRows.value.push(r)
  for (const k of Object.keys(cellEdits.value)) if (k.startsWith(`${r}:`)) delete cellEdits.value[k]
  addedCols.value.forEach(ac => ac.values.splice(pos, 1))
}
const delCol = (c: number) => {
  if (deletedCols.value.includes(c)) return
  const vis = table.value!.headers.map((_, x) => x).filter(x => !deletedCols.value.includes(x))
  const pos = vis.indexOf(c)
  deletedCols.value.push(c)
  for (const k of Object.keys(cellEdits.value)) if (k.endsWith(`:${c}`)) delete cellEdits.value[k]
  addedRows.value = addedRows.value.map(row => { const r = [...row]; r.splice(pos, 1); return r })
}
const liveRowIdx = computed(() => table.value ? table.value.rows.map((_, r) => r).filter(r => !deletedRows.value.includes(r)) : [])
const rowLabel = (r: number) => {
  const first = baseCols.value[0]
  const v = first === undefined ? '' : curCell(r, first)
  return v ? `「${v}」` : `第${r + 1}行`
}
const setAddedCell = (ai: number, ci: number, v: string) => { const row = addedRows.value[ai]; if (row) row[ci] = v }

const changeCount = computed(() =>
  Object.keys(cellEdits.value).length + deletedRows.value.length + deletedCols.value.length
  + Object.keys(headerEdits.value).length + addedCols.value.length + addedRows.value.length)

function addRow() {
  const n = baseCols.value.length + addedCols.value.length
  addedRows.value.push(Array.from({ length: n }, () => ''))
}
function popRow(i: number) { addedRows.value.splice(i, 1) }
function addCol() {
  const rows = liveRowIdx.value.length + addedRows.value.length
  addedCols.value.push({ header: '', values: Array.from({ length: rows }, () => '') })
  for (const row of addedRows.value) row.push('')
}

function buildChanges(): string {
  const t = table.value!
  const lines: string[] = ['【表格修订】请对你上文输出的表格应用以下变更，未提及的内容保持原样：']
  for (const c of baseCols.value) if (headerEdits.value[c] && headerEdits.value[c] !== t.headers[c])
    lines.push(`- 列「${t.headers[c]}」更名为「${headerEdits.value[c]}」`)
  for (const c of [...deletedCols.value].sort((a, b) => a - b))
    lines.push(`- 删除列「${headerEdits.value[c] ?? t.headers[c]}」`)
  addedCols.value.forEach((ac, k) => {
    const vals = [...liveRowIdx.value.map(r => curCellAddCol(r, k)), ...addedRows.value.map(row => row[baseCols.value.length + k] ?? '')]
    lines.push(`- 新增列「${ac.header.trim() || `新列${k + 1}`}」（自上而下各行值）：${vals.map(v => v.trim() || '—').join('；')}`)
  })
  for (const r of [...deletedRows.value].sort((a, b) => a - b))
    lines.push(`- 删除${rowLabel(r)}行（原：${t.rows[r].join(' | ')}）`)
  for (const [k, v] of Object.entries(cellEdits.value)) {
    const [r, c] = k.split(':').map(Number)
    if (deletedRows.value.includes(r) || deletedCols.value.includes(c)) continue
    lines.push(`- ${rowLabel(r)}的「${colName(c)}」格：${t.rows[r][c] || '（空）'} → ${v || '（空）'}`)
  }
  for (const row of addedRows.value)
    lines.push(`- 新增行：${row.map((v, i) => v.trim() || (i < baseCols.value.length ? '—' : '')).filter(Boolean).join(' | ')}`)
  return lines.length > 1 ? lines.join('\n') : ''
}
function curCellAddCol(r: number, k: number) { return addedCols.value[k]?.values[liveRowIdx.value.indexOf(r)] ?? '' }

function submit() {
  const text = buildChanges()
  if (!text) return
  editing.value = false
  emit('submit', text)
}
function cancel() {
  cellEdits.value = {}; deletedRows.value = []; deletedCols.value = []; headerEdits.value = {}
  addedCols.value = []; addedRows.value = []; editing.value = false
}
</script>

<template>
  <!-- 非表格/解析失败：理论上不会被挂载，防御性原样输出 -->
  <pre v-if="!table" class="text-[11px] whitespace-pre-wrap text-surface-600">{{ md }}</pre>

  <!-- 只读态：普通表格 + 悬浮编辑入口 -->
  <div v-else-if="!editing" class="group relative overflow-x-auto">
    <table class="w-full text-[11px] border-collapse my-0.5">
      <thead><tr><th v-for="(h, c) in table.headers" :key="c" class="text-left font-medium text-surface-500 border border-surface-200 bg-surface-50 px-1.5 py-1 whitespace-nowrap">{{ h }}</th></tr></thead>
      <tbody><tr v-for="(row, r) in table.rows" :key="r"><td v-for="(cell, c) in row" :key="c" class="border border-surface-200 px-1.5 py-1 text-surface-700">{{ cell }}</td></tr></tbody>
    </table>
    <button type="button" @click="editing = true"
      class="absolute top-1 right-1 px-2 py-0.5 rounded-md border border-surface-200 bg-white/95 text-[10px] text-surface-500 hover:text-brand-600 hover:border-brand-300 opacity-0 group-hover:opacity-100 transition-opacity shadow-sm">✎ 编辑表格</button>
  </div>

  <!-- 编辑态：操作日志收集，提交变更说明 -->
  <div v-else class="overflow-x-auto border border-brand-200 rounded-lg bg-white p-1.5">
    <table class="w-full text-[11px] border-collapse">
      <thead>
        <tr>
          <th v-for="c in baseCols" :key="'h' + c" class="border border-surface-200 bg-surface-50 p-0.5">
            <div class="flex items-center gap-0.5">
              <input :value="colName(c)" @input="headerEdits[c] = ($event.target as HTMLInputElement).value"
                class="min-w-[54px] w-full px-1 py-0.5 text-[11px] font-medium bg-transparent outline-none border-b border-transparent focus:border-brand-400" />
              <button type="button" @click="delCol(c)" class="px-0.5 text-surface-300 hover:text-red-500 leading-none">×</button>
            </div>
          </th>
          <th v-for="(ac, k) in addedCols" :key="'ha' + k" class="border border-brand-200 bg-brand-50/40 p-0.5">
            <input v-model="ac.header" placeholder="新列名"
              class="min-w-[54px] w-full px-1 py-0.5 text-[11px] font-medium bg-transparent outline-none focus:border-brand-400" />
          </th>
          <th class="w-14 border-none"><button type="button" @click="addCol" class="text-[10px] text-brand-600 hover:underline whitespace-nowrap px-1">+ 列</button></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="r in liveRowIdx" :key="'r' + r">
          <td v-for="c in baseCols" :key="'c' + c" class="border border-surface-200 p-0">
            <input :value="curCell(r, c)" @input="setCell(r, c, ($event.target as HTMLInputElement).value)"
              class="w-full min-w-[48px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
          </td>
          <td v-for="(ac, k) in addedCols" :key="'ca' + k" class="border border-brand-200 bg-brand-50/30 p-0">
            <input :value="curCellAddCol(r, k)" @input="ac.values[liveRowIdx.indexOf(r)] = ($event.target as HTMLInputElement).value"
              class="w-full min-w-[48px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
          </td>
          <td class="border-none text-center"><button type="button" @click="delRow(r)" class="text-surface-300 hover:text-red-500 px-1 leading-none" title="删除此行">×</button></td>
        </tr>
        <tr v-for="(row, ai) in addedRows" :key="'ar' + ai">
          <td v-for="(cell, ci) in row" :key="'ac' + ci" class="border border-emerald-200 bg-emerald-50/30 p-0">
            <input :value="cell" @input="setAddedCell(ai, ci, ($event.target as HTMLInputElement).value)"
              class="w-full min-w-[48px] px-1.5 py-1 text-[11px] bg-transparent outline-none focus:bg-white focus:ring-1 focus:ring-brand-300" />
          </td>
          <td class="border-none text-center"><button type="button" @click="popRow(ai)" class="text-surface-300 hover:text-red-500 px-1 leading-none">×</button></td>
        </tr>
      </tbody>
    </table>
    <div class="flex items-center justify-between px-1 pt-1.5">
      <button type="button" @click="addRow" class="text-[10px] text-brand-600 hover:underline">+ 行</button>
      <div class="flex items-center gap-2.5">
        <span v-if="changeCount" class="text-[10px] text-surface-400">已修改 {{ changeCount }} 处</span>
        <button type="button" @click="cancel" class="text-[10px] text-surface-400 hover:text-surface-600">取消</button>
        <button type="button" :disabled="!changeCount" @click="submit"
          class="px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors"
          :class="changeCount ? 'bg-brand-600 text-white hover:bg-brand-700' : 'bg-surface-100 text-surface-400 cursor-not-allowed'">提交修订</button>
      </div>
    </div>
  </div>
</template>
