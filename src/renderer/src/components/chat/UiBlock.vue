<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import {
  parseUiBlock, isSubmitted, markSubmitted, formSubmitText, tableDiffText,
  LIMITS, type UiFormBlock, type UiTableBlock, type TableRowState,
} from '@/utils/uiBlocks'
import { toast } from '@/composables/useToast'

const props = defineProps<{ raw: string; msgId: string }>()
const emit = defineEmits<{ send: [text: string] }>()

const parsed = computed(() => parseUiBlock(props.raw))
const submitted = ref(isSubmitted(props.msgId, props.raw))

const form = computed(() => (parsed.value?.type === 'form' ? (parsed.value as UiFormBlock) : null))
const table = computed(() => (parsed.value?.type === 'table' ? (parsed.value as UiTableBlock) : null))

// ── form ──
const formValues = reactive<Record<string, string | boolean>>({})
for (const f of (form.value?.fields ?? [])) {
  formValues[f.name] = f.kind === 'checkbox' ? Boolean(f.value) : String(f.value ?? '')
}
const formValid = computed(() => (form.value?.fields ?? []).every(f =>
  f.kind === 'checkbox' || !f.required || String(formValues[f.name] ?? '').trim()))
function submitForm() {
  if (!form.value || !formValid.value) { toast({ message: '必填项尚未填写完整', type: 'error' }); return }
  const text = formSubmitText(form.value, { ...formValues })
  markSubmitted(props.msgId, props.raw)
  submitted.value = true
  emit('send', text)
}

// ── editable table ──
const rows = ref<TableRowState[]>([])
if (table.value) {
  rows.value = table.value.rows.map(r => ({ cells: { ...r }, deleted: false }))
}
const rowTotal = computed(() => rows.value.filter(r => !r.deleted).length)
function addRow() {
  if (!table.value || rows.value.length >= LIMITS.tableRows) return
  const cells: Record<string, string> = {}
  for (const c of table.value.columns) cells[c.key] = ''
  rows.value.push({ cells, deleted: false })
}
const canSubmitTable = computed(() => {
  if (!table.value) return false
  return tableDiffText(table.value, table.value.rows, rows.value) !== null
})
function submitTable() {
  if (!table.value) return
  const text = tableDiffText(table.value, table.value.rows, rows.value)
  if (!text) { toast({ message: '尚未做任何修改', type: 'info' }); return }
  markSubmitted(props.msgId, props.raw)
  submitted.value = true
  emit('send', text)
}
function cellValue(row: TableRowState, key: string): string { return String(row.cells[key] ?? '') }
function setCell(row: TableRowState, key: string, v: string) { row.cells[key] = v }
</script>

<template>
  <!-- 降级：坏 JSON / 未知类型 / 超限 → 原样代码块 -->
  <pre v-if="!parsed" class="ui-block-fallback text-[11px] bg-surface-50 border border-surface-200 rounded-lg p-2 overflow-x-auto whitespace-pre-wrap">{{ raw }}</pre>

  <!-- 表单 -->
  <div v-else-if="form" class="rounded-lg border border-surface-200 bg-white my-1 overflow-hidden">
    <div class="px-2.5 py-1.5 bg-surface-50 border-b border-surface-100 flex items-center justify-between gap-2">
      <span class="text-[11px] font-semibold text-surface-700">{{ form.title }}</span>
      <span v-if="submitted" class="text-[10px] text-emerald-600">已提交</span>
    </div>
    <div class="p-2.5 space-y-1.5" :class="submitted ? 'opacity-60 pointer-events-none' : ''">
      <label v-for="f in form.fields" :key="f.name" class="block">
        <span class="text-[10px] text-surface-500">{{ f.label }}<span v-if="f.required" class="text-red-500">*</span></span>
        <select v-if="f.kind === 'select'" v-model="formValues[f.name]"
          class="mt-0.5 w-full text-[11px] border border-surface-200 rounded-md px-2 py-1 outline-none focus:border-brand-400 bg-white">
          <option value="" disabled selected>请选择</option>
          <option v-for="o in f.options" :key="o" :value="o">{{ o }}</option>
        </select>
        <div v-else-if="f.kind === 'checkbox'" class="mt-0.5 flex items-center gap-2">
          <input type="checkbox" v-model="formValues[f.name]" class="w-3 h-3 accent-indigo-600" />
          <span class="text-[10px] text-surface-600">{{ f.placeholder || '是' }}</span>
        </div>
        <input v-else v-model="formValues[f.name]" type="text" :placeholder="f.placeholder || ''"
          class="mt-0.5 w-full text-[11px] border border-surface-200 rounded-md px-2 py-1 outline-none focus:border-brand-400" />
      </label>
      <div class="pt-0.5 flex justify-end">
        <button @click="submitForm" :disabled="!formValid"
          class="px-2.5 py-0.5 text-[10px] font-medium rounded-md bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-40 disabled:cursor-not-allowed">
          {{ form.submit_label || '提交' }}
        </button>
      </div>
    </div>
  </div>

  <!-- 可编辑清单：外观≈普通表格，单元格原地改；行删除悬停浮现，不占固定列 -->
  <div v-else-if="table" class="rounded-lg border border-surface-200 bg-white my-1 overflow-hidden">
    <div class="px-2.5 py-1.5 bg-surface-50 border-b border-surface-100 flex items-center justify-between gap-2">
      <span class="text-[11px] font-semibold text-surface-700">{{ table.title }}</span>
      <span v-if="submitted" class="text-[10px] text-emerald-600">已提交</span>
    </div>
    <div class="px-1.5 pt-1 pb-1.5 overflow-x-auto" :class="submitted ? 'opacity-60 pointer-events-none' : ''">
      <table class="w-full text-[11px]">
        <thead>
          <tr>
            <th v-for="c in table.columns" :key="c.key" class="text-left font-medium text-surface-400 px-1 py-0.5 whitespace-nowrap">{{ c.label }}</th>
            <th class="w-0"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(r, i) in rows" :key="i" class="group border-t border-surface-100">
            <template v-if="!r.deleted">
              <td v-for="c in table.columns" :key="c.key" class="px-0 py-0">
                <input size="1" :value="cellValue(r, c.key)" @input="setCell(r, c.key, ($event.target as HTMLInputElement).value)"
                  class="w-full min-w-0 text-[11px] border border-transparent hover:border-surface-200 focus:border-brand-400 rounded px-1 py-0.5 outline-none bg-transparent focus:bg-white" />
              </td>
              <td class="w-0">
                <button v-if="!submitted && rowTotal > 1" @click="r.deleted = true"
                  class="opacity-0 group-hover:opacity-100 transition-opacity text-surface-300 hover:text-red-500 px-1 leading-none" title="删除此行">×</button>
              </td>
            </template>
          </tr>
        </tbody>
      </table>
      <div class="flex items-center justify-between px-1 pt-1" v-if="!submitted">
        <button @click="addRow" :disabled="rows.length >= LIMITS.tableRows"
          class="text-[10px] text-brand-600 hover:underline disabled:opacity-40">+ 加行</button>
        <button @click="submitTable" :disabled="!canSubmitTable"
          class="px-2.5 py-0.5 text-[10px] font-medium rounded-md bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-40 disabled:cursor-not-allowed">提交修改</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ui-block-fallback { font-family: ui-monospace, monospace; }
</style>
