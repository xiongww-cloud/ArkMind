<script setup lang="ts">
import { ref, reactive, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { parsePMSExcel, exportToPMSExcel } from '@/utils/solutionExcel'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { confirm, dangerConfirm, alert } from '@/composables/useConfirm'
import AppIcon from '@/components/common/AppIcon.vue'
import AppRemark from '@/components/common/AppRemark.vue'

import * as api from '@/api/db-client'
import { formatDateTime } from '@/utils/time'
const { updateSolutionVersionMeta, fetchProducts, createProduct, updateProduct, deleteVersion } = api
const props = defineProps({ project: { type: Object, required: true } })
const projectStore = useProjectStore()
const authStore = useAuthStore()
let pc = 0; const gpid = () => 'plan-' + (++pc) + '-' + Date.now()
const guid = () => Math.random().toString(36).substring(2, 11)
const gvid = () => 'ver-' + Date.now()
const op = computed(() => authStore.currentUser.name)

const gp = ref([])  // 产品列表 — 从后端 Product 表加载
const gparams = ref([])  // 参数列表 — 从后端加载

// ─── 设备型号内联补全 (Inline Autocomplete) ─────────────────────────────────
const isDeleting = ref(false)
const showUploadModal = ref(false)

// ─── Excel 格式导入 ──────────────────────────────────────────────────────────
const pmsFileInput = ref(null)
const pmsSelectedFile = ref(null) // 选中的文件，确认后才解析
const isPMSImporting = ref(false)
const pmsImportError = ref('')
const pmsFileName = ref('')

function triggerPMSFile() { pmsFileInput.value?.click() }

function onPMSFilePickedFromInput(e) {
  const file = e.target.files?.[0]
  if (!file) return
  pmsSelectedFile.value = file
  pmsFileName.value = file.name
}

function confirmPMSImport() {
  if (!pmsSelectedFile.value) return
  handlePMSFile(pmsSelectedFile.value)
}

function cancelImport() {
  showUploadModal.value = false
  pmsImportError.value = ''
  pmsFileName.value = ''
  pmsSelectedFile.value = null
  if (pmsFileInput.value) pmsFileInput.value.value = ''
}

async function handlePMSFile(file) {
  isPMSImporting.value = true
  pmsImportError.value = ''

  const productList = Array.isArray(gp.value) ? gp.value : []
  const result = await parsePMSExcel(file, productList)
  isPMSImporting.value = false
  if (pmsFileInput.value) pmsFileInput.value.value = ''

  if (result.error) {
    pmsImportError.value = result.error
    return
  }

  // 直接设置草稿的设备清单
  if (cplan.value?.draft) {
    cplan.value.draft.items = result.items.map((item: any) => {
      if (item.rowType === 'subnet' || !item.model) return item
      // 自动匹配产品库获取参数
      const match = (Array.isArray(gp.value) ? gp.value : []).find(
        (p: any) => (p.name || '').toLowerCase() === (item.model || '').toLowerCase()
      )
      if (match) {
        const allGparams = Array.isArray(gparams.value) ? gparams.value : []
        const productParams = allGparams.filter((p: any) => p.productId === match.id)
        return {
          ...item,
          productId: match.id,
          syncParams: false,
          mergedText: item.mergedText || match.full_specs || match.standard_specs || '',
          params: productParams.map((p: any) => ({ ...p, mark: '' })),
        }
      }
      return { ...item, syncParams: item.syncParams !== false }
    })
  }
  showUploadModal.value = false
  pmsSelectedFile.value = null
  pmsFileName.value = ''
}

function onDeviceKeydown(e) {
  isDeleting.value = e.key === 'Backspace'
}

function handleDeviceInput(e, item) {
  if (isDeleting.value) {
    isDeleting.value = false
    return
  }

  const typedValue = e.target.value
  item.model = typedValue

  if (!typedValue) {
    item.productId = null
    return
  }

  const searchPrefix = typedValue.trim().toLowerCase()
  const match = gp.value?.find(p =>
    (p.name || '').toLowerCase().startsWith(searchPrefix)
  )

  if (match) {
    item.model = match.name
    item.productId = match.id
    if (match.price && !item.unitPrice) {
      item.unitPrice = match.price
    }
    nextTick(() => {
      e.target.setSelectionRange(typedValue.length, match.name.length)
    })
  } else {
    item.productId = null
  }
}

function handleDeviceConfirm(item, e) {
  const text = (e?.target?.value || item.model || '').trim()
  if (!text) { item.productId = null; item.model = ''; return }
  item.model = text

  const match = (Array.isArray(gp.value) ? gp.value : []).find(p => (p.name || '').toLowerCase() === text.toLowerCase())
  if (match) {
    item.productId = match.id
    item.model = match.name || match.model
    item.syncParams = false  // matched device: don't sync back by default
    // Populate price from product library
    if (match.price && !item.unitPrice) {
      item.unitPrice = match.price
    }
    // Load default parameters from product
    item.mergedText = match.full_specs || match.standard_specs || ''
    // Pre-populate params from gparams if available, otherwise leave as-is
    const allGparams = Array.isArray(gparams.value) ? gparams.value : []
    const productParams = allGparams.filter(p => p.productId === match.id)
    if (productParams.length > 0) {
      item.params = productParams.map(p => ({ ...p, mark: '' }))
    }
  } else {
    // No product match: treat as new device, enable sync by default
    item.productId = null
    item.syncParams = true
  }
}

const plans = ref([])
const apid = ref('')
function iplans() { if (!props.project.plans || props.project.plans.length === 0) { props.project.plans = [{ id: gpid(), name: '默认方案', items: [], history: [], draft: null, createdAt: new Date().toISOString() }] }; plans.value = props.project.plans; if (!apid.value || !plans.value.find(p => p.id === apid.value)) apid.value = plans.value[0]?.id || '' }
iplans()
const lvt = (plan) => { const h = Array.isArray(plan.history) ? plan.history : []; if (h.length) return Math.max(...h.map(v => new Date(v.createdAt).getTime())); return new Date(plan.createdAt || 0).getTime() }
const splans = computed(() => [...plans.value].sort((a, b) => lvt(b) - lvt(a)))
const cplan = computed(() => plans.value.find(p => p.id === apid.value))
const cpname = computed(() => cplan.value?.name || '')

const av = ref('timeline')
const vvi = ref(null)
const im = computed(() => av.value === 'edit')
const ir = computed(() => av.value === 'view')
const vv = computed(() => { if (ir.value && vvi.value !== null) return cplan.value?.history[vvi.value] || null; return null })

const si = computed(() => { if (im.value && cplan.value?.draft) { const d = cplan.value.draft.items; return Array.isArray(d) ? d : [] } if (ir.value && vvi.value !== null) { const h = cplan.value?.history[vvi.value]?.items; if (!Array.isArray(h)) return []; const allProducts = Array.isArray(gp.value) ? gp.value : []; return h.map(item => { if (item.rowType === 'subnet' || item.productId) return item; const model = (item.model || '').trim(); if (!model) return item; const matched = allProducts.find(p => (p.name || '').toLowerCase() === model.toLowerCase()); if (!matched) return item; return { ...item, productId: matched.id, model: matched.name, unitPrice: item.unitPrice || matched.price || 0, mergedText: item.mergedText || matched.full_specs || '' } }) } return [] })
const li = computed({ get: () => si.value, set: (v) => { if (im.value && cplan.value?.draft) cplan.value.draft.items = v } })
const gt = computed(() => li.value.reduce((s, i) => i.rowType === 'subnet' ? s : s + (i.unitPrice || 0) * (i.quantity || 0), 0))
const deviceCount = computed(() => li.value.filter(i => i.rowType !== 'subnet').length)

const ste = computed(() => { if (!cplan.value) return []; const e = []; if (cplan.value.draft) e.push({ type: 'draft', data: cplan.value.draft }); const history = cplan.value.history; if (Array.isArray(history)) history.forEach((v, i) => e.push({ type: 'ver', data: v, idx: i })); return e })

const aii = ref(null)
function apl() { if (ir.value) return; li.value.push({ uid: guid(), rowType: 'device', productId: '', model: '', customName: '', unitPrice: 0, quantity: 1, unit: '台', params: [], mergedText: '', syncParams: true }); aii.value = li.value.length - 1 }
function asub() { if (ir.value) return; const idx = aii.value !== null ? aii.value + 1 : li.value.length; li.value.splice(idx, 0, { uid: guid(), rowType: 'subnet', name: '新子网' }); aii.value = idx }
function rpl(i) { if (ir.value) return; li.value.splice(i, 1); if (aii.value === i) aii.value = null; else if (aii.value > i) aii.value-- }
function hpc(item) { if (ir.value) return; item.params = []; item.mergedText = ''; item.model = gpn(item.productId) }

const dragX = ref(0)
const dragY = ref(0)
const dsi = ref(null); const dti = ref(null); const idr = ref(false)
const dragInsert = ref<{targetIdx: number; side: 'above'|'below'} | null>(null)
let dragTimer = null

function ogd(e, idx) {
  if (ir.value) return
  dsi.value = idx
  dragX.value = e.clientX
  dragY.value = e.clientY
  // 立即阻止文本选中
  document.body.style.userSelect = 'none'
  document.addEventListener('selectstart', preventSelect)
  document.addEventListener('mousemove', odm)
  dragTimer = setTimeout(() => {
    idr.value = true
  }, 200)
  document.addEventListener('mouseup', oup)
}

function preventSelect(e) { e.preventDefault() }

function odm(e) {
  e.preventDefault()
  dragX.value = e.clientX
  dragY.value = e.clientY
  if (!idr.value) return
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const card = el?.closest('[data-item-idx]') as HTMLElement | null
  if (!card) { dti.value = null; dragInsert.value = null; return }
  const tidx = parseInt(card.getAttribute('data-item-idx') || '')
  if (isNaN(tidx) || tidx === dsi.value) { dti.value = null; dragInsert.value = null; return }
  dti.value = tidx
  const rect = card.getBoundingClientRect()
  const relY = e.clientY - rect.top
  const half = rect.height / 2
  dragInsert.value = { targetIdx: tidx, side: relY < half ? 'above' : 'below' }
}

function oup(e) {
  document.removeEventListener('mousemove', odm)
  document.removeEventListener('mouseup', oup)
  document.removeEventListener('selectstart', preventSelect)
  document.body.style.userSelect = ''
  if (dragTimer) { clearTimeout(dragTimer); dragTimer = null }
  if (!idr.value) return
  const si = dsi.value
  const insert = dragInsert.value
  dsi.value = null; dti.value = null; idr.value = false; dragInsert.value = null
  if (si === null || !insert) return
  let ti = insert.targetIdx
  if (insert.side === 'below') ti = Math.min(ti + 1, li.value.length)
  if (ti === si || ti === si + 1) return
  const its = [...li.value]
  const [m] = its.splice(si, 1)
  const adjusted = si < ti ? ti - 1 : ti
  its.splice(adjusted, 0, m)
  li.value = its
  if (aii.value === si) aii.value = adjusted
  else if (aii.value !== null) {
    if (si < aii.value && adjusted >= aii.value) aii.value--
    else if (si > aii.value && adjusted <= aii.value) aii.value++
  }
}

const uo = ref(null); const upos = reactive({ top: 0, left: 0 })
const units = ['台', '套', '个', '年']
function tud(uid, e) { if (uo.value === uid) { uo.value = null; return }; const r = e.currentTarget.getBoundingClientRect(); upos.top = r.bottom + 4; upos.left = r.left; uo.value = uid }
function sud(u) { const item = li.value.find(i => i.uid === uo.value); if (item) item.unit = u; uo.value = null }
function cud(e) {
  if (uo.value && !e.target.closest('[data-unit-dropdown]')) uo.value = null
}

const imo = ref(false)
const ms = reactive({ ti: null, pid: '', ap: [], ts: [], tm: {}, sq: '' })
const fmp = computed(() => { if (!ms.sq.trim()) return ms.ap; const q = ms.sq.toLowerCase(); return ms.ap.filter(p => p.indicator.toLowerCase().includes(q) || p.requirement.toLowerCase().includes(q)) })
const gpn = (id) => (Array.isArray(gp.value) ? gp.value.find(p => p.id === id) : undefined)?.name || ''
function opm(item, idx) { if (!item.productId || ir.value) return; ms.sq = ''; ms.ti = idx; ms.pid = item.productId; ms.ap = Array.isArray(gparams.value) ? gparams.value.filter(p => p.productId === item.productId) : []; const params = Array.isArray(item.params) ? item.params : []; ms.ts = params.map(p => p.id); ms.tm = {}; params.forEach(p => { if (p.mark) ms.tm[p.id] = p.mark }); imo.value = true; aii.value = idx }
function clm() { imo.value = false }
function gmbc(m) { if (m === '▲') return 'bg-gray-800 text-white border-gray-800'; if (m === '★') return 'bg-black text-yellow-400 border-black'; return 'bg-gray-50 text-gray-400 hover:bg-gray-100 border-gray-200' }
function gmbt(m) { if (m === '▲') return '▲ 重点'; if (m === '★') return '★ 废标'; return '无标记' }
function tcb(id) { const i = ms.ts.indexOf(id); if (i > -1) ms.ts.splice(i, 1); else ms.ts.push(id) }
function tpm(id) { const c = ms.tm[id]; if (!c) { ms.tm[id] = '▲'; if (!ms.ts.includes(id)) ms.ts.push(id) } else if (c === '▲') ms.tm[id] = '★'; else delete ms.tm[id] }
function cms() { const item = li.value[ms.ti]; if (!item) return; const sel = ms.ap.filter(p => ms.ts.includes(p.id)).map(p => ({ ...p, mark: ms.tm[p.id] || '' })); item.params = sel; item.mergedText = sel.map((p, idx) => p.mark + (idx + 1) + '、' + p.indicator + '：' + p.requirement).join('\n'); if (item.productId) item.syncParams = true; clm() }
function autoEnableSync(idx) { const item = li.value[idx]; if (item?.productId) item.syncParams = true }


const smm = ref(false)
const isSaving = ref(false)
const saveError = ref('')
const atags = ['降本减配', '增项扩容', '参数微调', '方案重构']
const mf = reactive({ vn: '', tags: [], cm: '' })
function fmtVersion(v) { if (!v) return ''; return v.replace(/^V/, 'v') }
const invalidItems = ref([]) // uids of items with validation errors
function validateItems(items) { if (!items || !Array.isArray(items) || items.length === 0) return []; return items.filter(item => { if (item.rowType === 'subnet') return false; const noDevice = !item.productId && !item.model; const noPrice = !item.unitPrice || item.unitPrice <= 0; return noDevice || noPrice }) }
function clearValidation() { invalidItems.value = [] }
function omm() { if (!cplan.value) return; const vn = 'v' + (cplan.value.history.length + 1) + '.0'; const rawSrc = (av.value === 'view' && vvi.value !== null && cplan.value.history[vvi.value]) ? cplan.value.history[vvi.value].items : cplan.value.items; const srcItems = Array.isArray(rawSrc) ? rawSrc : []; const allProducts = Array.isArray(gp.value) ? gp.value : []; const copiedItems = JSON.parse(JSON.stringify(srcItems)).map(item => { if (item.rowType === 'subnet') return { ...item, syncParams: false }; const model = (item.model || '').trim(); const matched = allProducts.find(p => (p.name || '').toLowerCase() === model.toLowerCase()); const hasProduct = !!(matched?.id || item.productId); return { ...item, syncParams: hasProduct ? false : true, productId: matched?.id || item.productId || '', model: matched ? matched.name : item.model, unitPrice: (!item.unitPrice && matched?.price) ? matched.price : item.unitPrice, mergedText: (!item.mergedText && matched?.full_specs) ? matched.full_specs : item.mergedText } })
cplan.value.draft = { versionNum: vn, tags: [], content: '', items: copiedItems, startedAt: new Date().toISOString(), operator: op.value }; av.value = 'edit'; aii.value = null; clearValidation() }
function tmt(tag) { const i = mf.tags.indexOf(tag); if (i > -1) mf.tags.splice(i, 1); else mf.tags.push(tag) }
function findUnknownDevices(items) {
  if (!items || !Array.isArray(items)) return []
  return items.filter(item => {
    if (item.rowType === 'subnet') return false
    if (!item.productId && item.model && item.model.trim()) return true
    if (item.productId && !(Array.isArray(gp.value) ? gp.value : []).find(p => p.id === item.productId)) return true
    return false
  })
}

async function preSave() {
  if (!cplan.value?.draft) return
  const draftItems = Array.isArray(cplan.value?.draft?.items) ? cplan.value.draft.items : []
  const bad = validateItems(draftItems)
  if (bad.length > 0) { invalidItems.value = bad.map(i => i.uid); saveError.value = '保存失败：清单中存在未选择设备或未填写金额的条目，请完善后再试'; return }
  invalidItems.value = []; saveError.value = ''

  // 检测未知设备（产品库中不存在的型号）
  const unknown = findUnknownDevices(draftItems)
  if (unknown.length > 0) {
    const uniqueModels = Array.from(new Set(
      unknown.map(i => i.model?.trim()).filter(Boolean)
    ))
    const names = uniqueModels.join('、')
    const ok = await confirm({
      title: '发现未知设备型号',
      message: `清单中的「${names}」不在产品库中。是否添加到产品库后继续保存？`,
      confirmText: '是',
      cancelText: '否',
    })
    if (ok) {
      try {
        const createdMap = {} // modelLower → productId
        for (const model of uniqueModels) {
          // 创建产品时带上该条目已填写的技术参数，避免产品库只有型号没有参数
          const firstItem = unknown.find(i => i.model?.trim().toLowerCase() === model.toLowerCase())
          const specs = firstItem?.mergedText || ''
          const created = await createProduct({ name: model, fullSpecs: specs })
          if (created?.id) {
            createdMap[model.toLowerCase()] = created.id
            if (!(Array.isArray(gp.value) ? gp.value : []).find(p => p.id === created.id)) {
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

  mf.tags = [...(Array.isArray(cplan.value.draft.tags) ? cplan.value.draft.tags : [])]
  mf.cm = cplan.value.draft.content || ''
  smm.value = true
}
function confirmSave() { if (!cplan.value?.draft) return; cplan.value.draft.tags = [...mf.tags]; cplan.value.draft.content = mf.cm.trim(); smm.value = false; sdv() }
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
      const match = allProducts.find(p => (p.name || '').toLowerCase() === model.toLowerCase())
      if (match) {
        item.productId = match.id
        item.model = match.name
        if (match.price && !item.unitPrice) item.unitPrice = match.price
        if (match.full_specs && !item.mergedText) item.mergedText = match.full_specs
        item.syncParams = false
      }
    }

    // Map frontend items to backend DeviceItemInput format
    const deviceItems = itemsArr
      .map(item => {
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
            paramMarks: [],
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
          // Persist selected parameter marks for read-only mode
          paramMarks: Array.isArray(item.params) ? item.params.map(p => ({
            paramId: p.id,
            mark: p.mark || '',
            indicator: p.indicator || '',
            requirement: p.requirement || '',
          })) : [],
        }
      })

    // Call the real backend API via the store
    let saveErr = null
    let r = null
    try {
      r = await projectStore.savePlanVersion(
        props.project.id,
        cplan.value.id,
        {
          versionNum: d.versionNum,
          tags: Array.isArray(d.tags) ? d.tags : [],
          content: d.content || '',
          operator: d.operator || op.value,
          deviceItems,
          // Pass original draft items so the store can preserve productId & params
          _originItems: itemsArr,
        }
      )
    } catch (e) { saveErr = e; console.error('[保存版本] savePlanVersion 失败:', e) }
    if (!saveErr && r) {
      // 映射后端字段名 → 前端字段名（与 fetchProjectDetail 中的映射一致）
      if (r.items && Array.isArray(r.items)) {
        r.items = r.items.map(di => ({
          uid: di.id,
          productId: di.productId || '',
          rowType: di.rowType || 'device',
          name: di.name || '',
          model: di.rowType === 'subnet' ? '' : (di.model || ''),
          customName: di.rowType === 'subnet' ? '' : (di.deviceName || ''),
          unitPrice: di.rowType === 'subnet' ? 0 : (di.price || 0),
          quantity: di.rowType === 'subnet' ? 0 : (di.count || 1),
          unit: di.rowType === 'subnet' ? '' : (di.unit || '台'),
          mergedText: di.rowType === 'subnet' ? '' : (di.parameters || ''),
          params: [],
        }))
      }
      cplan.value.history = [r, ...(cplan.value.history || [])]
    }

    // Sync: update product parameters for items with syncParams enabled (best-effort)
    const syncItems = itemsArr.filter(item => item.rowType !== 'subnet' && item.syncParams && item.productId && item.mergedText)
    if (syncItems.length > 0) {
      try {
        await Promise.allSettled(syncItems.map(item =>
          updateProduct(item.productId, { fullSpecs: item.mergedText })
        ))
      } catch (e) { console.warn('[保存] 同步产品参数失败', e) }
    }

    // Record dynamic for new version (best-effort)
    try {
      await projectStore.addProjectDynamic(props.project.id, `为方案「${cplan.value.name}」保存了版本 ${d.versionNum || 'V1.0'}`)
    } catch (e) { console.warn('[保存] 记录动态失败', e) }

    if (saveErr) throw saveErr

    // On success: keep current items as read-only copy, clear draft, return to timeline
    try {
      cplan.value.items = JSON.parse(JSON.stringify(d.items))
      cplan.value.draft = null
      clearDraft(cplan.value.id)
    } catch (e) { console.warn('[保存] 清理草稿失败', e) }
    gtl()
  } catch (e) {
    saveError.value = e.message || '保存失败，请重试'
    alert({ title: '保存失败', message: e.message || '网络异常，请检查后端服务是否启动' })
    console.error('版本存库失败', e)
  } finally {
    isSaving.value = false
  }
}

function gtl() { av.value = 'timeline'; vvi.value = null; aii.value = null; clearValidation() }
async function cancelDraft() { if (!cplan.value) return; const ok = await dangerConfirm({ title: '删除编辑中的版本', message: `确定删除正在编辑的版本 ${fmtVersion(cplan.value.draft?.versionNum) || ''}？未保存的修改将丢失，此操作不可撤销。`, confirmText: '删除' }); if (!ok) return; const pid = cplan.value.id; cplan.value.draft = null; clearDraft(pid); gtl() }
function vhv(vi) { vvi.value = vi; av.value = 'view'; aii.value = null }
function sed() { av.value = 'edit'; aii.value = null; clearValidation() }

function tdc(tag) { if (tag.includes('降本')) return 'fill-red-500'; if (tag.includes('扩容')) return 'fill-green-500'; if (tag.includes('微调')) return 'fill-yellow-500'; if (tag.includes('重构')) return 'fill-blue-500'; return 'fill-gray-400' }
function tc(tag) { if (tag.includes('降本')) return 'bg-red-100 text-red-600'; if (tag.includes('扩容')) return 'bg-green-100 text-green-600'; if (tag.includes('微调')) return 'bg-yellow-100 text-yellow-700'; if (tag.includes('重构')) return 'bg-blue-100 text-blue-600'; return 'bg-gray-100 text-gray-500' }
function tac(tag) { if (tag.includes('降本')) return 'bg-red-100 text-red-600 border-red-200'; if (tag.includes('扩容')) return 'bg-green-100 text-green-600 border-green-200'; if (tag.includes('微调')) return 'bg-yellow-100 text-yellow-700 border-yellow-200'; if (tag.includes('重构')) return 'bg-blue-100 text-blue-600 border-blue-200'; return 'bg-gray-200 text-gray-700 border-gray-300' }

async function evx(ver) {
  if (!ver) return
  const pn = cplan.value?.name || '方案'
  const prjn = props.project?.name || '项目'
  const its = Array.isArray(ver.items) ? ver.items : []

  // Pre-resolve product names from store (exportToPMSExcel only uses item.model)
  const resolved = its.map((item) => ({
    ...item,
    model: item.model || gpn(item.productId) || '',
  }))

  await exportToPMSExcel({
    projectName: prjn,
    planName: pn,
    versionLabel: ver.versionNum || '快照',
    totalAmount: ver.amount || 0,
    items: resolved,
  })
}

async function deleteVersionEntry(entry) {
  const ok = await dangerConfirm({ title: '删除版本', message: `确定删除版本 ${fmtVersion(entry.data.versionNum)}？此操作不可撤销。`, confirmText: '删除' })
  if (!ok) return
  const verId = entry.data.versionId || entry.data.id
  if (!verId) return
  try {
    await deleteVersion(verId)
    const plan = plans.value.find(p => p.id === apid.value)
    if (plan && Array.isArray(plan.history)) {
      plan.history = plan.history.filter(h => (h.versionId || h.id) !== verId)
    }
  } catch (e) {
    console.warn('删除版本失败', e)
  }
}

const copyToast = ref(false)
let copyToastTimer = null

// 复制当前选中条目的技术参数
function copyCurrentParams() {
  const text = li.value[aii.value]?.mergedText || ''
  if (!text) return
  const showToast = () => { copyToast.value = true; clearTimeout(copyToastTimer); copyToastTimer = setTimeout(() => { copyToast.value = false }, 1500) }
  navigator.clipboard.writeText(text).then(showToast).catch(() => {
    const ta = document.createElement('textarea'); ta.value = text; document.body.appendChild(ta); ta.select(); document.execCommand('copy'); document.body.removeChild(ta); showToast()
  })
}

const rpid = ref(null); const rv = ref(''); let planNameInput = null
const editingContentVerId = ref(null)
const editingContentVal = ref('')
function secv(entry) {
  if (!entry || !entry.data) return
  if (entry.type === 'draft') { editingContentVerId.value = '__draft__'; editingContentVal.value = entry.data.content || ''; return }
  if (entry.type !== 'ver') return
  const vid = entry.data.versionId || entry.data.id
  if (!vid) return
  editingContentVerId.value = vid
  editingContentVal.value = entry.data.content || ''
  // 失焦后自动聚焦输入框，确保 blur 语义可靠
  nextTick(() => { (document.querySelector('[data-remark-input]') as HTMLInputElement)?.focus() })
}
async function svcv() {
  const verId = editingContentVerId.value
  const val = editingContentVal.value.trim()
  editingContentVerId.value = null  // 失焦立即退出编辑态，后台保存
  if (!verId) return
  if (verId === '__draft__') {
    const draftEntry = ste.value.find(e => e.type === 'draft')
    if (draftEntry) draftEntry.data.content = val || ''
    return
  }
  const entry = ste.value.find(e => e.type === 'ver' && (e.data.versionId === verId || e.data.id === verId))
  if (!entry) return
  if (val === (entry.data.content || '')) return
  entry.data.content = val
  try { await updateSolutionVersionMeta(verId, { content: val }) } catch (e) { console.warn('更新方案版本备注失败', e) }
}
function ccvc() { editingContentVerId.value = null }
async function anp() {
  const planId = projectStore.addPlan(props.project.id, '新方案')
  if (planId) {
    plans.value = props.project.plans
    apid.value = planId
    await projectStore.addProjectDynamic(props.project.id, '创建了新方案「新方案」')
    gtl()
  }
}
function srp(plan) { rpid.value = plan.id; rv.value = plan.name; nextTick(() => { if (planNameInput && typeof planNameInput.focus === 'function') { planNameInput.focus(); planNameInput.select() } }) }
function crp() { const plan = plans.value.find(p => p.id === rpid.value); if (plan && rv.value.trim() && rv.value.trim() !== plan.name) { projectStore.updatePlan(props.project.id, plan.id, { name: rv.value.trim() }) }; rpid.value = null }
function crpc() { rpid.value = null }
async function rmp(id) { if (plans.value.length <= 1) return; const plan = plans.value.find(p => p.id === id); if (!plan) return; const vc = Array.isArray(plan.history) ? plan.history.length : 0; const hasDraft = !!plan.draft; const parts = []; if (vc > 0) parts.push(`${vc} 个历史版本`); if (hasDraft) parts.push('1 份草稿'); const detail = parts.length ? `该方案包含 ${parts.join(' 和 ')}，` : ''; const ok = await dangerConfirm({ title: '删除方案', message: `${detail}删除后数据将无法恢复。确定删除方案「${plan.name}」？`, confirmText: '删除' }); if (!ok) return; if (apid.value === id) apid.value = plans.value[0]?.id || ''; clearDraft(id); projectStore.deletePlan(props.project.id, id) }

watch(apid, () => { gtl() })
watch(li, () => { if (im.value && invalidItems.value.length) clearValidation() }, { deep: true })

// ─── 编辑态实时自动保存（防刷新丢失）──────────────────────────────────────────
const DRAFT_PREFIX = 'pms_draft_'
function draftKey(planId) { return DRAFT_PREFIX + props.project.id + '_' + planId }

/**
 * 静默保存草稿到 localStorage（防刷新/崩溃丢失）
 * 仅序列化当前 draft 对象，不触发后端 API
 */
const saveDraft = () => {
  if (!cplan.value?.draft) return
  try {
    const key = draftKey(cplan.value.id)
    const payload = JSON.parse(JSON.stringify(cplan.value.draft))
    localStorage.setItem(key, JSON.stringify(payload))
  } catch { /* quota exceeded */ }
}

function clearDraft(planId) {
  try { localStorage.removeItem(draftKey(planId)) } catch (e) { console.error('[Solution] clearDraft failed:', e) }
}

function restoreDrafts() {
  plans.value.forEach(plan => {
    try {
      const raw = localStorage.getItem(draftKey(plan.id))
      if (!raw) return
      const draft = JSON.parse(raw)
      if (!draft || !Array.isArray(draft.items) || draft.items.length === 0) { clearDraft(plan.id); return }
      // 只恢复未正式保存过的草稿（plan.history 中没有该版本号的已归档版本）
      const alreadyArchived = (Array.isArray(plan.history) ? plan.history : []).some(h => h.versionNum === draft.versionNum)
      if (alreadyArchived) { clearDraft(plan.id); return }
      draft.items.forEach(item => { if (item.syncParams === undefined) item.syncParams = item.productId ? false : true })
      plan.draft = draft
    } catch { /* corrupted */ }
  })
}

// ─── 防抖工具（自实现，无需外部依赖）─────────────────────────────────────────
function debounce(fn, delay = 1000) {
  let timer = null
  const debounced = (...args) => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => { timer = null; fn(...args) }, delay)
  }
  debounced.cancel = () => { if (timer) { clearTimeout(timer); timer = null } }
  return debounced
}

const debouncedSaveDraft = debounce(saveDraft, 1000)

// 监听草稿变更 → 1 秒防抖后写入 localStorage
watch(() => cplan.value?.draft, () => {
  debouncedSaveDraft()
}, { deep: true })

onMounted(async () => {
  document.addEventListener('click', cud)
  restoreDrafts()
  try {
    const raw = await fetchProducts()
    const products = Array.isArray(raw) ? raw : (Array.isArray(raw?.data) ? raw.data : Array.isArray(raw?.items) ? raw.items : [])
    gp.value = products.map(p => ({ ...p, name: p.model || p.name }))
  } catch (e) {
    console.warn('[ProductCatalog] 加载产品库失败', e)
  }
})
onUnmounted(() => {
  document.removeEventListener('click', cud)
  document.removeEventListener('mousemove', odm)
  document.removeEventListener('mouseup', oup)
  debouncedSaveDraft.cancel()
})

</script>

<template>
  <div class="flex h-full w-full bg-white overflow-hidden">
    <div class="w-[260px] border-r border-surface-100 flex flex-col bg-white flex-shrink-0">
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">方案清单</span>
        <button @click="anp" class="flex items-center justify-center w-5 h-5 rounded text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors" title="新增方案">
          <AppIcon name="plus" class="w-3.5 h-3.5" />
        </button>
      </div>
      <div class="flex-1 overflow-y-auto px-2 py-1.5 [&::-webkit-scrollbar]:w-1 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
        <div v-for="plan in splans" :key="plan.id" @click="apid = plan.id" class="relative flex items-center px-2 py-1.5 rounded-lg cursor-pointer transition-colors mb-0.5 group" :class="apid === plan.id ? 'bg-blue-50/70 text-brand-700 font-medium' : 'text-surface-600 hover:bg-surface-100'">
          <div v-if="apid === plan.id" class="absolute left-0 top-1 bottom-1 w-[3px] bg-blue-500 rounded-l pointer-events-none" />
          <input v-if="rpid === plan.id" :ref="el => planNameInput = el" v-model="rv" @blur="crp" @keyup.enter="crp" @keyup.escape="crpc" @click.stop class="flex-1 min-w-0 text-sm px-1 py-0.5 border border-blue-400 rounded outline-none focus:ring-1 focus:ring-blue-500" />
          <span v-else class="text-xs truncate block w-full flex-1 min-w-0">{{ plan.name }}</span>
          <span class="text-[11px] text-surface-400 font-mono flex-shrink-0 ml-2">{{ Array.isArray(plan.history) ? plan.history.length : 0 }}</span>
          <button @click.stop="srp(plan)" class="opacity-0 group-hover:opacity-100 flex-shrink-0 w-5 h-5 ml-1 text-surface-400 hover:text-brand-600 flex items-center justify-center"><AppIcon name="edit" class="w-3 h-3" /></button>
          <button v-if="plans.length > 1" @click.stop="rmp(plan.id)" class="opacity-0 group-hover:opacity-100 flex-shrink-0 w-5 h-5 ml-0.5 text-surface-400 hover:text-red-500 flex items-center justify-center"><AppIcon name="trash" class="w-3 h-3" /></button>
        </div>
      </div>
    </div>

    <!-- Middle Panel: Timeline / Device List -->
    <div class="w-[360px] flex-shrink-0 border-r border-surface-100 flex flex-col bg-white/50 overflow-hidden">

            <template v-if="av === 'timeline'">
        <div class="h-10 px-3 flex items-center border-b border-surface-100 shrink-0">
          <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">方案版本</span>
        </div>
        <div class="flex-1 overflow-y-auto py-3 px-3 min-h-0 bg-gray-50/30 space-y-2 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">

          <!-- 草稿卡片 -->
          <div v-if="cplan?.draft" @click="sed" class="flex items-center gap-2 bg-white rounded-lg border px-3 py-2 cursor-pointer transition-shadow hover:shadow-sm group/card border-amber-200 shadow-sm">
            <div class="w-2 h-2 rounded-full bg-amber-400 flex-shrink-0" />
            <span class="text-xs font-medium text-amber-600 shrink-0">{{ fmtVersion(cplan.draft.versionNum) }}</span>
            <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-blue-100 text-blue-600 shrink-0">编辑中</span>
            <span class="text-[11px] text-surface-400 truncate flex-1 min-w-0">{{ cplan.draft.content || '草稿' }}</span>
            <span class="text-xs font-semibold text-slate-700 font-mono shrink-0">¥{{ gt.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span>
            <button @click.stop="cancelDraft" class="btn-ghost !p-0.5 text-slate-300 hover:text-red-500 opacity-0 group-hover/card:opacity-100 shrink-0" title="删除草稿"><AppIcon name="x" class="w-3 h-3" /></button>
          </div>

          <!-- 空状态 -->
          <div v-if="!cplan?.draft && ste.length === 0" class="text-center py-12 text-gray-400 text-xs space-y-3"><p>暂无版本记录</p><button @click="omm" class="btn-secondary"><AppIcon name="plus-thin" class="w-3.5 h-3.5" />新增版本</button></div>

          <!-- 已归档版本卡片 -->
          <div v-for="(entry, idx) in ste.filter(e => e.type === 'ver')" :key="entry.data.versionId || entry.data.id"
            @click="vhv(entry.idx)"
            class="bg-white rounded-lg border px-3 py-2.5 cursor-pointer transition-shadow hover:shadow-sm group/ver"
            :class="idx === 0 && !cplan?.draft ? 'border-brand-200 shadow-sm' : 'border-surface-100'">
            <!-- Row 1: dot + version + tags + content preview + actions -->
            <div class="flex items-center gap-2 mb-1.5">
              <div class="w-2 h-2 rounded-full flex-shrink-0" :class="idx === 0 && !cplan?.draft ? 'bg-brand-500' : 'bg-surface-300'" />
              <span class="text-xs font-semibold shrink-0 min-w-[28px]" :class="idx === 0 && !cplan?.draft ? 'text-brand-600' : 'text-surface-600'">{{ fmtVersion(entry.data.versionNum) }}</span>
              <div class="flex items-center gap-1 shrink-0" v-if="entry.data.tags && entry.data.tags.length">
                <span v-for="tag in entry.data.tags.slice(0,2)" :key="tag" class="px-1.5 py-0.5 rounded text-[9px] font-medium flex-shrink-0" :class="tc(tag)">{{ tag }}</span>
                <span v-if="entry.data.tags.length > 2" class="text-[9px] text-surface-400">+{{ entry.data.tags.length - 2 }}</span>
              </div>
              <span class="text-[10px] text-surface-400 shrink-0">{{ formatDateTime(entry.data.createdAt) }}</span>
              <span class="text-xs font-semibold text-slate-700 font-mono shrink-0">¥{{ (entry.data.amount || 0).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span>
              <div class="flex-1" />
              <button @click.stop="evx(entry.data)" class="btn-ghost !p-0.5 text-slate-300 hover:text-blue-600 opacity-0 group-hover/ver:opacity-100 shrink-0" title="下载 Excel"><AppIcon name="download" class="w-3 h-3" /></button>
            </div>
            <!-- Row 2: Commit message / remark (click to edit) -->
            <div class="flex items-center gap-1 ml-4">
              <template v-if="editingContentVerId === (entry.data.versionId || entry.data.id)">
                <input v-model="editingContentVal" @keyup.enter="svcv" @keyup.escape="ccvc" @blur="svcv" data-remark-input class="w-full text-[11px] bg-white border border-blue-400 rounded px-1.5 py-0.5 outline-none focus:ring-1 focus:ring-blue-200" placeholder="输入备注..." @click.stop />
              </template>
              <span v-else @click.stop="secv(entry)" class="text-[11px] cursor-text rounded px-1 -mx-1 py-0.5 hover:bg-surface-50 transition-colors flex-1 min-w-0 truncate" :class="entry.data.content ? 'text-surface-600' : 'text-surface-400 italic'">{{ entry.data.content || '点击添加版本说明' }}</span>
            </div>
          </div>
        </div>
      </template>

            <template v-else-if="av === 'edit'">
        <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
          <div class="flex items-center gap-1 min-w-0">
            <button @click="gtl" class="text-surface-400 hover:text-brand-600 p-0.5 flex-shrink-0"><AppIcon name="arrow-left" class="w-4 h-4" /></button>
            <span class="text-xs font-medium text-surface-700 truncate">{{ cpname }}</span>
            <span class="px-1.5 py-0.5 rounded text-[10px] font-medium border border-surface-200 text-surface-500 shrink-0">{{ fmtVersion(cplan?.draft?.versionNum) || '草稿' }}</span>
          </div>
          <div class="flex items-center gap-1 ml-auto flex-nowrap">
            <button @click="apl" class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0" title="添加设备"><AppIcon name="plus" class="w-3.5 h-3.5" /></button>
            <button @click="asub" class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0" title="子网"><AppIcon name="folder" class="w-3.5 h-3.5" /></button>
            <button @click="showUploadModal = true" class="h-6 w-6 flex items-center justify-center rounded-md text-surface-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0" title="导入"><AppIcon name="upload" class="w-3.5 h-3.5" /></button>
            <button @click="preSave" :disabled="isSaving" class="flex items-center gap-1 h-6 px-2 rounded-md text-xs bg-blue-600 text-white hover:bg-blue-700 disabled:bg-blue-400 disabled:cursor-wait transition-colors flex-shrink-0 whitespace-nowrap"><AppIcon v-if="isSaving" name="spinner" class="animate-spin w-3 h-3" /><AppIcon v-else name="save" class="w-3 h-3" /><span>{{ isSaving ? '保存中...' : '保存' }}</span></button>
          </div>
        </div>
        <div v-if="saveError" class="px-3 py-2 bg-red-50 border-b border-red-100 text-[11px] text-red-600 flex items-center gap-1.5 shrink-0"><AppIcon name="alert-circle" class="w-3 h-3 flex-shrink-0" /><span>{{ saveError }}</span></div>
        <div class="flex-1 overflow-y-auto p-3 space-y-2 min-h-0 pr-1 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full hover:[&::-webkit-scrollbar-thumb]:bg-gray-300">
          <div v-if="li.length === 0" class="text-center py-12 text-gray-400 text-xs">清空状态。请增加设备节点。</div>
          <template v-for="(item, idx) in li" :key="item.uid">
            <!-- Subnet Separator Card -->
            <div v-if="item.rowType === 'subnet'" @click="aii = idx" :data-item-idx="idx" class="group relative flex flex-row transition-all duration-200 overflow-hidden rounded-lg cursor-pointer" :class="[aii === idx ? 'border-2 border-blue-400 bg-blue-50/20 ring-2 ring-blue-400/20 ring-offset-0 z-10' : idr && dti === idx ? 'border-2 border-blue-400 border-dashed bg-blue-50/30' : idr && dsi === idx ? 'opacity-40' : 'border-2 border-blue-200/60 bg-blue-50/30 hover:border-blue-300 hover:bg-blue-50/50']">
              <!-- Insert line above -->
              <div v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'above'" class="absolute -top-[3px] left-0 right-0 z-20 flex justify-center pointer-events-none"><div class="h-[5px] w-[80%] bg-blue-500 rounded-full shadow-sm" /></div>
              <!-- Insert line below -->
              <div v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'below'" class="absolute -bottom-[3px] left-0 right-0 z-20 flex justify-center pointer-events-none"><div class="h-[5px] w-[80%] bg-blue-500 rounded-full shadow-sm" /></div>
              <div data-grip @mousedown="ogd($event, idx)" class="w-6 bg-blue-100/60 border-r border-blue-200/40 hover:bg-blue-200/60 cursor-grab flex-shrink-0 flex items-center justify-center text-blue-400 transition-colors"><AppIcon name="grip-vertical" class="w-3 h-3" /></div>
              <div class="flex-1 flex items-center px-2 py-1 gap-1 min-w-0">
                <AppIcon name="folder" class="w-3 h-3 text-blue-400 flex-shrink-0" />
                <input v-model="item.name" @click.stop type="text" placeholder="子网名称" class="flex-1 min-w-0 bg-transparent text-[12px] h-5 py-0 px-1 text-blue-700 font-medium outline-none focus:bg-white/80 focus:ring-1 focus:ring-blue-300 rounded placeholder:text-blue-300" />
                <button @click.stop="rpl(idx)" class="flex-shrink-0 w-4 h-4 rounded-full border border-blue-200 bg-white text-blue-300 flex items-center justify-center transition hover:text-red-500 hover:border-red-200 shadow-sm"><AppIcon name="x" class="w-2.5 h-2.5" /></button>
              </div>
            </div>

            <!-- Device Card -->
            <div v-else @click="aii = idx" :data-item-idx="idx" class="group relative flex flex-row transition-all duration-200 overflow-hidden" :class="[aii === idx ? 'border border-blue-400 bg-blue-50/10 ring-2 ring-blue-400/20 ring-offset-0 z-10 rounded-lg cursor-pointer' : idr && dti === idx ? 'border-blue-400 border-dashed bg-blue-50/30 rounded-lg' : idr && dsi === idx ? 'border border-gray-200 bg-white shadow-sm rounded-lg opacity-40 cursor-pointer' : 'border border-gray-200 bg-white shadow-sm hover:shadow hover:border-gray-300 rounded-lg cursor-pointer', invalidItems.includes(item.uid) ? 'ring-1 ring-red-200 border-red-300 bg-red-50/30' : '']">
              <!-- Insert line above -->
              <div v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'above'" class="absolute -top-[3px] left-0 right-0 z-20 flex justify-center pointer-events-none"><div class="h-[5px] w-[80%] bg-blue-500 rounded-full shadow-sm" /></div>
              <!-- Insert line below -->
              <div v-if="idr && dragInsert?.targetIdx === idx && dragInsert?.side === 'below'" class="absolute -bottom-[3px] left-0 right-0 z-20 flex justify-center pointer-events-none"><div class="h-[5px] w-[80%] bg-blue-500 rounded-full shadow-sm" /></div>
              <div data-grip @mousedown="ogd($event, idx)" class="w-6 bg-gray-50/60 border-r border-gray-100 hover:bg-gray-200/60 cursor-grab flex-shrink-0 flex items-center justify-center text-gray-400 transition-colors"><AppIcon name="grip-vertical" class="w-3 h-3" /></div>
              <div class="flex-1 flex flex-col p-1 gap-0.5 min-w-0">
                <div class="flex items-center gap-1">
                  <input v-model="item.customName" @click.stop type="text" placeholder="自定义名称" class="flex-1 min-w-0 bg-gray-50/80 border border-gray-200 rounded text-[12px] h-5 py-0 px-1.5 text-gray-700 text-center outline-none focus:border-[#007aff] focus:ring-1 focus:ring-blue-500 focus:bg-white placeholder:text-gray-350" />
                  <button @click.stop="rpl(idx)" class="flex-shrink-0 w-4 h-4 rounded-full border border-gray-200 bg-white text-gray-400 flex items-center justify-center transition hover:text-red-500 hover:border-red-200 shadow-sm"><AppIcon name="x" class="w-2.5 h-2.5" /></button>
                </div>
                <div class="flex items-center gap-1">
                  <input
                    :value="item.model"
                    @input="handleDeviceInput($event, item)"
                    @keydown="onDeviceKeydown($event)"
                    @keyup.enter="handleDeviceConfirm(item, $event)"
                    @blur="handleDeviceConfirm(item, $event)"
                    @click.stop
                    type="text"
                    placeholder="输入设备型号..."
                    class="flex-1 min-w-0 bg-gray-50/80 border border-gray-200 rounded text-[12px] h-5 py-0 pl-1.5 pr-2 text-gray-700 outline-none focus:border-[#007aff] focus:ring-1 focus:ring-blue-500 focus:bg-white"
                    :class="invalidItems.includes(item.uid) && !item.productId && !item.model ? 'border-red-300 bg-red-50/50' : ''"
                  />
                  <AppIcon v-if="invalidItems.includes(item.uid) && !item.productId && !item.model" name="alert-circle" class="w-3.5 h-3.5 text-red-400 flex-shrink-0" />
                  <span v-if="!item.model" class="shrink-0 inline-flex items-center px-2 py-1 rounded-md text-xs font-medium bg-gray-100 text-gray-400 ring-1 ring-inset ring-gray-200">无设备</span>
                  <span v-else-if="item.productId" class="shrink-0 inline-flex items-center px-2 py-1 rounded-md text-xs font-medium bg-green-50 text-green-600 ring-1 ring-inset ring-green-500/10"><AppIcon name="check" class="w-3 h-3 mr-1"/>已匹配</span>
                  <span v-else class="shrink-0 inline-flex items-center px-2 py-1 rounded-md text-xs font-medium bg-amber-50 text-amber-600 ring-1 ring-inset ring-amber-500/10"><AppIcon name="plus" class="w-3 h-3 mr-1"/>新设备</span>
                </div>
                <div class="flex items-center gap-1">
                  <div class="relative w-[103px]"><span class="absolute left-1 top-1/2 -translate-y-1/2 text-[11px] text-gray-400 pointer-events-none z-10">¥</span><input v-model.number="item.unitPrice" @click.stop @focus="$event.target.select()" @blur="item.quantity = Math.max(1, item.quantity || 1)" type="number" min="0" step="100" placeholder="0" class="w-full bg-transparent rounded text-[12px] h-5 py-0 pl-3.5 pr-1 text-right text-gray-700 outline-none hover:bg-gray-50 focus:bg-white focus:ring-1 focus:ring-blue-200 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none" :class="invalidItems.includes(item.uid) && (!item.unitPrice || item.unitPrice <= 0) ? 'bg-red-50/50' : ''" /></div>
                  <span class="text-[11px] text-gray-400 flex-shrink-0">×</span>
                  <input v-model.number="item.quantity" @click.stop @focus="$event.target.select()" @blur="item.quantity = Math.max(1, item.quantity || 1)" type="number" min="1" placeholder="1" class="w-11 h-5 py-0 px-1 text-center text-xs text-gray-700 bg-transparent rounded outline-none hover:bg-gray-50 focus:bg-white focus:ring-1 focus:ring-blue-200 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none" />
                  <button @click.stop="tud(item.uid, $event)" data-unit-dropdown class="w-[42px] h-5 flex items-center justify-center gap-px text-[11px] text-gray-500 bg-transparent rounded hover:bg-gray-50 transition-colors outline-none flex-shrink-0"><span>{{ item.unit || '台' }}</span><AppIcon name="chevron-down" class="w-2.5 h-2.5 text-gray-300 flex-shrink-0" /></button>
                  <div class="flex-1 min-w-0 flex items-center justify-end"><span class="text-xs text-blue-600 font-mono truncate">¥{{ ((item.unitPrice || 0) * (item.quantity || 0)).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span></div>
                </div>
              </div>
            </div>
          </template>
        </div>
        <div class="shrink-0 border-t border-gray-200 bg-gray-50/80 px-4 py-2.5 flex items-center justify-between"><span class="text-xs font-semibold text-gray-600 uppercase tracking-wider">总计方案报价</span><span class="text-sm font-bold text-red-600">¥ {{ gt.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span></div>
      </template>

            <template v-else-if="av === 'view'">
        <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
          <div class="flex items-center gap-1 min-w-0">
            <button @click="gtl" class="text-surface-400 hover:text-brand-600 p-0.5 flex-shrink-0"><AppIcon name="arrow-left" class="w-4 h-4" /></button>
            <span class="text-xs font-medium text-surface-700 truncate">{{ cpname }}</span>
            <span class="px-1.5 py-0.5 rounded text-[10px] font-medium border border-surface-200 text-surface-500 shrink-0">{{ fmtVersion(vv?.versionNum) }}</span>
            <span class="text-[10px] text-surface-400 ml-1 flex-shrink-0" v-if="vv">{{ formatDateTime(vv.createdAt) }}</span>
            <span v-if="vv?.operator && vv.operator !== op" class="text-[10px] text-surface-400 ml-1 flex-shrink-0">{{ vv.operator }}</span>
          </div>
          <div class="flex items-center gap-2 flex-shrink-0">
            <button @click="omm" class="btn-primary">
              <AppIcon name="edit" class="w-3 h-3" />
              修改
            </button>
          </div>
        </div>
        <div class="flex-1 overflow-y-auto p-3 space-y-2 min-h-0 pr-1 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
          <div v-if="li.length === 0" class="text-center py-12 text-gray-400 text-xs">此版本为空。</div>
          <template v-for="(item, idx) in li" :key="item.uid">
            <!-- Subnet separator (view mode) -->
            <div v-if="item.rowType === 'subnet'" @click="aii = idx" class="border-2 border-blue-200/50 bg-blue-50/20 rounded-lg overflow-hidden cursor-pointer" :class="aii === idx ? 'border-blue-400 bg-blue-100/30' : ''">
              <div class="flex items-center justify-center px-2 py-1 gap-1">
                <AppIcon name="folder" class="w-3 h-3 text-blue-400 flex-shrink-0" />
                <span class="text-[12px] font-medium text-blue-600">{{ item.name || '未命名子网' }}</span>
              </div>
            </div>
            <!-- Device card (view mode) -->
            <div v-else @click="aii = idx" class="border border-gray-200 bg-white rounded-lg overflow-hidden" :class="aii === idx ? 'border-blue-400 bg-blue-50/10 ring-2 ring-blue-400/20 ring-offset-0 cursor-pointer' : 'cursor-pointer'">
              <div class="flex flex-row">
                <div class="w-6 bg-gray-50/60 border-r border-gray-100 flex-shrink-0 flex items-center justify-center text-gray-300"><AppIcon name="grip-vertical" class="w-3 h-3" /></div>
                <div class="flex-1 px-2 py-1 gap-0.5 min-w-0">
                  <div class="text-[12px] font-medium text-gray-700 truncate mb-1">{{ item.customName || '（未命名）' }}</div>
                  <div class="text-[11px] text-gray-400">{{ item.model || gpn(item.productId) || '未指定设备' }}</div>
                  <div class="flex items-center gap-2 mt-1">
                    <span class="text-[11px] text-gray-500">¥{{ item.unitPrice?.toLocaleString('zh-CN') || 0 }}</span>
                    <span class="text-[11px] text-gray-400">×{{ item.quantity }} {{ item.unit }}</span>
                    <span class="text-[11px] font-semibold text-[#007aff] ml-auto">¥{{ ((item.unitPrice || 0) * (item.quantity || 0)).toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span>
                </div>
              </div>
            </div>
            </div>
          </template>
        </div>
        <div class="shrink-0 border-t border-gray-200 bg-gray-50/80 px-4 py-2.5 flex items-center justify-between"><span class="text-xs font-semibold text-gray-600 uppercase tracking-wider">总计方案报价</span><span class="text-sm font-bold text-red-600">¥ {{ gt.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span></div>
      </template>

    </div>

    <!-- Right Panel: 参数配置排版区 -->
    <div v-if="av !== 'timeline'" class="flex-1 flex flex-col bg-white relative overflow-hidden">
      <div class="h-10 px-3 flex items-center justify-between border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">参数配置排版区</span>
        <div class="flex items-center gap-3">
          <button v-if="aii !== null && li[aii]?.mergedText" @click="copyCurrentParams" class="text-[#007aff] hover:underline text-xs font-medium"><AppIcon name="clipboard" class="w-3 h-3 inline-block mr-0.5 -mt-px" />复制</button>
          <label v-if="im && aii !== null" class="flex items-center gap-1.5 cursor-pointer select-none">
            <input type="checkbox" v-model="li[aii].syncParams" class="w-3 h-3 rounded border-gray-300 text-blue-600 focus:ring-blue-500" />
            <span class="text-[11px] text-slate-500">更新产品参数</span>
          </label>
        </div>
      </div>
      <div class="flex-1 p-3 flex flex-col relative">
        <div v-if="aii === null || li.length === 0" class="h-full flex flex-col items-center justify-center text-gray-300"><p>在左侧选中设备进行客户化编辑</p></div>
        <div v-else-if="li[aii]?.rowType === 'subnet'" class="h-full flex flex-col items-center justify-center text-gray-400 space-y-2">
          <AppIcon name="folder" class="w-8 h-8 text-blue-300" />
          <p class="text-sm">{{ li[aii].name || '未命名子网' }}</p>
          <p class="text-xs text-gray-300">子网分隔标记，无参数配置</p>
        </div>
        <div v-else-if="aii == null || !li[aii] || (!li[aii].productId && !li[aii].model && !li[aii].mergedText && ir)" class="h-full flex items-center justify-center text-gray-400 text-sm">请先为该项指定设备型号。</div>
        <div v-else class="h-full flex flex-col">
          <textarea v-model="li[aii].mergedText" :readonly="ir" class="flex-1 w-full p-4 border border-gray-200 rounded-xl focus:border-[#007aff] focus:ring-4 focus:ring-blue-500/10 outline-none resize-none text-[13px] text-gray-800 leading-relaxed bg-[#f9f9fb] transition-all" :class="ir ? 'cursor-default select-text' : ''" :placeholder="ir ? '' : '该设备没有参数，请手动输入'"></textarea></div>
      </div>
    </div>
  </div>

  <!-- Modify Modal -->
  <Teleport to="body">
    <div v-if="smm" class="fixed inset-0 z-[80] flex items-center justify-center bg-gray-900/40 backdrop-blur-sm">
      <div class="bg-white rounded-2xl shadow-2xl w-full max-w-md overflow-hidden">
        <div class="px-6 py-4 border-b border-gray-100"><h3 class="text-sm font-semibold text-gray-800">保存版本 — {{ cpname }}</h3></div>
        <div class="p-6 space-y-4">
          <div><label class="block text-[11px] font-medium text-gray-500 mb-1.5">变更标签</label><div class="flex flex-wrap gap-1.5"><button v-for="tag in atags" :key="tag" @click="tmt(tag)" class="px-2.5 py-1 rounded-full text-[11px] font-medium transition-colors border" :class="mf.tags.includes(tag) ? tac(tag) : 'bg-gray-50 text-gray-400 border-gray-200 hover:border-gray-300'"><svg class="w-1.5 h-1.5 inline-block mr-1 -mt-px" viewBox="0 0 6 6"><circle cx="3" cy="3" r="3" :class="tdc(tag)"/></svg>{{ tag }}</button></div></div>
          <div><label class="block text-[11px] font-medium text-gray-500 mb-1">变更说明</label><textarea v-model="mf.cm" rows="2" placeholder="记录此次变更的目的或客户反馈..." class="w-full text-[12px] px-3 py-2 border border-gray-200 rounded-lg outline-none focus:border-[#007aff] focus:ring-1 focus:ring-blue-500 resize-none"></textarea></div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 bg-gray-50/50 flex justify-end gap-2">
          <button @click="smm = false" class="btn-ghost">取消</button>
          <button @click="confirmSave" class="btn-primary">确认保存</button>
        </div>
      </div>
    </div>
  </Teleport>

  <!-- Parameter Modal -->
  <Teleport to="body">
    <div v-if="imo" class="fixed inset-0 z-[80] flex items-center justify-center bg-gray-900/40 backdrop-blur-sm">
      <div class="bg-white/90 backdrop-blur-xl rounded-2xl shadow-2xl w-full max-w-4xl max-h-[85vh] flex flex-col overflow-hidden border border-white/20">
        <div class="px-6 py-4 border-b border-gray-200/50 flex justify-between items-center bg-white/50 shrink-0">
          <div><span class="text-xs text-gray-400 font-medium uppercase tracking-wider">参数资源库</span><h3 class="text-lg font-semibold text-slate-800">{{ gpn(ms.pid) }}</h3></div>
        </div>
        <div class="flex-1 overflow-y-auto p-6 bg-[#f5f5f7]/50 flex flex-col">
          <div class="flex items-center mb-5">
            <div class="relative flex-1">
              <AppIcon name="search" class="w-4 h-4 text-gray-400 absolute left-3 top-1/2 transform -translate-y-1/2" />
              <input v-model="ms.sq" type="text" placeholder="搜索指标名称或参数要求..." class="w-full pl-9 pr-8 py-2 text-[13px] bg-white border border-gray-200 rounded-lg outline-none focus:border-[#007aff] focus:ring-1 focus:ring-blue-500 shadow-sm"/>
              <button v-if="ms.sq" @click="ms.sq = ''" class="absolute right-3 top-1/2 transform -translate-y-1/2 text-gray-400 hover:text-gray-600"><AppIcon name="x" class="w-3 h-3" /></button>
            </div>
          </div>
          <div class="space-y-3">
            <div v-for="param in fmp" :key="param.id" @click="tcb(param.id)" class="flex items-center p-3 border rounded-xl bg-white cursor-pointer transition-all duration-200 shadow-sm" :class="ms.ts.includes(param.id) ? 'border-[#007aff] ring-1 ring-[#007aff]/50' : 'border-gray-200'">
              <div class="flex items-center mr-3">
                <button @click.stop="tpm(param.id)" class="w-16 h-6 mr-3 flex-shrink-0 flex items-center justify-center border rounded-md text-[11px] font-medium transition-all" :class="gmbc(ms.tm[param.id])" title="点击切换状态">{{ gmbt(ms.tm[param.id]) }}</button>
                <input type="checkbox" :value="param.id" v-model="ms.ts" @click.stop class="w-4 h-4 text-[#007aff] border-gray-300 rounded focus:ring-[#007aff]"/>
              </div>
              <div class="flex-1"><div class="font-bold text-gray-800 text-[13px] mb-1">{{ param.indicator }}</div><div class="text-[12px] text-gray-500 leading-relaxed">{{ param.requirement }}</div></div>
            </div>
            <div v-if="fmp.length === 0" class="text-center py-8 text-gray-400 text-sm">未匹配到参数</div>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-200/50 bg-white/80 flex justify-end items-center gap-3 shrink-0">
          <button @click="clm" class="px-5 py-2 text-[13px] font-medium text-gray-600 bg-gray-100 rounded-lg hover:bg-gray-200 transition-colors">取消</button>
          <button @click="cms" class="h-6 px-2 rounded-md text-xs font-medium text-white bg-blue-600 hover:bg-blue-700 transition-colors shadow-sm">确认 ({{ ms.ts.length }})</button>
        </div>
      </div>
    </div>
  </Teleport>

  <!-- Unit Dropdown (Teleported to avoid clipping) -->
  <Teleport to="body">
    <div v-if="uo" data-unit-dropdown class="fixed z-[80] bg-white/90 backdrop-blur-md shadow-lg border border-gray-100 rounded-lg py-1 min-w-[60px]" :style="{ top: upos.top + 'px', left: upos.left + 'px' }">
      <button v-for="u in units" :key="u" @click.stop="sud(u)" class="w-full px-3 py-1.5 text-sm text-left transition-colors hover:bg-blue-50 hover:text-blue-600" :class="li.find(i => i.uid === uo)?.unit === u ? 'text-[#007aff] font-medium' : 'text-surface-600'">{{ u }}</button>
    </div>
  </Teleport>

  <!-- Hidden PMS file input -->
  <input ref="pmsFileInput" type="file" class="hidden" accept=".xlsx,.xls" @change="onPMSFilePickedFromInput" />

  <!-- Import Excel Modal (PMS / SCT) -->
  <Teleport to="body">
    <div v-if="showUploadModal" class="fixed inset-0 z-[80] flex items-center justify-center bg-gray-900/40 backdrop-blur-sm" @click.self="cancelImport">
      <div class="bg-white rounded-2xl shadow-2xl w-full max-w-3xl mx-4 flex flex-col overflow-hidden transition-all duration-200">
        <div class="px-6 py-4 border-b border-gray-100 flex items-center justify-between shrink-0">
          <h3 class="text-sm font-semibold text-gray-800">上传 Excel</h3>
          <button @click="cancelImport" class="text-gray-400 hover:text-gray-600">
            <AppIcon name="x" class="w-4 h-4" />
          </button>
        </div>

        <div class="p-6">
          <div
            @click="triggerPMSFile"
            class="border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-colors border-slate-200 hover:border-blue-400 hover:bg-blue-50/30"
          >
            <AppIcon name="upload" class="w-10 h-10 mx-auto mb-3 text-slate-300" />
            <p v-if="!pmsFileName" class="text-sm text-slate-400">点击选择 Excel 文件</p>
            <p v-else class="text-sm text-blue-600 font-medium truncate">{{ pmsFileName }}</p>
          </div>
          <div v-if="isPMSImporting" class="mt-4 flex items-center gap-2 text-xs text-blue-600 justify-center">
            <AppIcon name="spinner" class="w-3.5 h-3.5 animate-spin" />
            正在解析 Excel...
          </div>
          <div v-if="pmsImportError" class="mt-4 p-3 bg-red-50 border border-red-100 rounded-lg text-xs text-red-600">
            {{ pmsImportError }}
          </div>
          <div class="mt-4 p-3 bg-slate-50 border border-slate-100 rounded-lg text-[11px] text-slate-500 leading-relaxed">
            <p class="font-medium text-slate-600 mb-1">格式要求（.xlsx）</p>
            <p>表头行须包含以下列，列顺序固定：</p>
            <p class="mt-1 font-mono text-[10px]">A:序号 B:产品型号 C:产品参数 D:数量 E:单位 F:单价 G:小计</p>
            <p class="mt-1">含「设备名称」列时：</p>
            <p class="font-mono text-[10px]">A:序号 B:设备名称 C:产品型号 D:产品参数 E:数量 F:单位 G:单价 H:小计</p>
            <p class="mt-1">· 序号列为数字，遇到「合计/总计」行停止解析</p>
            <p>· 支持子网（序号列合并多行的单元格为子网名称，从属行自动归入该子网）</p>
            <p>· 产品型号会自动匹配产品库，已有型号自动填入参数</p>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-gray-100 flex justify-end gap-3 shrink-0">
          <button @click="cancelImport" class="btn-secondary">取消</button>
          <button @click="confirmPMSImport" :disabled="!pmsSelectedFile || isPMSImporting" class="btn-primary">
            <AppIcon v-if="isPMSImporting" name="spinner" class="animate-spin w-3 h-3" />
            <span>{{ isPMSImporting ? '解析中...' : '确认' }}</span>
          </button>
        </div>
      </div>
    </div>
  </Teleport>

  <!-- Copy toast -->
  <Teleport to="body">
    <Transition name="toast-fade">
      <div v-if="copyToast" class="fixed bottom-8 left-1/2 -translate-x-1/2 z-[100] text-[12px] text-slate-400 bg-white/90 backdrop-blur-sm border border-slate-200 rounded-lg px-4 py-1.5 shadow-md">
        已复制到剪贴板
      </div>
    </Transition>
  </Teleport>

  <!-- Drag ghost -->
  <Teleport to="body">
    <div v-if="idr" class="fixed px-3 py-1.5 bg-blue-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{ left: (dragX + 12) + 'px', top: (dragY + 12) + 'px' }">
      <AppIcon name="grip-vertical" class="w-3 h-3 inline-block mr-1.5 -mt-px" />
      {{ li[dsi]?.customName || li[dsi]?.name || li[dsi]?.model || '拖动' }}
    </div>
  </Teleport>
</template>

<style scoped>
.toast-fade-enter-active,
.toast-fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}
.toast-fade-enter-from,
.toast-fade-leave-to {
  opacity: 0;
  transform: translate(-50%, 4px);
}
</style>
