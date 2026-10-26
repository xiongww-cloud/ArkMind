<script setup lang="ts">
import { ref, reactive, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { exportToPMSExcel, buildItemsFromMapping } from '@/utils/solutionExcel'
import { detectSCTSheet, buildSCTMapping, classifySCTRow, shouldSkipSCTRow } from '@/utils/sctExcel'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { useProductStore } from '@/stores/product'
import { toast } from '@/composables/useToast'
import { confirm, dangerConfirm, alert } from '@/composables/useConfirm'
import AppIcon from '@/components/common/AppIcon.vue'
import ExcelImportWizard from '@/components/shared/ExcelImportWizard.vue'
import ServiceFeeAllocator from '@/components/project/ServiceFeeAllocator.vue'
import DeviceTable from '@/components/project/DeviceTable.vue'
import type {
  ExcelImportPreset,
  PresetLaunchPayload
} from '@/components/shared/ExcelImportWizard.vue'
import type { ImportField, RawDataRow, SheetData } from '@/utils/excelRaw'

import * as api from '@/api/db-client'
import { formatDateTime } from '@/utils/time'
const { updateSolutionVersionMeta, fetchProducts, createProduct, deleteVersion } =
  api
const props = defineProps({ project: { type: Object, required: true } })
const projectStore = useProjectStore()
const productStore = useProductStore()
const authStore = useAuthStore()
const guid = () => Math.random().toString(36).substring(2, 11)
const gvid = () => 'ver-' + Date.now()
const op = computed(() => authStore.currentUser.name)

const gp = ref([]) // 产品列表 — 从后端 Product 表加载

const showImportWizard = ref(false)
// ─── 价格上浮 ───────────────────────────────────────────────────────────────
/** 调价系数（如 1.2），与 markupDivide 组合成有效倍数：×coef 或 ÷coef */
const markupCoef = ref(1.2)
const markupDivide = ref(false)
const showMarkupInput = ref(false)
// ─── SCT 服务费分摊模式 ─────────────────────────────────────────────────────
const showSCTAllocator = ref(false)
const sctSheet = ref<SheetData | null>(null)
const sctAllocator = ref<InstanceType<typeof ServiceFeeAllocator> | null>(null)

// ─── Excel 向导导入 ──────────────────────────────────────────────────────────
const solutionImportFields: ImportField[] = [
  { key: 'sn', label: '序号', guesses: ['序号', '编号', 'No', 'no', '#'] },
  { key: 'deviceName', label: '设备名称', guesses: ['设备名', '设备名称', '名称'] },
  {
    key: 'model',
    label: '产品型号',
    guesses: ['型号', '产品型号', 'Model', 'model', '产品名称']
  },
  {
    key: 'params',
    label: '产品参数',
    guesses: ['参数', '产品参数', '规格', 'Specs', 'specs', '描述']
  },
  {
    key: 'qty',
    label: '数量',
    guesses: ['数量', '数', 'Qty', 'qty', '台数'],
    numeric: true,
    defaultHint: '1'
  },
  { key: 'unit', label: '单位', guesses: ['单位', 'Unit', 'unit'], defaultHint: '台' },
  {
    key: 'price',
    label: '单价',
    guesses: ['单价', 'Price', 'price', '含税单价'],
    numeric: true,
    defaultHint: '0'
  }
]

/** 华为 SCT 报价单预设：自动识别 → 交给 ServiceFeeAllocator 分摊模式 */
const sctPreset: ExcelImportPreset = {
  label: '华为 SCT',
  detect: detectSCTSheet,
  buildMapping: buildSCTMapping,
  classify: classifySCTRow,
  shouldSkip: shouldSkipSCTRow
}

function classifySolutionRow(
  row: RawDataRow,
  mapping: (string | undefined)[],
  sheet: SheetData
): string | null {
  // 查映射后的型号列
  const modelIdx = mapping.findIndex((k) => k === 'model')
  const modelVal = modelIdx >= 0 ? (row.cells[modelIdx] || '').trim() : ''
  // 检查是否为合计/总计行（任意单元格含关键词）
  const hasTotal = row.cells.some((c) => c.includes('合计') || c.includes('总计'))
  if (hasTotal) return 'ignore'
  // 检查是否为子网行：序号列纵向合并的主单元格
  const snIdx = mapping.findIndex((k) => k === 'sn')
  if (snIdx >= 0) {
    const m = row.merges.find((mg) => mg.col === snIdx + 1 && mg.isMaster && mg.rowSpan > 1)
    if (m && !modelVal) return 'subnet'
  }
  // 有型号 → 设备
  if (modelVal) return 'device'
  // 无型号但行内有内容 → 仍作为设备导入（型号可选，后续可手动填写）
  const hasContent = row.cells.some((c) => c.trim().length > 0)
  if (hasContent) return 'device'
  // 空行
  return 'ignore'
}

function shouldSkipSolutionRow(row: RawDataRow, sheet: SheetData): boolean {
  // 全空行跳过
  return row.cells.every((c) => !c.trim())
}

function onImportConfirm(payload: any) {
  const { mapping, rows, decisions, sheet } = payload
  const productList = Array.isArray(gp.value) ? gp.value : []
  const items = buildItemsFromMapping(rows, mapping, decisions, sheet, productList)
  // 匹配产品库参数
  if (cplan.value?.draft) {
    cplan.value.draft.items = items.map((item: any) => {
      if (item.rowType === 'subnet' || !item.model) return item
      const match = productList.find(
        (p: any) => (p.name || '').toLowerCase() === item.model.toLowerCase()
      )
      if (match) {
        return {
          ...item,
          productId: match.id,
          // 产品库无参数时默认勾选"更新产品参数"，让本次导入的参数回写产品库
          syncParams: !match.full_specs,
          mergedText: item.mergedText || match.full_specs || ''
        }
      }
      return { ...item, syncParams: item.syncParams !== false }
    })
  }
  showImportWizard.value = false
}

/** SCT 识别后：关闭通用向导，打开服务费分摊模式 */
function onSCTLaunch(payload: PresetLaunchPayload) {
  showImportWizard.value = false
  sctSheet.value = payload.sheet
  showSCTAllocator.value = true
}

/** SCT 分摊确认：把硬件设备（含复合单价）注入设备清单 */
function handleServiceFeeConfirm(result: any) {
  const { hardwareItems } = result
  const productList = Array.isArray(gp.value) ? gp.value : []
  if (!cplan.value?.draft) {
    showSCTAllocator.value = false
    return
  }
  const items = hardwareItems.map((hw: any) => {
    const model = hw.cleanModel || hw.rawName
    const match = productList.find(
      (p: any) => (p.name || '').toLowerCase() === (model || '').toLowerCase()
    )
    return {
      uid: guid(),
      productId: match?.id || '',
      model,
      customName: hw.customName || '',
      unitPrice: hw.compositePrice || hw.unitPrice || 0,
      quantity: hw.quantity || 1,
      unit: hw.unit || '台',
      params: [],
      mergedText: match?.full_specs || '',
      syncParams: match ? false : true,
      _originalUnitPrice: hw.unitPrice || 0,
      _allocatedAmount: hw.allocatedAmount || 0
    }
  })
  cplan.value.draft.items = [...cplan.value.draft.items, ...items]
  showSCTAllocator.value = false
  sctSheet.value = null
}

function handleServiceFeeCancel() {
  showSCTAllocator.value = false
  sctSheet.value = null
}

const plans = ref([])
const apid = ref('')
function iplans() {
  if (!props.project.plans || props.project.plans.length === 0) {
    // 默认方案使用确定性 id（含项目 id），确保草稿 key 跨刷新稳定
    const defaultPlanId = 'default-plan-' + props.project.id
    props.project.plans = [
      {
        id: defaultPlanId,
        name: '默认设备清单',
        items: [],
        history: [],
        draft: null,
        createdAt: new Date().toISOString()
      }
    ]
  }
  plans.value = props.project.plans
  if (!apid.value || !plans.value.find((p) => p.id === apid.value))
    apid.value = plans.value[0]?.id || ''
}
iplans()
const lvt = (plan) => {
  const h = Array.isArray(plan.history) ? plan.history : []
  if (h.length) return Math.max(...h.map((v) => new Date(v.createdAt).getTime()))
  return new Date(plan.createdAt || 0).getTime()
}
const splans = computed(() => [...plans.value].sort((a, b) => lvt(b) - lvt(a)))
const cplan = computed(() => plans.value.find((p) => p.id === apid.value))
const cpname = computed(() => cplan.value?.name || '')

const av = ref('timeline')
const vvi = ref(null)
const im = computed(() => av.value === 'edit')
const ir = computed(() => av.value === 'view')
const vv = computed(() => {
  if (ir.value && vvi.value !== null) return cplan.value?.history[vvi.value] || null
  return null
})
/** 当前编辑中草稿（总价栏徽标用；any 逃逸 cplan 的 never 推断类历史噪音） */
const curDraft = computed<any>(() => ((cplan.value as any) || {}).draft || null)

const si = computed(() => {
  if (im.value && cplan.value?.draft) {
    const d = cplan.value.draft.items
    return Array.isArray(d) ? d : []
  }
  if (ir.value && vvi.value !== null) {
    const h = cplan.value?.history[vvi.value]?.items
    if (!Array.isArray(h)) return []
    const allProducts = Array.isArray(gp.value) ? gp.value : []
    return h.map((item) => {
      if (item.rowType === 'subnet' || item.productId) return item
      const model = (item.model || '').trim()
      if (!model) return item
      const matched = allProducts.find((p) => (p.name || '').toLowerCase() === model.toLowerCase())
      if (!matched) return item
      return {
        ...item,
        productId: matched.id,
        model: matched.name,
        unitPrice: item.unitPrice || matched.price || 0,
        mergedText: item.mergedText || matched.full_specs || ''
      }
    })
  }
  return []
})
const li = computed({
  get: () => si.value,
  set: (v) => {
    if (im.value && cplan.value?.draft) cplan.value.draft.items = v
  }
})
const gt = computed(() =>
  li.value.reduce(
    (s, i) => (i.rowType === 'subnet' ? s : s + (i.unitPrice || 0) * (i.quantity || 0)),
    0
  )
)
/** 编辑中草稿的金额（从 draft.items 直接计算，独立于当前视图） */
const draftAmount = computed(() => {
  const items = Array.isArray(cplan.value?.draft?.items) ? cplan.value.draft.items : []
  return items.reduce(
    (s, i) => (i.rowType === 'subnet' ? s : s + (i.unitPrice || 0) * (i.quantity || 0)),
    0
  )
})
const deviceCount = computed(() => li.value.filter((i) => i.rowType !== 'subnet').length)

const ste = computed(() => {
  if (!cplan.value) return []
  const e = []
  if (cplan.value.draft) e.push({ type: 'draft', data: cplan.value.draft })
  const history = cplan.value.history
  if (Array.isArray(history)) history.forEach((v, i) => e.push({ type: 'ver', data: v, idx: i }))
  return e
})

// ─── 时间线选中态（点卡片仅选中，查看/编辑需点按钮）────────────────────────
const selEntryKey = ref(null)
const selEntry = computed(() => {
  const entries = ste.value
  if (!entries.length) return null
  if (selEntryKey.value) {
    const hit = entries.find((en: any) =>
      en.type === 'draft' ? selEntryKey.value === 'draft' : (en.data.versionId || en.data.id) === selEntryKey.value
    )
    if (hit) return hit
  }
  const draft = entries.find((en: any) => en.type === 'draft')
  return draft || entries[0]
})
function selectEntry(en: any) {
  selEntryKey.value = en.type === 'draft' ? 'draft' : en.data.versionId || en.data.id
}

/** 选中版本的报价概览 */
const selOverview = computed(() => {
  const en = selEntry.value
  if (!en) return null
  const items = Array.isArray(en.data.items) ? en.data.items : []
  const devices = items.filter((i: any) => i.rowType !== 'subnet')
  const subnets = items.length - devices.length
  const amount = en.type === 'draft' ? draftAmount.value : (en.data.amount || 0)
  const totalUnits = devices.reduce((s: number, d: any) => s + (Number(d.quantity) || 0), 0)
  return {
    status: en.type === 'draft' ? '编辑中（草稿）' : '已归档',
    devices: devices.length,
    totalUnits,
    subnets,
    amount
  }
})

/** 概览：按型号聚合的设备清单（默认收起） */
const overviewExpanded = ref(false)
watch(selEntryKey, () => {
  overviewExpanded.value = false
})
const selModelSummary = computed(() => {
  const en = selEntry.value
  if (!en) return []
  const items = Array.isArray(en.data.items) ? en.data.items : []
  const map = new Map()
  for (const it of items) {
    if (it.rowType === 'subnet') continue
    const key = String(it.model || it.customName || '未指定型号').trim() || '未指定型号'
    map.set(key, (map.get(key) || 0) + (Number(it.quantity) || 0))
  }
  return [...map.entries()].map(([model, qty]) => ({ model, qty }))
})

/** 概览：变更说明（多行文本，失焦保存） */
const remarkVal = ref('')
watch(
  selEntry,
  (en) => {
    remarkVal.value = en?.data?.content || ''
  },
  { immediate: true }
)
async function saveSelRemark() {
  const en = selEntry.value
  if (!en) return
  const val = remarkVal.value.trim()
  if (en.type === 'draft') {
    en.data.content = val
    return
  }
  if (val === (en.data.content || '')) return
  en.data.content = val
  const vid = en.data.versionId || en.data.id
  if (vid) {
    try {
      await updateSolutionVersionMeta(vid, { content: val })
    } catch (e) {
      console.warn('更新版本备注失败', e)
    }
  }
}

/** 工具栏添加设备：插入根级区域末尾（第一个子网行之前），保证不落入任何子网 */
function apl() {
  if (ir.value) return
  const firstSubnet = li.value.findIndex((i) => i.rowType === 'subnet')
  const at = firstSubnet === -1 ? li.value.length : firstSubnet
  li.value.splice(at, 0, {
    uid: guid(),
    rowType: 'device',
    productId: '',
    model: '',
    customName: '',
    unitPrice: 0,
    quantity: 1,
    unit: '台',
    params: [],
    mergedText: '',
    syncParams: true
  })
  if (firstSubnet !== -1) {
    toast({ message: '已添加到根级清单（顶部区域）', type: 'success' })
  }
}
/** 工具栏添加子网：追加为新的空子网（清单末尾） */
function asub() {
  if (ir.value) return
  li.value.splice(li.value.length, 0, { uid: guid(), rowType: 'subnet', name: '新子网' })
}

const gpn = (id) =>
  (Array.isArray(gp.value) ? gp.value.find((p) => p.id === id) : undefined)?.name || ''

/** 徽标显示：有效倍数 ≥1 显示 ×N；<1 显示 ÷N（1/倍数），如 ÷1.2 */
function fmtMarkup(m: number): string {
  const strip = (s: string) => s.replace(/0+$/, '').replace(/\.$/, '')
  return m >= 1 ? `×${strip(m.toFixed(2))}` : `÷${strip((1 / m).toFixed(2))}`
}

/** 应用上浮：以原价基线重算（无快照先快照），改系数不复利；系数随版本持久化 */
function applyMarkup() {
  const coef = markupCoef.value
  if (!coef || coef <= 0) {
    showMarkupInput.value = false
    return
  }
  const draft: any = (cplan.value as any)?.draft
  if (!draft) return
  const multiplier = markupDivide.value ? 1 / coef : coef
  let touched = 0
  for (const item of draft.items || []) {
    if (item.rowType === 'subnet') continue
    if (item.originalPrice == null) item.originalPrice = item.unitPrice || 0
    item.unitPrice = Math.round(item.originalPrice * multiplier * 100) / 100
    touched++
  }
  draft.priceMarkup = multiplier
  const expr = `${markupDivide.value ? '÷' : '×'}${coef}`
  toast({ message: `已对 ${touched} 台设备 ${expr} 调价（以原价为基线，可随时还原）`, type: 'success' })
  showMarkupInput.value = false
}

/** 去小数：所有设备单价向上取整（一次性变换，不入原价基线；可撤销） */
function ceilPrices() {
  const draft: any = (cplan.value as any)?.draft
  if (!draft) return
  let n = 0
  for (const item of draft.items || []) {
    if (item.rowType === 'subnet') continue
    const p = Number(item.unitPrice) || 0
    const c = Math.ceil(p)
    if (c !== p) {
      item.unitPrice = c
      n++
    }
  }
  toast({ message: n ? `已对 ${n} 台设备单价向上取整` : '单价均已是整数', type: 'success' })
}

/** 修改上浮：复开系数输入并预填当前有效倍数（统一按 × 形式回填） */
function openMarkupEdit() {
  const draft: any = (cplan.value as any)?.draft
  if (draft?.priceMarkup && Math.abs(draft.priceMarkup - 1) > 0.004) {
    markupCoef.value = Math.round(draft.priceMarkup * 100) / 100
    markupDivide.value = false
  }
  showMarkupInput.value = true
}

/** 还原原价：有快照的行恢复基线价，系数归 1（快照保留，可反复上浮/还原） */
function restorePrices() {
  const draft: any = (cplan.value as any)?.draft
  if (!draft) return
  let restored = 0
  for (const item of draft.items || []) {
    if (item.rowType === 'subnet') continue
    if (item.originalPrice != null) {
      item.unitPrice = item.originalPrice
      restored++
    }
  }
  draft.priceMarkup = 1
  toast({ message: restored ? `已还原 ${restored} 台设备的原价` : '没有可还原的原价快照', type: 'success' })
}

const smm = ref(false)
const isSaving = ref(false)
const saveError = ref('')
const mf = reactive({ cm: '' })
function fmtVersion(v) {
  if (!v) return ''
  return v.replace(/^V/, 'v')
}
/** 前端 items → 后端 DeviceItemInput 格式 */
function mapItemsToBackend(itemsArr) {
  return (Array.isArray(itemsArr) ? itemsArr : []).map((item) => {
    if (item.rowType === 'subnet') {
      return {
        rowType: 'subnet',
        name: item.name || '',
        deviceName: '',
        model: '',
        parameters: '',
        count: 0,
        unit: '',
        price: 0,
        paramMarks: []
      }
    }
    return {
      rowType: 'device',
      name: '',
      deviceName: item.customName || '',
      model: item.model || gpn(item.productId),
      productId: item.productId || null,
      parameters: item.mergedText || '',
      count: item.quantity || 1,
      unit: item.unit || '台',
      price: item.unitPrice || 0,
      originalPrice: item.originalPrice ?? null,
      // Persist selected parameter marks for read-only mode
      paramMarks: Array.isArray(item.params)
        ? item.params.map((p) => ({
            paramId: p.id,
            mark: p.mark || '',
            indicator: p.indicator || '',
            requirement: p.requirement || ''
          }))
        : []
    }
  })
}
const invalidItems = ref([]) // uids of items with validation errors
const dt = ref<any>(null) // DeviceTable ref：lintAll / fixAll / stripControlAll / pullAllEmpty

/** 删除控标内容（破坏性批操作，先确认；撤销可恢复） */
async function stripControlMarks() {
  const ok = await dangerConfirm({
    title: '删除控标内容',
    message:
      '将对全部设备参数执行：\n· 剥除 ★/▲ 控标符号（保留参数行）\n· 删除含「截图 / 证明 / 报告 / 盖章」的证明材料段（按标点切段）\n· 只删符号与小段文本，不会删除整条参数\n· 删除后自动修复标点与序号\n\n可通过撤销（Ctrl+Z）恢复。',
    confirmText: '删除'
  })
  if (!ok) return
  dt.value?.stripControlAll()
}
function validateItems(items) {
  if (!items || !Array.isArray(items) || items.length === 0) return []
  return items.filter((item) => {
    if (item.rowType === 'subnet') return false
    const noDevice = !item.productId && !item.model
    const noPrice = !item.unitPrice || item.unitPrice <= 0
    return noDevice || noPrice
  })
}
function clearValidation() {
  invalidItems.value = []
}
async function omm() {
  if (!cplan.value) return
  if (cplan.value.draft) {
    alert({
      title: '已有编辑中的版本',
      message: `请先保存或删除版本 ${fmtVersion(cplan.value.draft.versionNum) || ''} 再新建`
    })
    return
  }
  const vn = 'v' + (cplan.value.history.length + 1) + '.0'
  const rawSrc =
    av.value === 'view' && vvi.value !== null && cplan.value.history[vvi.value]
      ? cplan.value.history[vvi.value].items
      : cplan.value.items
  const srcItems = Array.isArray(rawSrc) ? rawSrc : []
  const allProducts = Array.isArray(gp.value) ? gp.value : []
  const copiedItems = JSON.parse(JSON.stringify(srcItems)).map((item) => {
    if (item.rowType === 'subnet') return { ...item, syncParams: false }
    const model = (item.model || '').trim()
    const matched = allProducts.find((p) => (p.name || '').toLowerCase() === model.toLowerCase())
    const hasProduct = !!(matched?.id || item.productId)
    return {
      ...item,
      syncParams: hasProduct ? false : true,
      productId: matched?.id || item.productId || '',
      model: matched ? matched.name : item.model,
      unitPrice: !item.unitPrice && matched?.price ? matched.price : item.unitPrice,
      mergedText: !item.mergedText && matched?.full_specs ? matched.full_specs : item.mergedText
    }
  })
  // 先写库（status='draft'），失败则回退为纯本地草稿
  let versionId = ''
  try {
    const r = await projectStore.createPlanDraft(props.project.id, cplan.value.id, {
      versionNum: vn,
      content: '',
      tags: [],
      operator: op.value,
      deviceItems: mapItemsToBackend(copiedItems)
    })
    versionId = r?.id || ''
  } catch (e) {
    console.warn('[Solution] 创建草稿版本失败，降级为本地草稿:', e)
  }
  cplan.value.draft = {
    versionNum: vn,
    tags: [],
    content: '',
    items: copiedItems,
    startedAt: new Date().toISOString(),
    operator: op.value,
    priceMarkup:
      av.value === 'view' && vvi.value !== null
        ? cplan.value.history[vvi.value]?.priceMarkup || 1
        : 1,
    _versionId: versionId || undefined
  }
  av.value = 'edit'
  clearValidation()
}
/** 新增空白方案版本（空清单，版本号递增） */
async function ommBlank() {
  if (!cplan.value) return
  if (cplan.value.draft) {
    alert({
      title: '已有编辑中的版本',
      message: `请先保存或删除版本 ${fmtVersion(cplan.value.draft.versionNum) || ''} 再新建`
    })
    return
  }
  const vn = 'v' + (cplan.value.history.length + 1) + '.0'
  let versionId = ''
  try {
    const r = await projectStore.createPlanDraft(props.project.id, cplan.value.id, {
      versionNum: vn,
      content: '',
      tags: [],
      operator: op.value,
      deviceItems: []
    })
    versionId = r?.id || ''
  } catch (e) {
    console.warn('[Solution] 创建草稿版本失败，降级为本地草稿:', e)
  }
  cplan.value.draft = {
    versionNum: vn,
    tags: [],
    content: '',
    items: [],
    startedAt: new Date().toISOString(),
    operator: op.value,
    priceMarkup: 1,
    _versionId: versionId || undefined
  }
  av.value = 'edit'
  clearValidation()
}
function findUnknownDevices(items) {
  if (!items || !Array.isArray(items)) return []
  return items.filter((item) => {
    if (item.rowType === 'subnet') return false
    if (!item.productId && item.model && item.model.trim()) return true
    if (
      item.productId &&
      !(Array.isArray(gp.value) ? gp.value : []).find((p) => p.id === item.productId)
    )
      return true
    return false
  })
}

async function preSave() {
  if (!cplan.value?.draft) return
  const draftItems = Array.isArray(cplan.value?.draft?.items) ? cplan.value.draft.items : []
  const bad = validateItems(draftItems)
  if (bad.length > 0) {
    invalidItems.value = bad.map((i) => i.uid)
    saveError.value = '保存失败：清单中存在未选择设备或未填写金额的条目，请完善后再试'
    return
  }
  invalidItems.value = []
  saveError.value = ''

  // 检测未知设备（产品库中不存在的型号）
  const unknown = findUnknownDevices(draftItems)
  if (unknown.length > 0) {
    const uniqueModels = Array.from(new Set(unknown.map((i) => i.model?.trim()).filter(Boolean)))
    const names = uniqueModels.join('、')
    const ok = await confirm({
      title: '发现未知设备型号',
      message: `清单中的「${names}」不在产品库中。是否添加到产品库后继续保存？`,
      confirmText: '是',
      cancelText: '否'
    })
    if (ok) {
      try {
        const createdMap = {} // modelLower → productId
        for (const model of uniqueModels) {
          // 创建产品时带上该条目已填写的技术参数，避免产品库只有型号没有参数
          const firstItem = unknown.find(
            (i) => i.model?.trim().toLowerCase() === model.toLowerCase()
          )
          const specs = firstItem?.mergedText || ''
          const created = await createProduct({ name: model, fullSpecs: specs })
          if (created?.id) {
            createdMap[model.toLowerCase()] = created.id
            if (!(Array.isArray(gp.value) ? gp.value : []).find((p) => p.id === created.id)) {
              gp.value.push({ ...created, name: created.name || created.model || model })
            }
          }
        }
        for (const item of unknown) {
          const key = item.model?.trim().toLowerCase()
          if (createdMap[key]) item.productId = createdMap[key]
        }
      } catch (e) {
        saveError.value = '添加设备到产品库失败：' + (e.message || '网络异常')
        return
      }
    }
  }

  mf.cm = cplan.value.draft.content || ''
  smm.value = true
}
function confirmSave() {
  if (!cplan.value?.draft) return
  cplan.value.draft.content = mf.cm.trim()
  smm.value = false
  sdv()
}
async function sdv() {
  if (!cplan.value?.draft || isSaving.value) return
  const d = cplan.value.draft
  isSaving.value = true
  saveError.value = ''

  try {
    // Auto-match devices to product catalog before saving
    const itemsArr = Array.isArray(d.items) ? d.items : []
    const allProducts = Array.isArray(gp.value) ? gp.value : []
    for (const item of itemsArr) {
      if (item.rowType === 'subnet') continue
      const model = (item.model || '').trim()
      if (!model) continue
      const match = allProducts.find((p) => (p.name || '').toLowerCase() === model.toLowerCase())
      if (match) {
        item.productId = match.id
        item.model = match.name
        if (match.price && !item.unitPrice) item.unitPrice = match.price
        if (match.full_specs && !item.mergedText) item.mergedText = match.full_specs
      }
    }

    // Map frontend items to backend DeviceItemInput format
    const deviceItems = mapItemsToBackend(itemsArr)

    // Call the real backend API via the store
    let saveErr = null
    let r = null
    try {
      r = await projectStore.savePlanVersion(props.project.id, cplan.value.id, {
        versionNum: d.versionNum,
        tags: [],
        content: d.content || '',
        operator: d.operator || op.value,
        priceMarkup: d.priceMarkup || 1,
        deviceItems,
        _versionId: d._versionId,
        // Pass original draft items so the store can preserve productId & params
        _originItems: itemsArr
      })
    } catch (e) {
      saveErr = e
      console.error('[保存版本] savePlanVersion 失败:', e)
    }
    if (!saveErr && r) {
      // 映射后端字段名 → 前端字段名（与 fetchProjectDetail 中的映射一致）
      if (r.items && Array.isArray(r.items)) {
        r.items = r.items.map((di) => ({
          uid: di.id,
          productId: di.productId || '',
          rowType: di.rowType || 'device',
          name: di.name || '',
          model: di.rowType === 'subnet' ? '' : di.model || '',
          customName: di.rowType === 'subnet' ? '' : di.deviceName || '',
          unitPrice: di.rowType === 'subnet' ? 0 : di.price || 0,
          originalPrice: di.rowType === 'subnet' ? null : (di.originalPrice ?? null),
          quantity: di.rowType === 'subnet' ? 0 : di.count || 1,
          unit: di.rowType === 'subnet' ? '' : di.unit || '台',
          mergedText: di.rowType === 'subnet' ? '' : di.parameters || '',
          params: []
        }))
      }
      cplan.value.history = [r, ...(cplan.value.history || [])]
    }

    // Sync: update product parameters for items with syncParams enabled (best-effort)
    // 走 store.updateProduct（API+缓存双写）——裸 api 调用会让产品中心读着旧缓存显示"没起作用"
    const syncItems = itemsArr.filter(
      (item) => item.rowType !== 'subnet' && item.syncParams && item.productId && item.mergedText
    )
    if (syncItems.length > 0) {
      const results = await Promise.allSettled(
        syncItems.map((item) => productStore.updateProduct(item.productId, { fullSpecs: item.mergedText }))
      )
      const failed = results.filter((r) => r.status === 'rejected')
      if (failed.length > 0) {
        failed.forEach((f: any) => console.warn('[保存] 同步产品参数失败:', f.reason))
        toast({ message: `${failed.length}/${syncItems.length} 个产品参数同步失败`, type: 'warning' })
      }
    }

    // Record dynamic for new version (best-effort)
    try {
      await projectStore.addProjectDynamic(
        props.project.id,
        `为设备清单「${cplan.value.name}」保存了版本 ${d.versionNum || 'V1.0'}`
      )
    } catch (e) {
      console.warn('[保存] 记录动态失败', e)
    }

    if (saveErr) throw saveErr

    // On success: keep current items as read-only copy, clear draft, return to timeline
    try {
      cplan.value.items = JSON.parse(JSON.stringify(d.items))
      cplan.value.draft = null
      clearDraft(cplan.value.id)
    } catch (e) {
      console.warn('[保存] 清理草稿失败', e)
    }
    gtl()
  } catch (e) {
    saveError.value = e.message || '保存失败，请重试'
    alert({ title: '保存失败', message: e.message || '网络异常，请检查后端服务是否启动' })
    console.error('版本存库失败', e)
  } finally {
    isSaving.value = false
  }
}

function gtl() {
  av.value = 'timeline'
  vvi.value = null
  clearValidation()
}
async function cancelDraft() {
  if (!cplan.value) return
  const ok = await dangerConfirm({
    title: '删除编辑中的版本',
    message: `确定删除正在编辑的版本 ${fmtVersion(cplan.value.draft?.versionNum) || ''}？未保存的修改将丢失，此操作不可撤销。`,
    confirmText: '删除'
  })
  if (!ok) return
  const pid = cplan.value.id
  const vid = cplan.value.draft?._versionId
  cplan.value.draft = null
  clearDraft(pid)
  if (vid) {
    try {
      await deleteVersion(vid)
    } catch (e) {
      console.warn('[Solution] 删除草稿版本失败:', e)
    }
  }
  gtl()
}
function vhv(vi) {
  vvi.value = vi
  av.value = 'view'
}
/** 概览栏「编辑」：基于选中归档版本新建草稿进入编辑（复用 omm 的拷贝逻辑） */
function editFromOverview() {
  const en = selEntry.value
  if (!en || en.type !== 'ver') return
  vvi.value = en.idx
  av.value = 'view' // omm 从 view 态取源版本 items
  omm()
}
function sed() {
  av.value = 'edit'
  clearValidation()
}

async function evx(ver) {
  if (!ver) return
  const pn = cplan.value?.name || '设备清单'
  const prjn = props.project?.name || '项目'
  const its = Array.isArray(ver.items) ? ver.items : []

  // Pre-resolve product names from store (exportToPMSExcel only uses item.model)
  const resolved = its.map((item) => ({
    ...item,
    model: item.model || gpn(item.productId) || ''
  }))

  await exportToPMSExcel({
    projectName: prjn,
    planName: pn,
    versionLabel: ver.versionNum || '快照',
    totalAmount: ver.amount || 0,
    items: resolved
  })
}

async function deleteVersionEntry(entry) {
  const ok = await dangerConfirm({
    title: '删除版本',
    message: `确定删除版本 ${fmtVersion(entry.data.versionNum)}？此操作不可撤销。`,
    confirmText: '删除'
  })
  if (!ok) return
  const verId = entry.data.versionId || entry.data.id
  if (!verId) return
  try {
    await deleteVersion(verId)
    const plan = plans.value.find((p) => p.id === apid.value)
    if (plan && Array.isArray(plan.history)) {
      plan.history = plan.history.filter((h) => (h.versionId || h.id) !== verId)
    }
  } catch (e) {
    console.warn('删除版本失败', e)
  }
}

const rpid = ref(null)
const rv = ref('')
let planNameInput = null
async function anp() {
  const planId = projectStore.addPlan(props.project.id, '新设备清单')
  if (planId) {
    plans.value = props.project.plans
    apid.value = planId
    await projectStore.addProjectDynamic(props.project.id, '创建了设备清单「新设备清单」')
    gtl()
  }
}
function srp(plan) {
  rpid.value = plan.id
  rv.value = plan.name
  nextTick(() => {
    if (planNameInput && typeof planNameInput.focus === 'function') {
      planNameInput.focus()
      planNameInput.select()
    }
  })
}
function crp() {
  const plan = plans.value.find((p) => p.id === rpid.value)
  if (plan && rv.value.trim() && rv.value.trim() !== plan.name) {
    projectStore.updatePlan(props.project.id, plan.id, { name: rv.value.trim() })
  }
  rpid.value = null
}
function crpc() {
  rpid.value = null
}
async function rmp(id) {
  if (plans.value.length <= 1) return
  const plan = plans.value.find((p) => p.id === id)
  if (!plan) return
  const vc = Array.isArray(plan.history) ? plan.history.length : 0
  const hasDraft = !!plan.draft
  const parts = []
  if (vc > 0) parts.push(`${vc} 个历史版本`)
  if (hasDraft) parts.push('1 份草稿')
  const detail = parts.length ? `该设备清单包含 ${parts.join(' 和 ')}，` : ''
  const ok = await dangerConfirm({
    title: '删除设备清单',
    message: `${detail}删除后数据将无法恢复。确定删除设备清单「${plan.name}」？`,
    confirmText: '删除'
  })
  if (!ok) return
  if (apid.value === id) apid.value = plans.value[0]?.id || ''
  clearDraft(id)
  projectStore.deletePlan(props.project.id, id)
}

watch(apid, () => {
  selEntryKey.value = null
  gtl()
})
watch(
  li,
  () => {
    if (im.value && invalidItems.value.length) clearValidation()
  },
  { deep: true }
)

// ─── 编辑态实时自动保存（防刷新丢失）──────────────────────────────────────────
const DRAFT_PREFIX = 'pms_draft_'
function draftKey(planId) {
  return DRAFT_PREFIX + props.project.id + '_' + planId
}

/**
 * 静默保存草稿到 localStorage（防刷新/崩溃丢失）
 * 仅序列化当前 draft 对象，不触发后端 API
 */
const saveDraft = () => {
  const draft = cplan.value?.draft
  if (!draft) return
  // 主：写库（status='draft'，已有 _versionId 时）
  if (draft._versionId) {
    projectStore.savePlanDraft(props.project.id, cplan.value.id, {
      _versionId: draft._versionId,
      versionNum: draft.versionNum,
      content: draft.content || '',
      tags: [],
      priceMarkup: draft.priceMarkup || 1,
      deviceItems: mapItemsToBackend(draft.items)
    })
  }
  // 兜底：同时写 localStorage（离线/降级场景）
  try {
    const key = draftKey(cplan.value.id)
    const payload = JSON.parse(JSON.stringify(draft))
    localStorage.setItem(key, JSON.stringify(payload))
  } catch {
    /* quota exceeded */
  }
}

function clearDraft(planId) {
  try {
    localStorage.removeItem(draftKey(planId))
  } catch (e) {
    console.error('[Solution] clearDraft failed:', e)
  }
}

function restoreDrafts() {
  plans.value.forEach((plan) => {
    try {
      // 主：DB 已恢复的草稿（fetchProjectDetail 按 status 分流）优先
      if (plan.draft) {
        const d = plan.draft
        if (d.priceMarkup == null) d.priceMarkup = 1
        if (Array.isArray(d.items)) {
          d.items.forEach((item) => {
            if (item.syncParams === undefined) item.syncParams = item.productId ? false : true
          })
        }
        return
      }
      // 兜底：localStorage 草稿
      const raw = localStorage.getItem(draftKey(plan.id))
      if (!raw) return
      const draft = JSON.parse(raw)
      if (!draft || !Array.isArray(draft.items) || draft.items.length === 0) {
        clearDraft(plan.id)
        return
      }
      // 只恢复未正式保存过的草稿（plan.history 中没有该版本号的已归档版本）
      const alreadyArchived = (Array.isArray(plan.history) ? plan.history : []).some(
        (h) => h.versionNum === draft.versionNum
      )
      if (alreadyArchived) {
        clearDraft(plan.id)
        return
      }
      draft.items.forEach((item) => {
        if (item.syncParams === undefined) item.syncParams = item.productId ? false : true
      })
      plan.draft = draft
    } catch {
      /* corrupted */
    }
  })
}

// ─── 防抖工具（自实现，无需外部依赖）─────────────────────────────────────────
function debounce(fn, delay = 300) {
  let timer = null
  const debounced = (...args) => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => {
      timer = null
      fn(...args)
    }, delay)
  }
  debounced.cancel = () => {
    if (timer) {
      clearTimeout(timer)
      timer = null
    }
  }
  // 立即执行待写入任务（用于卸载/页面离开前冲刷）
  debounced.flush = () => {
    if (timer) {
      clearTimeout(timer)
      timer = null
      fn()
    }
  }
  return debounced
}

const debouncedSaveDraft = debounce(saveDraft, 300)

// 监听草稿变更 → 300ms 防抖后写入 localStorage
watch(
  () => cplan.value?.draft,
  () => {
    debouncedSaveDraft()
  },
  { deep: true }
)

// 页面刷新/关闭/切后台前同步冲刷草稿到 localStorage，避免丢失
function flushDraftOnUnload() {
  debouncedSaveDraft.flush()
}

onMounted(async () => {
  window.addEventListener('pagehide', flushDraftOnUnload)
  restoreDrafts()
  try {
    const raw = await fetchProducts()
    const products = Array.isArray(raw)
      ? raw
      : Array.isArray(raw?.data)
        ? raw.data
        : Array.isArray(raw?.items)
          ? raw.items
          : []
    gp.value = products.map((p) => ({ ...p, name: p.model || p.name }))
  } catch (e) {
    console.warn('[ProductCatalog] 加载产品库失败', e)
  }
})
onUnmounted(() => {
  window.removeEventListener('pagehide', flushDraftOnUnload)
  debouncedSaveDraft.flush()
  debouncedSaveDraft.cancel()
})
</script>

<template>
  <div class="flex h-full w-full bg-white overflow-hidden">
    <div
      v-if="av === 'timeline'"
      class="w-[260px] border-r border-surface-100 flex flex-col bg-white flex-shrink-0"
    >
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider"
          >设备清单</span
        >
        <button
          @click="anp"
          class="flex items-center justify-center w-5 h-5 rounded text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors"
          v-tooltip="'新增设备清单'"
        >
          <AppIcon name="plus" class="w-3.5 h-3.5" />
        </button>
      </div>
      <div
        class="flex-1 overflow-y-auto px-2 py-1.5 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full"
      >
        <div
          v-for="plan in splans"
          :key="plan.id"
          @click="apid = plan.id"
          class="relative flex items-center px-2 py-1.5 rounded-lg cursor-pointer transition-colors mb-0.5 group"
          :class="
            apid === plan.id
              ? 'bg-blue-50/70 text-brand-700 font-medium'
              : 'text-surface-600 hover:bg-surface-100'
          "
        >
          <div
            v-if="apid === plan.id"
            class="absolute left-0 top-1 bottom-1 w-[3px] bg-blue-500 rounded-l pointer-events-none"
          />
          <input
            v-if="rpid === plan.id"
            :ref="(el) => (planNameInput = el)"
            v-model="rv"
            @blur="crp"
            @keyup.enter="crp"
            @keyup.escape="crpc"
            @click.stop
            class="flex-1 min-w-0 text-xs px-1 py-0.5 border border-blue-400 rounded outline-none focus:ring-1 focus:ring-blue-500"
          />
          <span v-else class="text-xs truncate block w-full flex-1 min-w-0">{{ plan.name }}</span>
          <span class="text-[11px] text-surface-400 font-mono flex-shrink-0 ml-2">{{
            Array.isArray(plan.history) ? plan.history.length : 0
          }}</span>
          <button
            @click.stop="srp(plan)"
            class="opacity-0 group-hover:opacity-100 flex-shrink-0 w-5 h-5 ml-1 text-surface-400 hover:text-brand-600 flex items-center justify-center"
          >
            <AppIcon name="edit" class="w-3 h-3" />
          </button>
          <button
            v-if="plans.length > 1"
            @click.stop="rmp(plan.id)"
            class="opacity-0 group-hover:opacity-100 flex-shrink-0 w-5 h-5 ml-0.5 text-surface-400 hover:text-red-500 flex items-center justify-center"
          >
            <AppIcon name="trash" class="w-3 h-3" />
          </button>
        </div>
      </div>
    </div>

    <!-- 时间线：占据中栏+右栏，版本卡片平铺 -->
    <div
      v-if="av === 'timeline'"
      class="flex-1 min-w-0 flex bg-white/50 overflow-hidden"
    >
      <div class="w-[380px] shrink-0 flex flex-col border-r border-surface-100">
        <div
          class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0"
        >
          <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider"
            >清单版本</span
          >
          <button
            @click="ommBlank"
            class="flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors"
            v-tooltip="'新增空白版本'"
          >
            <AppIcon name="plus" class="w-3 h-3" />
          </button>
        </div>
        <div
          class="flex-1 overflow-y-auto py-3 px-3 min-h-0 bg-gray-50/30 flex flex-col items-start space-y-2 [&>*]:w-full [&>*]:max-w-[440px] [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full"
        >
          <!-- 草稿卡片 -->
          <div
            v-if="cplan?.draft"
            @click="selectEntry({ type: 'draft', data: cplan.draft })"
            class="bg-white rounded-lg border px-3 py-2.5 cursor-pointer transition-shadow hover:shadow-sm group/card border-amber-200 shadow-sm"
            :class="selEntry?.type === 'draft' ? 'ring-2 ring-amber-300/60' : ''"
          >
            <!-- Row 1: dot + version + time + amount + actions -->
            <div class="flex items-center gap-2">
              <div class="w-2 h-2 rounded-full bg-amber-400 flex-shrink-0" />
              <span class="text-xs font-medium text-amber-600 shrink-0">{{
                fmtVersion(cplan.draft.versionNum)
              }}</span>
              <span class="text-[10px] text-surface-400 shrink-0">{{
                formatDateTime(cplan.draft.startedAt || cplan.draft.createdAt)
              }}</span>
              <span class="text-xs font-semibold text-slate-700 font-mono shrink-0"
                >¥{{
                  draftAmount.toLocaleString('zh-CN', {
                    minimumFractionDigits: 2,
                    maximumFractionDigits: 2
                  })
                }}</span
              >
            </div>
          </div>

          <!-- 空状态 -->
          <div
            v-if="!cplan?.draft && ste.length === 0"
            class="text-center py-12 text-gray-400 text-xs space-y-3 col-span-full"
          >
            <p>暂无版本记录</p>
            <button @click="omm" class="btn-secondary">
              <AppIcon name="plus-thin" class="w-3.5 h-3.5" />新增版本
            </button>
          </div>

          <!-- 已归档版本卡片 -->
          <div
            v-for="(entry, idx) in ste.filter((e) => e.type === 'ver')"
            :key="entry.data.versionId || entry.data.id"
            @click="selectEntry(entry)"
            class="bg-white rounded-lg border px-3 py-2.5 cursor-pointer transition-shadow hover:shadow-sm group/ver"
            :class="
              selEntry &&
              (selEntry.data.versionId || selEntry.data.id) === (entry.data.versionId || entry.data.id)
                ? 'ring-2 ring-blue-300/70 border-blue-300 shadow-sm'
                : idx === 0 && !cplan?.draft
                  ? 'border-brand-200 shadow-sm'
                  : 'border-surface-100'
            "
          >
            <!-- Row 1: dot + version + time + amount + actions -->
            <div class="flex items-center gap-2">
              <div
                class="w-2 h-2 rounded-full flex-shrink-0"
                :class="idx === 0 && !cplan?.draft ? 'bg-brand-500' : 'bg-surface-300'"
              />
              <span
                class="text-xs font-semibold shrink-0 min-w-[28px]"
                :class="idx === 0 && !cplan?.draft ? 'text-brand-600' : 'text-surface-600'"
                >{{ fmtVersion(entry.data.versionNum) }}</span
              >
              <span class="text-[10px] text-surface-400 shrink-0">{{
                formatDateTime(entry.data.createdAt)
              }}</span>
              <span class="text-xs font-semibold text-slate-700 font-mono shrink-0"
                >¥{{
                  (entry.data.amount || 0).toLocaleString('zh-CN', {
                    minimumFractionDigits: 2,
                    maximumFractionDigits: 2
                  })
                }}</span
              >
            </div>
          </div>
        </div>
      </div>

      <!-- 版本概览栏（与清单版本标题栏同级） -->
      <div
        v-if="av === 'timeline'"
        class="flex-1 min-w-0 flex flex-col bg-white overflow-hidden"
      >
        <div
          class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0"
        >
          <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider"
            >版本概览</span
          >
          <div v-if="selEntry" class="flex items-center gap-1.5">
            <template v-if="selEntry.type === 'draft'">
              <button
                @click="sed"
                class="btn-ghost !p-1 text-slate-400 hover:text-blue-600"
                v-tooltip="'编辑'"
              >
                <AppIcon name="edit" class="w-3.5 h-3.5" />
              </button>
              <button
                @click="cancelDraft"
                class="btn-ghost !p-1 text-slate-400 hover:text-red-500"
                v-tooltip="'删除草稿'"
              >
                <AppIcon name="trash" class="w-3.5 h-3.5" />
              </button>
            </template>
            <template v-else>
              <button
                @click="vhv(selEntry.idx)"
                class="btn-ghost !p-1 text-slate-400 hover:text-blue-600"
                v-tooltip="'查看'"
              >
                <AppIcon name="eye" class="w-3.5 h-3.5" />
              </button>
              <button
                @click="editFromOverview"
                class="btn-ghost !p-1 text-slate-400 hover:text-blue-600"
                v-tooltip="'编辑（基于该版本新建草稿）'"
              >
                <AppIcon name="edit" class="w-3.5 h-3.5" />
              </button>
              <button
                @click="evx(selEntry.data)"
                class="btn-ghost !p-1 text-slate-400 hover:text-blue-600"
                v-tooltip="'下载 Excel'"
              >
                <AppIcon name="download" class="w-3.5 h-3.5" />
              </button>
            </template>
          </div>
        </div>
        <div v-if="!selEntry" class="flex-1 flex items-center justify-center text-gray-300 text-xs">
          暂无版本
        </div>
        <div v-else class="flex-1 overflow-y-auto p-3 space-y-3">
          <div class="flex items-center gap-2">
            <span
              class="px-1.5 py-0.5 rounded text-[10px] font-medium border shrink-0"
              :class="
                selEntry.type === 'draft'
                  ? 'border-amber-200 text-amber-600 bg-amber-50'
                  : 'border-surface-200 text-surface-500 bg-surface-50'
              "
              >{{
                fmtVersion(selEntry.data.versionNum) + (selEntry.type === 'draft' ? ' · 草稿' : '')
              }}</span
            >
            <span class="text-[10px] text-surface-400">{{
              formatDateTime(selEntry.data.startedAt || selEntry.data.createdAt)
            }}</span>
          </div>
          <!-- 报价概览磁贴 -->
          <div v-if="selOverview" class="grid grid-cols-2 gap-2">
            <div class="rounded-lg border border-surface-100 bg-surface-50/60 px-2.5 py-2">
              <div class="text-[10px] text-surface-400">状态</div>
              <div class="text-[12px] font-medium mt-0.5" :class="selEntry.type === 'draft' ? 'text-amber-600' : 'text-surface-700'">{{ selOverview.status }}</div>
            </div>
            <div class="rounded-lg border border-surface-100 bg-surface-50/60 px-2.5 py-2">
              <div class="text-[10px] text-surface-400">合计金额</div>
              <div class="text-[12px] font-semibold font-mono text-red-600 mt-0.5">¥{{ selOverview.amount.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</div>
            </div>
            <div class="rounded-lg border border-surface-100 bg-surface-50/60 px-2.5 py-2">
              <div class="text-[10px] text-surface-400">设备总数</div>
              <div class="text-[12px] font-medium text-surface-700 mt-0.5">{{ selOverview.totalUnits }} 台</div>
            </div>
            <div class="rounded-lg border border-surface-100 bg-surface-50/60 px-2.5 py-2">
              <div class="text-[10px] text-surface-400">子网数量</div>
              <div class="text-[12px] font-medium text-surface-700 mt-0.5">{{ selOverview.subnets }} 个</div>
            </div>
          </div>
          <!-- 设备型号（默认收起，点击展开） -->
          <div>
            <button
              @click="overviewExpanded = !overviewExpanded"
              class="w-full flex items-center justify-between px-2.5 py-2 rounded-lg border border-surface-100 bg-surface-50/60 hover:bg-surface-100 transition-colors"
            >
              <span class="text-[11px] font-medium text-surface-600 flex items-center gap-1.5">
                <AppIcon
                  name="chevron-down"
                  class="w-3 h-3 transition-transform"
                  :class="overviewExpanded ? '' : '-rotate-90'"
                />设备型号
              </span>
              <span class="text-[11px] text-surface-400"
                >共 {{ selOverview?.totalUnits ?? 0 }} 台{{ overviewExpanded ? '' : ' · 点击展开' }}</span
              >
            </button>
            <div
              v-if="overviewExpanded"
              class="border border-t-0 border-surface-100 rounded-b-lg divide-y divide-surface-100 max-h-52 overflow-y-auto"
            >
              <div
                v-for="m in selModelSummary"
                :key="m.model"
                class="flex items-center justify-between px-2.5 py-1.5 text-[11px] gap-2"
              >
                <span class="text-surface-600 truncate" :title="m.model">{{ m.model }}</span>
                <span class="text-surface-400 font-mono shrink-0">× {{ m.qty }}</span>
              </div>
              <div v-if="selModelSummary.length === 0" class="px-2.5 py-2 text-[11px] text-surface-300">
                无设备
              </div>
            </div>
          </div>
          <!-- 变更说明（多行文本，失焦保存） -->
          <div>
            <div class="text-[11px] font-medium text-surface-500 mb-1">变更说明</div>
            <textarea
              v-model="remarkVal"
              @blur="saveSelRemark"
              rows="4"
              placeholder="记录此版本的变更目的或客户反馈..."
              class="w-full text-[11px] px-2 py-1.5 border border-surface-200 rounded-lg outline-none focus:border-blue-400 focus:ring-1 focus:ring-blue-200 resize-none bg-white leading-relaxed"
            ></textarea>
          </div>
        </div>
      </div>
    </div>

    <!-- Main: 设备表格（编辑 / 只读） -->
    <div v-else class="flex-1 flex flex-col min-w-0 bg-white overflow-hidden">
      <div
        class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0"
      >
        <div class="flex items-center gap-1.5 min-w-0">
          <button @click="gtl" class="text-surface-400 hover:text-brand-600 p-0.5 flex-shrink-0">
            <AppIcon name="arrow-left" class="w-4 h-4" />
          </button>
          <span class="text-xs font-medium text-surface-700 truncate">{{ cpname }}</span>
          <span
            class="px-1.5 py-0.5 rounded text-[10px] font-medium border border-surface-200 text-surface-500 shrink-0"
            >{{ fmtVersion(im ? cplan?.draft?.versionNum : vv?.versionNum) || '草稿' }}</span
          >
          <span v-if="ir && vv?.createdAt" class="text-[10px] text-surface-400 flex-shrink-0"
            >{{ formatDateTime(vv.createdAt) }}</span
          >
          <span
            v-if="ir && vv?.operator && vv.operator !== op"
            class="text-[10px] text-surface-400 flex-shrink-0"
            >{{ vv.operator }}</span
          >
        </div>
        <div class="flex items-center gap-1 ml-auto flex-nowrap">
          <template v-if="im">
            <button
              @click="apl"
              class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0"
              v-tooltip="'添加设备（追加到末尾）'"
            >
              <AppIcon name="plus" class="w-3.5 h-3.5" />
            </button>
            <button
              @click="asub"
              class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0"
              v-tooltip="'添加子网（追加到末尾）'"
            >
              <AppIcon name="folder" class="w-3.5 h-3.5" />
            </button>
            <button
              @click="showImportWizard = true"
              class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0"
              v-tooltip="'导入 Excel'"
            >
              <AppIcon name="upload" class="w-3.5 h-3.5" />
            </button>
            <div class="w-px h-4 bg-surface-200 mx-0.5 flex-shrink-0" />
            <button
              @click="dt?.undo()"
              :disabled="!dt?.canUndo"
              class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0 disabled:opacity-30 disabled:pointer-events-none"
              v-tooltip="'撤销 (Ctrl+Z)'"
            >
              <AppIcon name="undo" class="w-3.5 h-3.5" />
            </button>
            <button
              @click="dt?.redo()"
              :disabled="!dt?.canRedo"
              class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0 disabled:opacity-30 disabled:pointer-events-none"
              v-tooltip="'重做 (Ctrl+Shift+Z)'"
            >
              <AppIcon name="undo" class="w-3.5 h-3.5 rotate-180" />
            </button>
            <button
              @click="dt?.lintAll()"
              class="h-6 px-2 flex items-center gap-1 rounded-md text-[11px] text-surface-500 hover:text-amber-600 hover:bg-amber-50 transition-colors flex-shrink-0"
              v-tooltip="'检查序号、分隔符、标点与行尾标点（结果面板内可一键修复）'"
            >
              <AppIcon name="search" class="w-3.5 h-3.5" /><span>检查</span>
            </button>
            <button
              @click="stripControlMarks"
              class="h-6 px-2 flex items-center gap-1 rounded-md text-[11px] text-surface-500 hover:text-red-600 hover:bg-red-50 transition-colors flex-shrink-0"
              v-tooltip="'剥除 ★/▲ 符号，删除含截图/证明/报告/盖章的证明材料段，删除后自动修复标点（可撤销）'"
            >
              <AppIcon name="trash" class="w-3.5 h-3.5" /><span>删除控标</span>
            </button>
            <button
              @click="preSave"
              :disabled="isSaving"
              class="flex items-center gap-1 h-6 px-2 rounded-md text-xs bg-blue-600 text-white hover:bg-blue-700 disabled:bg-blue-400 disabled:cursor-wait transition-colors flex-shrink-0 whitespace-nowrap ml-1"
            >
              <AppIcon v-if="isSaving" name="spinner" class="animate-spin w-3 h-3" /><AppIcon
                v-else
                name="save"
                class="w-3 h-3"
              /><span>{{ isSaving ? '保存中...' : '保存' }}</span>
            </button>
          </template>
          <button v-else @click="omm" class="btn-primary">
            <AppIcon name="edit" class="w-3 h-3" />
            修改
          </button>
        </div>
      </div>
      <div
        v-if="saveError"
        class="px-3 py-2 bg-red-50 border-b border-red-100 text-[11px] text-red-600 flex items-center gap-1.5 shrink-0"
      >
        <AppIcon name="alert-circle" class="w-3 h-3 flex-shrink-0" /><span>{{ saveError }}</span>
      </div>
      <DeviceTable
        ref="dt"
        :items="li"
        :products="gp"
        :readonly="ir"
        :invalid-uids="invalidItems"
        class="flex-1 min-h-0"
      />
      <!-- 总价栏：左=调价工具（各工具异色文本），右=总价标签+金额 -->
      <div
        class="shrink-0 border-t border-gray-200 bg-gray-50/80 px-4 py-2.5 flex items-center justify-between"
      >
        <div class="flex items-center gap-2">
          <template v-if="im">
            <!-- 编辑态：徽标/按钮原处变为输入行 -->
            <div v-if="showMarkupInput" class="flex items-center gap-1 bg-orange-50 rounded-md px-1.5 py-0.5 border border-orange-200">
              <button
                @click="markupDivide = !markupDivide"
                class="w-5 h-5 flex items-center justify-center text-[11px] font-medium text-orange-600 hover:text-orange-800 bg-white border border-orange-200 rounded"
                :title="markupDivide ? '除法：单价 ÷ 系数（点击切回乘法）' : '乘法：单价 × 系数（点击切为除法）'"
              >{{ markupDivide ? '÷' : '×' }}</button>
              <input
                v-model.number="markupCoef"
                type="number"
                min="0.1"
                max="10"
                step="0.1"
                class="w-12 h-5 text-xs text-center bg-white border border-orange-200 rounded outline-none focus:border-orange-400"
                placeholder="1.2"
                @keyup.enter="applyMarkup"
                @keyup.escape="showMarkupInput = false"
              />
              <button @click="applyMarkup" class="text-xs text-orange-600 hover:text-orange-800 font-medium px-1">应用</button>
              <button @click="showMarkupInput = false" class="text-xs text-gray-400 hover:text-gray-600 px-1">取消</button>
            </div>
            <!-- 已上浮徽标：修改原位展开为输入行 -->
            <span
              v-else-if="Math.abs((curDraft?.priceMarkup ?? 1) - 1) > 0.004"
              class="inline-flex items-center gap-1 text-[11px] text-orange-700 bg-orange-50 border border-orange-200 rounded px-1.5 py-0.5"
            >
              {{ (curDraft?.priceMarkup ?? 1) >= 1 ? '已上浮' : '已下浮' }} {{ fmtMarkup(curDraft?.priceMarkup ?? 1) }}
              <button
                @click="openMarkupEdit"
                class="text-orange-600 hover:text-orange-900 font-medium underline underline-offset-2"
                title="修改调价系数（以原价为基线重算，不复利；支持乘法/除法）"
                >修改</button
              >
              <button
                @click="restorePrices"
                class="text-orange-600 hover:text-orange-900 font-medium underline underline-offset-2"
                title="把所有设备单价还原为上浮前的原价快照"
                >还原原价</button
              >
            </span>
            <!-- 无调价：上浮入口 -->
            <button
              v-else
              @click="showMarkupInput = true"
              class="h-6 px-2 flex items-center rounded-md text-[11px] font-medium text-orange-600 hover:text-orange-700 hover:bg-orange-50 transition-colors"
            >
              价格上浮
            </button>
            <button
              @click="ceilPrices"
              class="h-6 px-2 flex items-center rounded-md text-[11px] font-medium text-blue-600 hover:text-blue-700 hover:bg-blue-50 transition-colors"
              title="把所有设备单价向上取整为整数（可撤销）"
            >
              向上取整
            </button>
          </template>
          <span
            v-if="!im && Math.abs((vv?.priceMarkup ?? 1) - 1) > 0.004"
            class="text-[11px] text-orange-700 bg-orange-50 border border-orange-200 rounded px-1.5 py-0.5"
          >
            {{ (vv.priceMarkup ?? 1) >= 1 ? '已上浮' : '已下浮' }} {{ fmtMarkup(vv.priceMarkup ?? 1) }}
          </span>
        </div>
        <span class="flex items-center gap-2">
          <span class="text-xs font-semibold text-gray-600 uppercase tracking-wider"
            >总价</span
          >
          <span class="text-sm font-bold text-red-600"
            >¥
            {{
              gt.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
            }}</span
          >
        </span>
      </div>
    </div>
  </div>

  <!-- Modify Modal -->
  <Teleport to="body">
    <div
      v-if="smm"
      class="fixed inset-0 z-[80] flex items-center justify-center bg-gray-900/40 backdrop-blur-sm"
    >
      <div class="bg-white rounded-2xl shadow-2xl w-full max-w-md overflow-hidden">
        <div class="px-6 py-4 border-b border-gray-100">
          <h3 class="text-sm font-semibold text-gray-800">保存版本 — {{ cpname }}</h3>
        </div>
        <div class="p-6 space-y-4">
          <div>
            <label class="block text-[11px] font-medium text-gray-500 mb-1">变更说明</label
            ><textarea
              v-model="mf.cm"
              rows="2"
              placeholder="记录此次变更的目的或客户反馈..."
              class="w-full text-[12px] px-3 py-2 border border-gray-200 rounded-lg outline-none focus:border-[#007aff] focus:ring-1 focus:ring-blue-500 resize-none"
            ></textarea>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50/50 flex justify-end gap-2">
          <button @click="smm = false" class="btn-ghost">取消</button>
          <button @click="confirmSave" class="btn-primary">确认保存</button>
        </div>
      </div>
    </div>
  </Teleport>

  <!-- Excel Import Wizard -->
  <ExcelImportWizard
    v-model="showImportWizard"
    title="导入清单 Excel"
    :fields="solutionImportFields"
    :rowActions="[
      { key: 'device', label: '添加产品', color: 'emerald' },
      { key: 'subnet', label: '添加子网', color: 'blue' },
      { key: 'ignore', label: '忽略', color: 'slate' },
      { key: 'problem', label: '需要选择', color: 'amber', isProblem: true }
    ]"
    :classify="classifySolutionRow"
    :shouldSkipRow="shouldSkipSolutionRow"
    :preset="sctPreset"
    @confirm="onImportConfirm"
    @preset-launch="onSCTLaunch"
  />

  <!-- SCT 服务费分摊模态框 -->
  <Teleport to="body">
    <div
      v-if="showSCTAllocator"
      class="fixed inset-0 z-[90] flex items-center justify-center bg-black/40 backdrop-blur-sm"
    >
      <div
        class="bg-white rounded-2xl shadow-2xl w-full max-w-4xl mx-4 flex flex-col overflow-hidden"
        style="height: min(85vh, 720px)"
      >
        <div class="px-6 py-3 border-b border-gray-100 flex items-center justify-between shrink-0">
          <div class="flex items-center gap-2">
            <h3 class="text-sm font-semibold text-gray-800">华为 SCT 报价单 — 服务费分摊</h3>
            <span
              class="px-2 py-0.5 rounded-full text-[10px] font-medium bg-brand-50 text-brand-700 ring-1 ring-brand-200"
              >已识别</span
            >
          </div>
          <button
            @click="handleServiceFeeCancel"
            class="text-gray-400 hover:text-gray-600 transition-colors"
          >
            <AppIcon name="x" class="w-4 h-4" />
          </button>
        </div>
        <div class="flex-1 overflow-hidden flex flex-col min-h-0">
          <ServiceFeeAllocator
            ref="sctAllocator"
            :sheet="sctSheet"
            @confirm="handleServiceFeeConfirm"
            @cancel="handleServiceFeeCancel"
          />
        </div>
      </div>
    </div>
  </Teleport>

</template>
