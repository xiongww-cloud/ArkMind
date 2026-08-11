<script setup lang="ts">
/**
 * ExcelImportWizard — 通用 Excel 导入向导（3 步）
 *
 * 流程：点击上传 → 自动弹出文件选择窗 → ① Sheet/表头行 → ② 列映射 → ③ 行审查确认
 */
import { ref, computed, watch, nextTick } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { parseExcelRaw, guessMapping, type ImportField, type RawExcelData, type SheetData, type RawDataRow } from '@/utils/excelRaw'

// ═══════════════════════════════════════════════════════════════════════════
//  PROPS / EMITS
// ═══════════════════════════════════════════════════════════════════════════

export interface RowAction {
  key: string
  label: string
  color: string
  isProblem?: boolean
}

export interface RowDecision {
  action: string
  subnetName?: string
}

export interface ImportConfirmPayload {
  headerRowNumber: number
  mapping: (string | undefined)[]
  rows: RawDataRow[]
  decisions: RowDecision[]
  sheet: SheetData
  sheetNames: string[]
  sheetIndex: number
}

const props = defineProps<{
  modelValue: boolean
  title: string
  fields: ImportField[]
  rowActions: RowAction[]
  classify: (row: RawDataRow, mapping: (string | undefined)[], sheet: SheetData) => string | null
  shouldSkipRow?: (row: RawDataRow, sheet: SheetData) => boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
  confirm: [payload: ImportConfirmPayload]
  cancel: []
}>()

// ═══════════════════════════════════════════════════════════════════════════
//  STATE
// ═══════════════════════════════════════════════════════════════════════════

const step = ref(0) // 0 = 等待文件选择, 1 = Sheet/表头, 2 = 映射, 3 = 审查
const fileInputRef = ref<HTMLInputElement | null>(null)
const selectedFile = ref<File | null>(null)
const fileName = ref('')
const loading = ref(false)
const errorMsg = ref('')
const filePicked = ref(false) // 标记是否已选过文件（避免取消时重复触发）

const excelData = ref<RawExcelData | null>(null)
const activeSheetIndex = ref(0)
const headerRowNum = ref(1)

const sheetOptions = computed(() => excelData.value?.sheets.map((s, i) => ({ index: i, name: s.name })) ?? [])
const activeSheet = computed(() => excelData.value?.sheets[activeSheetIndex.value])

const mapping = ref<(string | undefined)[]>([])
const decisions = ref<RowDecision[]>([])

// ═══════════════════════════════════════════════════════════════════════════
//  打开向导时自动弹出文件选择
// ═══════════════════════════════════════════════════════════════════════════

watch(() => props.modelValue, (val) => {
  if (val && !filePicked.value) {
    // 延迟一帧确保 DOM 已渲染
    nextTick(() => pickFile())
  }
})

// ═══════════════════════════════════════════════════════════════════════════
//  文件选择 + 解析
// ═══════════════════════════════════════════════════════════════════════════

function pickFile() { fileInputRef.value?.click() }

async function onFilePicked(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) {
    // 用户取消了文件选择 → 关闭向导（仅首次未选过文件时）
    if (!filePicked.value) close()
    return
  }
  input.value = ''
  selectedFile.value = file
  filePicked.value = true
  fileName.value = file.name
  loading.value = true
  errorMsg.value = ''
  try {
    excelData.value = await parseExcelRaw(file)
    if (!excelData.value.sheets.length) {
      errorMsg.value = '未找到工作表'
      step.value = 1
      return
    }
    activeSheetIndex.value = 0
    headerRowNum.value = activeSheet.value?.headerRowNumber ?? 1
    initMappingAndDecisions()
    step.value = 1 // 直接进入 Sheet/表头行选择
  } catch (err: any) {
    errorMsg.value = err?.message || '文件解析失败'
    step.value = 1
  } finally {
    loading.value = false
  }
}

function onSheetChange() {
  // 切换 Sheet 时直接用已解析的数据，不重新解析（避免空 Sheet 导致 sheets 清空）
  const sheet = activeSheet.value
  if (!sheet) return
  headerRowNum.value = sheet.headerRowNumber
  initMappingAndDecisions()
}

async function reparseWithHeader(row: number) {
  const file = selectedFile.value
  if (!file) return
  loading.value = true
  try {
    const result = await parseExcelRaw(file, activeSheetIndex.value, row)
    if (result.sheets.length > 0) {
      // 只替换当前 sheet，保留其余 sheets（避免切换表头行后 sheet 选择器消失）
      const sheets = [...(excelData.value?.sheets ?? [])]
      sheets[activeSheetIndex.value] = result.sheets[0]
      excelData.value = { sheets }
      headerRowNum.value = row
      initMappingAndDecisions()
    }
  } catch {} finally { loading.value = false }
}

// ═══════════════════════════════════════════════════════════════════════════
//  映射猜测 + 行决策初始化
// ═══════════════════════════════════════════════════════════════════════════

function initMappingAndDecisions() {
  const sheet = activeSheet.value
  if (!sheet) return
  mapping.value = guessMapping(sheet.headers, props.fields)
  decisions.value = sheet.rows.map(row => {
    const skip = props.shouldSkipRow?.(row, sheet) ?? false
    if (skip) return { action: 'ignore' }
    const autoAction = props.classify(row, mapping.value, sheet)
    // classify 返回 null 表示"问题行"（需要用户选择），用 'problem' 标记
    return { action: autoAction ?? 'problem' }
  })
}

// ═══════════════════════════════════════════════════════════════════════════
//  行审查统计
// ═══════════════════════════════════════════════════════════════════════════

const problemRowCount = computed(() =>
  decisions.value.filter(d => props.rowActions.find(a => a.key === d.action)?.isProblem).length
)

const validRowCount = computed(() =>
  decisions.value.filter(d => d.action !== 'ignore' && d.action !== 'problem').length
)

/** 审查表可见行（隐藏已忽略的行，保留原始索引用于 decisions 映射） */
const visibleRows = computed(() => {
  const sheet = activeSheet.value
  if (!sheet) return []
  return sheet.rows
    .map((row, ri) => ({ row, ri }))
    .filter(({ ri }) => decisions.value[ri]?.action !== 'ignore')
})

function batchSetProblemRows(action: string) {
  for (let i = 0; i < decisions.value.length; i++) {
    const d = decisions.value[i]
    if (props.rowActions.find(a => a.key === d.action)?.isProblem) {
      decisions.value[i] = { ...d, action }
    }
  }
}

// ═══════════════════════════════════════════════════════════════════════════
//  导航
// ═══════════════════════════════════════════════════════════════════════════

const canNextFromStep1 = computed(() => !!activeSheet.value && !loading.value && !errorMsg.value)
const canNextFromStep2 = computed(() => {
  return props.fields.filter(f => f.required).every(f => mapping.value.includes(f.key))
})

/** 非空列索引列表（过滤掉所有行都为空的列，审查表和映射表共用） */
const nonEmptyCols = computed(() => {
  const sheet = activeSheet.value
  if (!sheet) return []
  const cols: number[] = []
  for (let ci = 0; ci < sheet.maxCols; ci++) {
    const hasContent = sheet.headers[ci]?.trim() || sheet.rows.some(r => (r.cells[ci] || '').trim())
    if (hasContent) cols.push(ci)
  }
  return cols
})

function nextStep() {
  if (step.value === 1 && canNextFromStep1.value) step.value = 2
  else if (step.value === 2 && canNextFromStep2.value) step.value = 3
}

function prevStep() {
  if (step.value > 1) step.value--
}

function confirmImport() {
  const sheet = activeSheet.value
  if (!sheet) return
  emit('confirm', {
    headerRowNumber: headerRowNum.value,
    mapping: mapping.value,
    rows: sheet.rows,
    decisions: decisions.value,
    sheet,
    sheetNames: excelData.value?.sheets.map(s => s.name) ?? [],
    sheetIndex: activeSheetIndex.value,
  })
  close()
}

function close() {
  emit('update:modelValue', false)
  emit('cancel')
  step.value = 0
  excelData.value = null
  selectedFile.value = null
  fileName.value = ''
  errorMsg.value = ''
  filePicked.value = false
  mapping.value = []
  decisions.value = []
}

// ═══════════════════════════════════════════════════════════════════════════
//  列映射
// ═══════════════════════════════════════════════════════════════════════════

const mappingOptions = computed(() => [
  { key: '', label: '忽略' },
  ...props.fields.map(f => ({ key: f.key, label: f.label + (f.required ? ' *' : '') })),
])

function isRequiredMapped(): boolean {
  return props.fields.filter(f => f.required).every(f => mapping.value.includes(f.key))
}
</script>

<template>
  <Teleport to="body">
    <!-- 隐藏文件选择框（始终存在，供自动触发和更换文件使用） -->
    <input v-if="modelValue" ref="fileInputRef" type="file" class="hidden" accept=".xlsx,.xls" @change="onFilePicked" />

    <!-- 弹窗：选好文件后才显示（step > 0） -->
    <Transition name="modal-fade">
      <div v-if="modelValue && step > 0" class="fixed inset-0 z-[85] flex items-center justify-center bg-black/30 backdrop-blur-sm" @click.self="close">
        <div class="bg-white rounded-2xl shadow-2xl w-full max-w-5xl mx-4 flex flex-col overflow-hidden"
          :style="{ height: step === 1 ? 'min(45vh, 400px)' : 'min(85vh, 680px)' }">

          <!-- Header -->
          <div class="px-6 py-3 border-b border-gray-100 flex items-center justify-between shrink-0">
            <div class="flex items-center gap-3">
              <h3 class="text-sm font-semibold text-gray-800">{{ title }}</h3>
              <span v-if="fileName" class="text-[11px] text-surface-400 truncate max-w-[200px]">{{ fileName }}</span>
              <!-- Step 指示器 -->
              <div v-if="step > 0" class="flex items-center gap-1 text-[11px]">
                <span :class="step >= 1 ? 'text-brand-600 font-medium' : 'text-surface-400'">① 选择</span>
                <span class="text-surface-300">→</span>
                <span :class="step >= 2 ? 'text-brand-600 font-medium' : 'text-surface-400'">② 映射</span>
                <span class="text-surface-300">→</span>
                <span :class="step >= 3 ? 'text-brand-600 font-medium' : 'text-surface-400'">③ 审查</span>
              </div>
            </div>
            <button @click="close" class="text-gray-400 hover:text-gray-600 transition-colors">
              <AppIcon name="x" class="w-4 h-4" />
            </button>
          </div>

          <!-- Body -->
          <div class="flex-1 overflow-hidden flex flex-col">

            <!-- ── STEP 1：Sheet / 表头行选择 ── -->
            <template v-if="step === 1">
              <div class="flex-1 overflow-y-auto p-6 space-y-5">
                <!-- 错误 -->
                <div v-if="errorMsg" class="p-3 bg-red-50 border border-red-100 rounded-lg text-xs text-red-600">
                  {{ errorMsg }}
                </div>

                <!-- 重新选择文件 -->
                <div class="flex items-center gap-2">
                  <button @click="pickFile" class="text-xs text-brand-600 hover:underline">更换文件</button>
                  <span class="text-xs text-surface-400">{{ fileName }}</span>
                </div>

                <!-- Sheet / 表头行选择 -->
                <template v-if="activeSheet">
                  <div class="grid grid-cols-2 gap-4">
                    <div v-if="sheetOptions.length > 1">
                      <label class="block text-[11px] text-surface-500 mb-1">工作表</label>
                      <select v-model="activeSheetIndex" @change="onSheetChange" class="w-full h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none focus:border-brand-400 transition-colors">
                        <option v-for="opt in sheetOptions" :key="opt.index" :value="opt.index">{{ opt.name }}</option>
                      </select>
                    </div>
                    <div>
                      <label class="block text-[11px] text-surface-500 mb-1">表头行</label>
                      <select v-model.number="headerRowNum" @change="reparseWithHeader(headerRowNum)" class="w-full h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none focus:border-brand-400 transition-colors">
                        <option v-for="r in 10" :key="r" :value="r">第 {{ r }} 行</option>
                      </select>
                    </div>
                  </div>
                  <!-- 表头预览 -->
                  <div class="p-3 bg-slate-50 border border-slate-100 rounded-lg">
                    <p class="text-[11px] text-slate-500 mb-1.5">识别到的列名：</p>
                    <div class="flex flex-wrap gap-1.5">
                      <span v-for="(h, i) in activeSheet.headers" :key="i"
                        class="px-2 py-0.5 rounded text-[11px] font-medium"
                        :class="mapping[i] ? 'bg-brand-50 text-brand-700' : 'bg-slate-100 text-slate-500'">
                        {{ h || '(空)' }}
                      </span>
                    </div>
                  </div>
                  <p class="text-[11px] text-slate-400">
                    共 {{ activeSheet.rows.length }} 行数据，{{ activeSheet.headers.length }} 列
                  </p>
                </template>
              </div>
            </template>

            <!-- ── STEP 2：列映射 ── -->
            <template v-if="step === 2 && activeSheet">
              <div class="flex-1 overflow-y-auto p-6">
                <p class="text-[12px] text-surface-600 mb-3">为每一列选择对应的目标字段。带 * 的字段必须映射。</p>
                <div class="border border-surface-200 rounded-xl overflow-hidden">
                  <table class="w-full text-[12px]">
                    <thead class="bg-slate-50 sticky top-0">
                      <tr>
                        <th class="px-3 py-2 text-left font-medium text-slate-500 w-12">#</th>
                        <th class="px-3 py-2 text-left font-medium text-slate-500">Excel 列名</th>
                        <th class="px-3 py-2 text-left font-medium text-slate-500">映射到字段</th>
                        <th class="px-3 py-2 text-left font-medium text-slate-500">示例数据</th>
                      </tr>
                    </thead>
                    <tbody class="divide-y divide-surface-100">
                      <tr v-for="(ci, idx) in nonEmptyCols" :key="ci" class="hover:bg-slate-50/50">
                        <td class="px-3 py-2 text-surface-400">{{ idx + 1 }}</td>
                        <td class="px-3 py-2 font-medium text-surface-800">{{ activeSheet.headers[ci] || '(空列)' }}</td>
                        <td class="px-3 py-2">
                          <select v-model="mapping[ci]" class="w-full h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none focus:border-brand-400 transition-colors">
                            <option v-for="opt in mappingOptions" :key="opt.key" :value="opt.key || undefined">{{ opt.label }}</option>
                          </select>
                        </td>
                        <td class="px-3 py-2 text-surface-500 truncate max-w-[200px]">
                          {{ activeSheet.rows.slice(0, 3).map(r => r.cells[ci]).filter(Boolean).join(' / ') || '-' }}
                        </td>
                      </tr>
                    </tbody>
                  </table>
                </div>
                <div v-if="!isRequiredMapped()" class="mt-3 p-3 bg-amber-50 border border-amber-200 rounded-lg text-[11px] text-amber-700">
                  以下必填字段尚未映射：{{ props.fields.filter(f => f.required && !mapping.includes(f.key)).map(f => f.label).join('、') }}
                </div>
              </div>
            </template>

            <!-- ── STEP 3：行审查 ── -->
            <template v-if="step === 3 && activeSheet">
              <!-- 汇总条 -->
              <div class="px-6 py-2 border-b border-surface-100 flex items-center gap-4 text-[11px] shrink-0 bg-slate-50/50">
                <span class="text-surface-500">共 {{ decisions.length }} 行</span>
                <span class="text-emerald-600 font-medium">有效 {{ validRowCount }} 行</span>
                <span class="text-slate-400">忽略 {{ decisions.length - visibleRows.length }} 行（已隐藏）</span>
                <span v-if="problemRowCount > 0" class="text-amber-600 font-medium">
                  问题 {{ problemRowCount }} 行
                  <button @click="batchSetProblemRows('ignore')" class="ml-2 text-[10px] text-amber-600 hover:text-amber-800 underline">全部忽略</button>
                </span>
              </div>
              <!-- 行表格 -->
              <div class="flex-1 overflow-y-auto">
                <table class="w-full text-[11px]">
                  <thead class="bg-slate-50 sticky top-0 z-10">
                    <tr>
                      <th class="px-2 py-1.5 text-left font-medium text-slate-500 w-10">#</th>
                      <th v-for="ci in nonEmptyCols" :key="ci"
                        class="px-2 py-1.5 text-left font-medium text-slate-500"
                        :class="mapping[ci] ? 'text-brand-600' : 'text-surface-300'">
                        {{ activeSheet.headers[ci] || '(空)' }}
                      </th>
                      <th class="px-2 py-1.5 text-left font-medium text-slate-500 w-28">操作</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-surface-50">
                    <tr v-for="{ row, ri } in visibleRows" :key="ri"
                      class="hover:bg-slate-50/50 transition-colors"
                      :class="{ 'bg-amber-50/60': rowActions.find(a => a.key === decisions[ri]?.action)?.isProblem }">
                      <td class="px-2 py-1.5 text-surface-400">{{ row.rowNumber }}</td>
                      <td v-for="ci in nonEmptyCols" :key="ci"
                        class="px-2 py-1.5 max-w-[150px] truncate"
                        :class="mapping[ci] ? 'text-surface-800' : 'text-surface-300'">
                        {{ row.cells[ci] || '' }}
                      </td>
                      <td class="px-2 py-1.5">
                        <select v-model="decisions[ri].action"
                          class="w-full h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none focus:border-brand-400 transition-colors"
                          :class="rowActions.find(a => a.key === decisions[ri]?.action)?.isProblem ? 'border-amber-300 text-amber-700' : ''">
                          <option v-for="a in rowActions" :key="a.key" :value="a.key">{{ a.label }}</option>
                        </select>
                        <input v-if="decisions[ri]?.action === 'subnet'"
                          v-model="decisions[ri].subnetName"
                          placeholder="子网名称"
                          class="mt-1 w-full h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none focus:border-brand-400 transition-colors"
                        />
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </template>

          </div>

          <!-- Footer -->
          <div class="px-6 py-3 border-t border-gray-100 flex justify-between items-center shrink-0 bg-white">
            <div>
              <button v-if="step > 1" @click="prevStep" class="btn-secondary text-xs">
                ← 上一步
              </button>
            </div>
            <div class="flex items-center gap-2">
              <button @click="close" class="btn-secondary text-xs">取消</button>
              <button v-if="step === 1" @click="nextStep" :disabled="!canNextFromStep1" class="btn-primary text-xs">
                下一步 →
              </button>
              <button v-else-if="step === 2" @click="nextStep" :disabled="!canNextFromStep2" class="btn-primary text-xs">
                下一步 →
              </button>
              <button v-else-if="step === 3" @click="confirmImport" class="btn-primary text-xs" :disabled="validRowCount === 0">
                确认导入 {{ validRowCount }} 行
              </button>
            </div>
          </div>

        </div>
      </div>
    </Transition>
  </Teleport>
</template>
