<script setup lang="ts">
import { ref, computed, onMounted, watch, onUnmounted, onActivated, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useProductStore, UNCATEGORIZED_NAME } from '@/stores/product'
import { useSelectionStore } from '@/stores/selection'
import { confirm, dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import AppIcon from '@/components/common/AppIcon.vue'
import { uploadFile, listMaterials, deleteMaterial, previewFile, exportFile, getMaterialAssociations, updateMaterialAssociations, updateFileMeta, exportWriteBuffer, removeTempFile } from '@/api/db-client'
import { exportProductsToExcel } from '@/utils/productExcel'
import ExcelImportWizard from '@/components/shared/ExcelImportWizard.vue'
import type { ImportField, RawDataRow, SheetData } from '@/utils/excelRaw'
import { useUploadProgress } from '@/composables/useUploadProgress'
import { useResizablePanels } from '@/composables/useResizablePanels'
import { useMultiSelect } from '@/composables/useMultiSelect'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { pasteShortcut } from '@/utils/platform'
import { formatDateTime } from '@/utils/time'
import { getCurrentWebview } from '@tauri-apps/api/webview'

const store = useProductStore()
const route = useRoute()
const router = useRouter()
const selectionStore = useSelectionStore()

// ─── Layout ──────────────────────────────────────────────────────────
const { sidebarWidth, listWidth, isResizing, onResizeStart } = useResizablePanels()

const seriesSearch = ref(''); const selectedSeriesId = ref<string|null>(null)
const editingSeriesId = ref<string|null>(null); const seriesEditName = ref('')
const collapsedSeries = ref<Set<string>>(new Set())
const modalCollapsed = ref<Record<string, boolean>>({})
function toggleSeries(id: string) { if (collapsedSeries.value.has(id)) collapsedSeries.value.delete(id); else collapsedSeries.value.add(id); collapsedSeries.value = new Set(collapsedSeries.value) }
function toggleModalSeries(id: string) { const m = { ...modalCollapsed.value }; m[id] = !m[id]; modalCollapsed.value = m }
function selectSeries(id: string|null) { selectedSeriesId.value = id; selectedProductId.value = null; clearMultiSelect(); store.selectSeries(id) }
async function startAddSeries(parentId?: string|null) {
  let name = '新目录'
  let i = 1
  while (isDuplicateSeriesName(name, undefined, parentId || null)) { name = '新目录 ' + (++i) }
  const r: any = await store.createSeries({ name, parentId: parentId || null }); await store.fetchSeries(); if (parentId) { collapsedSeries.value.delete(parentId); collapsedSeries.value = new Set(collapsedSeries.value) }; editingSeriesId.value = r?.id || r?.series?.id || null; seriesEditName.value = name; setTimeout(() => { const el = document.querySelector<HTMLInputElement>('[data-series-edit]'); el?.focus(); el?.select() }, 50)
}
function startEditSeries(id: string) { editingSeriesId.value = id; const s = store.series.find((x: any) => x.id === id); seriesEditName.value = s?.name || ''; setTimeout(() => { const el = document.querySelector<HTMLInputElement>('[data-series-edit]'); el?.focus(); el?.select() }, 50) }
function isDuplicateSeriesName(name: string, excludeId?: string, parentId?: string|null): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return (store.series as any[]).some((s: any) => s.id !== excludeId && s.name.toLowerCase() === q && (s.parentId || null) === (parentId || null))
}
async function saveSeriesName(id: string) {
  const n = seriesEditName.value.trim(); if (!n) { editingSeriesId.value = null; return }
  const s = (store.series as any[]).find((x: any) => x.id === id)
  if (isDuplicateSeriesName(n, id, s?.parentId || null)) { toast({ message: '同级目录下已存在同名目录', type: 'error' }); return }
  await store.updateSeries(id, { name: n }); editingSeriesId.value = null; await store.fetchSeries()
}
async function removeSeries(id: string) { const ok = await dangerConfirm({ title: '删除目录', message: '确定删除此产品目录？', confirmText: '删除' }); if (!ok) return; await store.deleteSeries(id); if (selectedSeriesId.value === id) selectSeries(null) }

function buildTree(allSeries: any[], collapsed: Set<string>, q: string): any[] {
  const roots = allSeries.filter((s: any) => !s.parentId)
  const ch = allSeries.filter((s: any) => s.parentId)
  const matchSet = new Set<string>()
  if (q) { function markUp(id: string|null) { while (id) { matchSet.add(id); const p = allSeries.find((x: any) => x.id === id); id = p?.parentId || null } }; for (const s of allSeries.filter((x: any) => x.name.toLowerCase().includes(q))) markUp(s.id) }
  function hasChildren(pid: string) { return ch.some((c: any) => c.parentId === pid && (!q || matchSet.has(c.id))) }
  function buildChildren(parentId: string, depth: number): any[] { if (collapsed instanceof Set ? collapsed.has(parentId) : !!collapsed[parentId]) return []; const direct = ch.filter((c: any) => c.parentId === parentId && (!q || matchSet.has(c.id))); if (direct.length === 0) return []; return [...direct].sort((a: any, b: any) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || (a.name || '').localeCompare(b.name || '')).flatMap((c: any) => { const subs = buildChildren(c.id, depth + 1); return [{ ...c, _depth: depth, _hasChildren: subs.length > 0 || hasChildren(c.id) }, ...subs] }) }
  const visibleRoots = roots.filter((r: any) => !q || matchSet.has(r.id))
  const uncat = visibleRoots.find((r: any) => r.name === UNCATEGORIZED_NAME && !r.parentId)
  const otherRoots = visibleRoots.filter((r: any) => r !== uncat).sort((a: any, b: any) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || a.name.localeCompare(b.name))
  return otherRoots.flatMap((r: any) => { const subs = buildChildren(r.id, 1); return [{ ...r, _depth: 0, _hasChildren: subs.length > 0 || hasChildren(r.id) }, ...subs] })
    .concat(uncat ? [{ ...uncat, _depth: 0, _hasChildren: hasChildren(uncat.id) }, ...buildChildren(uncat.id, 1)] : [])
}

const seriesTree = computed(() => buildTree(store.series as any[], collapsedSeries.value, ''))
const modalTree = computed(() => buildTree(store.series as any[], modalCollapsed.value, ''))

const isDragging = ref(false); const dragType = ref<'series'|'product'|null>(null)
const dragSeriesId = ref<string|null>(null); const dragProductId = ref<string|null>(null)
const dragX = ref(0); const dragY = ref(0); const dragStartX = ref(0); const dragStartY = ref(0); const dragStarted = ref(false)
const dragTimer = ref<ReturnType<typeof setTimeout>|null>(null); const dragTargetId = ref<string|null>(null)
const dragInsert = ref<{targetId:string;side:'above'|'below'|'inside'}|null>(null)
function startSeriesDrag(e: MouseEvent, sid: string) { e.preventDefault(); dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY; document.addEventListener('mousemove', onDragMove); dragTimer.value = setTimeout(() => { isDragging.value = true; dragType.value = 'series'; dragSeriesId.value = sid; dragStarted.value = true }, 300); document.addEventListener('mouseup', onDragUpPre) }
function startProductDrag(e: MouseEvent, pid: string) { e.preventDefault(); dragX.value = dragStartX.value = e.clientX; dragY.value = dragStartY.value = e.clientY; document.addEventListener('mousemove', onDragMove); dragTimer.value = setTimeout(() => { isDragging.value = true; dragType.value = 'product'; dragProductId.value = pid; dragStarted.value = true }, 300); document.addEventListener('mouseup', onDragUpPre) }
function onDragUpPre() { document.removeEventListener('mouseup', onDragUpPre); document.removeEventListener('mousemove', onDragMove); if (dragTimer.value) { clearTimeout(dragTimer.value); dragTimer.value = null }; if (isDragging.value) onDragUp() }
function onDragMove(e: MouseEvent) {
  dragX.value = e.clientX; dragY.value = e.clientY
  const el = document.elementFromPoint(e.clientX, e.clientY)
  const t = el?.closest('[data-drop-target]') as HTMLElement|null
  dragTargetId.value = t?.dataset.dropTarget || null
  if (t && dragType.value === 'series') {
    const rect = t.getBoundingClientRect()
    const relY = e.clientY - rect.top
    const third = rect.height / 3
    const side = relY < third ? 'above' : relY > rect.height - third ? 'below' : 'inside'
    dragInsert.value = { targetId: t.dataset.dropTarget || '', side }
  } else {
    dragInsert.value = null
  }
}
async function onDragUp() {
  document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp)
  const type = dragType.value; const tid = dragTargetId.value; const sId = dragSeriesId.value; const pId = dragProductId.value
  const insert = dragInsert.value
  isDragging.value = false; dragType.value = null; dragSeriesId.value = null; dragProductId.value = null; dragTargetId.value = null; dragInsert.value = null; dragStarted.value = false
  if (!type || !tid) return
  if (type === 'series') {
    if (!sId || sId === tid) return
    try {
      if (insert?.side === 'inside' || tid === '__root__') {
        await store.updateSeries(sId, { parentId: tid === '__root__' ? null : tid })
      } else {
        const target = (store.series as any[]).find((s: any) => s.id === tid)
        if (!target) return
        const targetOrder = target.sort_order ?? 0
        await store.updateSeries(sId, { parentId: target.parentId || null, sortOrder: insert?.side === 'above' ? targetOrder - 1 : targetOrder + 1 })
      }
      await store.fetchSeries(); toast({ message: '目录已移动', type: 'success' })
    } catch { toast({ message: '移动失败', type: 'error' }) }
  }
  else if (type === 'product') {
    if (!pId) return
    try {
      const newSeriesId = (tid === store.uncategorizedSeriesId || tid === '__root__') ? null : tid
      const idsToMove = selectedCount.value > 1 && isSelected(pId)
        ? [...selectedIds.value]
        : [pId]
      for (const rid of idsToMove) {
        await store.updateProduct(rid, { seriesId: newSeriesId })
      }
      await store.fetchProducts()
      clearMultiSelect()
      toast({ message: idsToMove.length > 1 ? `已移动 ${idsToMove.length} 个产品` : '产品已移动', type: 'success' })
    } catch { toast({ message: '移动失败', type: 'error' }) }
  }
}
onUnmounted(() => { if (dragTimer.value) clearTimeout(dragTimer.value); document.removeEventListener('mousemove', onDragMove); document.removeEventListener('mouseup', onDragUp); document.removeEventListener('mouseup', onDragUpPre); unlistenDragDrop?.() })

// ─── Products ────────────────────────────────────────────────────────
const selectedProductId = ref<string|null>(null)
const productListRef = ref<HTMLElement|null>(null)
const filteredProducts = computed(() => { let r = store.products as any[]; if (selectedSeriesId.value) { if (selectedSeriesId.value === store.uncategorizedSeriesId) { r = r.filter((p: any) => !p.seriesId) } else { r = r.filter((p: any) => p.seriesId === selectedSeriesId.value) } }; const q = seriesSearch.value.toLowerCase().trim(); if (q) r = r.filter((p: any) => p.name.toLowerCase().includes(q)); return [...r].sort((a: any, b: any) => (b.updatedAt||b.createdAt||'').localeCompare(a.updatedAt||a.createdAt||'')) })
function selectProduct(id: string|null) { selectedProductId.value = id }

const { selectedIds, clear: clearMultiSelect, selectSingle: multiSelectSingle, selectRange: multiSelectRange, isSelected, selectedCount } = useMultiSelect(filteredProducts)
const pendingShiftDown = ref(false)

function onProductMouseDown(e: MouseEvent, pid: string) {
  if ((e.target as HTMLElement).closest('input')) return
  if (e.shiftKey) {
    e.preventDefault()
    multiSelectRange(pid)
    pendingShiftDown.value = true
    return
  }
  pendingShiftDown.value = false
  if (!isSelected(pid)) multiSelectSingle(pid)
  startProductDrag(e, pid)
}

function onProductCardClick(e: MouseEvent, pid: string) {
  if (e.shiftKey || pendingShiftDown.value) return
  clearMultiSelect()
  selectProduct(pid)
}

function isDuplicateName(name: string, excludeId?: string): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return (store.products as any[]).some((p: any) => p.id !== excludeId && (p.name || '').toLowerCase() === q)
}
async function createProduct() {
  try {
    let name = '新产品'
    let i = 1
    while (isDuplicateName(name)) { name = `新产品 ${++i}` }
    const d: any = { name }
    if (selectedSeriesId.value && selectedSeriesId.value !== store.uncategorizedSeriesId) {
      d.seriesId = selectedSeriesId.value
    }
    const newProduct = await store.createProduct(d)
    await store.fetchProducts()
    if (newProduct?.id) {
      selectedProductId.value = newProduct.id
      editingName.value = true; editName.value = ''
      await nextTick()
      // Scroll to the new product card in the list
      const el = productListRef.value?.querySelector(`[data-product-id="${newProduct.id}"]`)
      if (el) el.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
      else if (productListRef.value) productListRef.value.scrollTop = 0
      setTimeout(() => (document as any).getElementById('pname')?.focus(), 50)
    }
  } catch (e: any) { toast({ message: e.message || '创建失败', type: 'error' }) }
}
const currentProduct = computed(() => (store.products as any[]).find((p: any) => p.id === selectedProductId.value) || null)

// ─── Product Excel 导入/导出 ─────────────────────────────────────────────
const exportingProducts = ref(false)
const showProductImportWizard = ref(false)

const productImportFields: ImportField[] = [
  { key: 'name',      label: '产品名称', guesses: ['产品名称', '名称', 'Name', 'name', '型号'], required: true },
  { key: 'fullSpecs', label: '技术参数', guesses: ['技术参数', '参数', 'Specs', 'specs', '规格', '描述'] },
  { key: 'description', label: '描述',   guesses: ['描述', '说明', 'Description', 'description', '备注'] },
]

function classifyProductRow(row: RawDataRow, mapping: (string | undefined)[]): string | null {
  const nameIdx = mapping.findIndex(k => k === 'name')
  const nameVal = nameIdx >= 0 ? (row.cells[nameIdx] || '').trim() : ''
  if (nameVal) return 'import'
  // 无名称但有内容 → 问题行
  const hasContent = row.cells.some(c => c.trim().length > 0)
  if (hasContent) return null
  return 'ignore'
}

// 目录下拉选项（全部系列，按层级缩进，不受折叠状态影响）
const seriesOptions = computed(() => {
  const all = store.series as any[]
  const out: { id: string; label: string }[] = []
  function walk(pid: string | null, depth: number) {
    for (const s of all.filter((x: any) => (x.parentId || null) === pid).sort((a: any, b: any) => (a.sort_order ?? 0) - (b.sort_order ?? 0) || (a.name || '').localeCompare(b.name || ''))) {
      out.push({ id: s.id, label: '　'.repeat(depth) + s.name })
      walk(s.id, depth + 1)
    }
  }
  walk(null, 0)
  return out
})

// 导入分析：全局同名 + 文件内重复 → 跳过
const importTargetSeriesId = ref<string | null>(null)
const importing = ref(false)

async function onProductImportConfirm(payload: any) {
  const { mapping, rows, decisions, sheet } = payload
  importing.value = true
  let created = 0, skipped = 0
  try {
    // 确定目标目录
    importTargetSeriesId.value = (selectedSeriesId.value && selectedSeriesId.value !== store.uncategorizedSeriesId) ? selectedSeriesId.value : null
    const seen = new Set<string>()
    for (let i = 0; i < rows.length; i++) {
      const decision = decisions[i]
      if (!decision || decision.action === 'ignore' || decision.action === 'problem') continue
      const row = rows[i]
      const nameIdx = mapping.findIndex((k: string | undefined) => k === 'name')
      const specsIdx = mapping.findIndex((k: string | undefined) => k === 'fullSpecs')
      const descIdx = mapping.findIndex((k: string | undefined) => k === 'description')
      const name = nameIdx >= 0 ? (row.cells[nameIdx] || '').trim() : ''
      if (!name) { skipped++; continue }
      const fullSpecs = specsIdx >= 0 ? (row.cells[specsIdx] || '').trim() : ''
      const description = descIdx >= 0 ? (row.cells[descIdx] || '').trim() : ''
      // 文件内去重
      const key = name.toLowerCase()
      if (seen.has(key) || isDuplicateName(name)) { skipped++; continue }
      seen.add(key)
      try {
        await store.createProduct({ name, fullSpecs, description, seriesId: importTargetSeriesId.value })
        created++
      } catch { skipped++ }
    }
    await store.fetchProducts()
    toast({ message: `成功导入 ${created} 个产品${skipped > 0 ? `，跳过 ${skipped} 个` : ''}`, type: 'success' })
    showProductImportWizard.value = false
  } finally { importing.value = false }
}

async function handleExportProducts() {
  if (exportingProducts.value) return
  exportingProducts.value = true
  try {
    const buf = await exportProductsToExcel(store.products as any[])
    const dateStr = new Date().toISOString().slice(0, 10).replace(/-/g, '')
    const r = await exportWriteBuffer({ buffer: Array.from(new Uint8Array(buf)), fileName: `产品导出_${dateStr}.xlsx` })
    if (r.success) toast({ message: `已导出 ${store.products.length} 个产品`, type: 'success' })
    else if (r.error !== 'cancelled') toast({ message: r.error || '导出失败', type: 'error' })
  } catch (e: any) { toast({ message: '导出失败: ' + (e?.message || ''), type: 'error' }) }
  finally { exportingProducts.value = false }
}

const editingName = ref(false); const editName = ref('')
function startEditName() { editingName.value = true; editName.value = currentProduct.value?.name || ''; setTimeout(() => (document as any).getElementById('pname')?.focus(), 50) }
async function saveName() {
  const v = editName.value.trim()
  if (!selectedProductId.value || !v) { editingName.value = false; return }
  if (isDuplicateName(v, selectedProductId.value)) { toast({ message: '已存在同名产品，请修改名称', type: 'error' }); return }
  await store.updateProduct(selectedProductId.value, { name: v }); editingName.value = false; await store.fetchProducts()
}
async function deleteProduct(id: string) { const ok = await dangerConfirm({ title: '删除产品', message: '确定删除此产品？', confirmText: '删除' }); if (!ok) return; await store.deleteProduct(id); if (selectedProductId.value === id) selectedProductId.value = null; await store.fetchProducts() }

// ─── Local edit state (v-model) with debounced save ────────────────
const localDesc = ref(''); const localFullSpecs = ref('')
watch(() => selectedProductId.value, pid => {
  // 取消所有 pending saves
  for (const t of Object.values(saveTimers)) clearTimeout(t as number)
  saveTimers = {}
  if (!pid) return
  const p = currentProduct.value
  // product_list 返回蛇形字段，兼容两种命名
  localDesc.value = p?.description || ''
  localFullSpecs.value = p?.fullSpecs || p?.full_specs || ''
})

// 每个字段独立的 debounce timer
let saveTimers: Record<string, ReturnType<typeof setTimeout>> = {}

// @input 驱动的自动保存：只在用户输入时触发，程序赋值（加载产品数据）不会触发
function onDescInput() {
  if (!selectedProductId.value) return
  if (saveTimers.desc) clearTimeout(saveTimers.desc)
  saveTimers.desc = setTimeout(() => { delete saveTimers.desc; store.updateProduct(selectedProductId.value!, { description: localDesc.value }) }, 600)
}
function onFullSpecsInput() {
  if (!selectedProductId.value) return
  if (saveTimers.fullSpecs) clearTimeout(saveTimers.fullSpecs)
  saveTimers.fullSpecs = setTimeout(() => { delete saveTimers.fullSpecs; store.updateProduct(selectedProductId.value!, { fullSpecs: localFullSpecs.value }) }, 600)
}

// ─── Tags ─────────────────────────────────────────────────────────────

// ═══════════════════════════════════════════════════════════════════
// Materials ─ upload + list
// ═══════════════════════════════════════════════════════════════════
const materials = ref<any[]>([])
const materialUploadOpen = ref(false)
const materialPasteArea = ref<HTMLTextAreaElement | null>(null)
// 待上传物料：name 不含扩展名，ext 单独存（编辑文件名不涉及后缀）
const materialFiles = ref<Array<{ path: string; name: string; ext: string; _temp?: boolean }>>([])
const materialDragOver = ref(false)
const materialType = ref('彩页')
const materialNote = ref('')

/** 拆分为 名称 + 扩展名（名称不含后缀，ext 小写） */
function splitNameExt(fileName: string): { name: string; ext: string } {
  const i = fileName.lastIndexOf('.')
  if (i > 0) return { name: fileName.slice(0, i), ext: fileName.slice(i + 1).toLowerCase() }
  return { name: fileName, ext: '' }
}
const materialAssoc = ref<{ productIds: string[], seriesIds: string[] }>({ productIds: [], seriesIds: [] })
const materialSearch = ref('')
const materialProjectsExpanded = ref(false)
const materialDownloading = ref<string|null>(null)
const assocMaterialId = ref<string|null>(null)
const editingMaterialId = ref<string|null>(null)
const editingMaterialName = ref('')
const assocProductIds = ref<string[]>([])

// ─── 窗口级 Ctrl/Cmd+V 兜底：不依赖焦点，切回应用后仍可粘贴文件 ───
function isEditableTarget(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  if (!t) return false
  const tag = t.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t.isContentEditable
}
let lastMatClipRead = 0
let matKeydownHandler: ((e: KeyboardEvent) => void) | null = null
watch(materialUploadOpen, (open) => {
  if (open) {
    matKeydownHandler = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey
      if (!mod || (e.key.toLowerCase() !== 'v')) return
      if (isEditableTarget(e.target)) return
      e.preventDefault()
      const now = Date.now()
      if (now - lastMatClipRead < 500) return
      lastMatClipRead = now
      readMaterialClipboard()
    }
    window.addEventListener('keydown', matKeydownHandler)
  } else {
    if (matKeydownHandler) { window.removeEventListener('keydown', matKeydownHandler); matKeydownHandler = null }
  }
})

function onMaterialDragOver(e: DragEvent) { e.preventDefault(); materialDragOver.value = true }
function onMaterialDragLeave() { materialDragOver.value = false }
function onMaterialDrop(e: DragEvent) {
  e.preventDefault()
  // Don't clear materialDragOver here — the Tauri native onDragDropEvent listener
  // needs it to be true to capture the file path. It will be cleared by the listener.
}

/** 粘贴上传：剪贴板文件/截图 → 保存临时文件 → 加入待上传列表 */
async function onMaterialPaste(e: ClipboardEvent) {
  let added = 0
  const files = e.clipboardData?.files
  if (files && files.length > 0) {
    e.preventDefault()
    for (const file of Array.from(files)) {
      const rawName = file.name || 'pasted'
      const ext = (rawName.split('.').pop() || file.type.split('/')[1] || 'bin').toLowerCase()
      try {
        const buf = await file.arrayBuffer()
        const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: file.name || null })
        if (r?.success && r.path) {
          const displayName = r.name || rawName
          const { name, ext: e } = splitNameExt(displayName)
          materialFiles.value.push({ path: r.path, name: name || 'pasted', ext: e || ext, _temp: true })
          added++
        }
      } catch (err) { console.warn('[ProductCenter] paste save failed:', err) }
    }
  }
  // 回退：从 clipboardData.items 获取图片（截图等无文件名）
  if (added === 0) {
    const items = e.clipboardData?.items
    if (items) {
      for (const item of items) {
        if (!item.type.startsWith('image/')) continue
        e.preventDefault()
        const file = item.getAsFile()
        if (!file) continue
        try {
          const buf = await file.arrayBuffer()
          const ext = item.type.split('/')[1] === 'jpeg' ? 'jpg' : (item.type.split('/')[1] || 'png')
          const r: any = await invoke('chat_save_temp_image', { bytes: new Uint8Array(buf), ext, origName: null })
          if (r?.success && r.path) {
            const displayName = r.name || 'pasted'
            const { name, ext: e } = splitNameExt(displayName)
            materialFiles.value.push({ path: r.path, name: name || 'pasted', ext: e || ext, _temp: true })
            added++
          }
        } catch (err) { console.warn('[ProductCenter] paste image failed:', err) }
      }
    }
  }
  // 系统剪贴板中的文件不暴露给 clipboardData（浏览器安全限制），
  // 若浏览器侧未取到任何文件，回退从后端读取剪贴板文件列表
  if (added === 0) {
    await readMaterialClipboard()
  }
  if (added > 0) toast({ message: `已粘贴 ${added} 个文件`, type: 'success' })
  // 处理完成后重新聚焦隐藏 textarea，保持后续粘贴可用
  requestAnimationFrame(() => {
    materialPasteArea.value?.focus()
  })
}

/** 从后端读取系统剪贴板（资源管理器/Finder 复制）的文件，加入待上传列表 */
async function readMaterialClipboard() {
  try {
    const r: any = await invoke('chat_read_clipboard_files')
    if (r?.success && Array.isArray(r.files) && r.files.length) {
      for (const f of r.files) {
        const displayName = f.name || 'pasted'
        const { name, ext } = splitNameExt(displayName)
        // 去重：keydown 与 paste 可能双触发
        if (materialFiles.value.some(m => m.path === f.path)) continue
        materialFiles.value.push({ path: f.path, name: name || 'pasted', ext, _temp: true })
      }
    }
  } catch (err) { console.warn('[ProductCenter] macos clipboard read failed:', err) }
}

const materialTypeOptions = ['彩页', '白皮书', '检测报告', '其他']
const materialTypeLabels: Record<string, string> = { 'BROCHURE': '彩页', 'WHITEPAPER': '白皮书', 'TEST_REPORT': '检测报告', 'OTHER': '其他' }
const materialTypeColors: Record<string, string> = { 'BROCHURE': 'bg-blue-50 text-blue-600', 'WHITEPAPER': 'bg-purple-50 text-purple-600', 'TEST_REPORT': 'bg-amber-50 text-amber-600', 'OTHER': 'bg-slate-100 text-slate-500' }
const materialTypeLabelsExt: Record<string, string> = {
  'BROCHURE': '彩页', 'WHITEPAPER': '白皮书', 'TEST_REPORT': '检测报告', 'OTHER': '其他',
  '彩页': '彩页', '白皮书': '白皮书', '检测报告': '检测报告', '其他': '其他',
}
const materialTypeColorsExt: Record<string, string> = {
  'BROCHURE': 'bg-blue-50 text-blue-600', 'WHITEPAPER': 'bg-purple-50 text-purple-600', 'TEST_REPORT': 'bg-amber-50 text-amber-600', 'OTHER': 'bg-slate-100 text-slate-500',
  '彩页': 'bg-blue-50 text-blue-600', '白皮书': 'bg-purple-50 text-purple-600', '检测报告': 'bg-amber-50 text-amber-600', '其他': 'bg-slate-100 text-slate-500',
}
function materialTypeLabel(m: any) { return materialTypeLabelsExt[m.mapping_type] || materialTypeLabelsExt[m.description] || m.mapping_type || m.description || '其他' }
function materialTypeColor(m: any) { return materialTypeColorsExt[m.mapping_type] || materialTypeColorsExt[m.description] || materialTypeColorsExt['OTHER'] }

async function loadMaterials() { if (!selectedProductId.value) return; try { const r: any = await listMaterials(selectedProductId.value); materials.value = r?.materials || r || [] } catch { materials.value = [] } }

async function openMaterialUpload() {
  assocMaterialId.value = null // clear associate mode
  materialFiles.value = []; materialType.value = '彩页'; materialNote.value = ''
  materialAssoc.value = { productIds: [selectedProductId.value!], seriesIds: [] }
  modalCollapsed.value = Object.fromEntries((store.series as any[]).map((s: any) => [s.id, true]))
  materialUploadOpen.value = true
  // macOS：paste 事件只在可编辑元素上触发，弹窗打开后聚焦隐藏的 textarea
  requestAnimationFrame(() => {
    materialPasteArea.value?.focus()
  })
}
async function handleFilePick() {
  try {
    const result = await openDialog({ multiple: true, canCreateDirectories: false })
    if (!result) return
    const paths: string[] = Array.isArray(result) ? result : [result as string]
    for (const p of paths) {
      const path = typeof p === 'string' ? p : (p as any).path || p
      const fullName = typeof p === 'string' ? path.replace(/^.*[\\\/]/, '') : ((p as any).name || path.replace(/^.*[\\\/]/, ''))
      if (materialFiles.value.some(f => f.path === path)) continue
      const { name, ext } = splitNameExt(fullName)
      materialFiles.value.push({ path, name: name || '未命名', ext })
    }
  } catch (e) {
    console.error('handleFilePick error:', e)
    toast({ message: '打开文件对话框失败', type: 'error' })
  }
}
function toggleAssocProduct(pid: string) {
  const idx = materialAssoc.value.productIds.indexOf(pid)
  if (idx === -1) materialAssoc.value.productIds.push(pid)
  else materialAssoc.value.productIds.splice(idx, 1)
}
function toggleAssocSeries(sid: string) {
  const idx = materialAssoc.value.seriesIds.indexOf(sid)
  if (idx === -1) materialAssoc.value.seriesIds.push(sid)
  else materialAssoc.value.seriesIds.splice(idx, 1)
  // Auto select products in this series
  const pids = (store.products as any[]).filter((p: any) => p.seriesId === sid).map((p: any) => p.id)
  for (const pid of pids) {
    if (idx === -1 && !materialAssoc.value.productIds.includes(pid)) materialAssoc.value.productIds.push(pid)
    else if (idx !== -1) materialAssoc.value.productIds = materialAssoc.value.productIds.filter((x: any) => x !== pid)
  }
}
const { start: upStart, done: upDone, fail: upFail } = useUploadProgress()

async function confirmUpload() {
  const productIds = materialAssoc.value.productIds.filter((id: string) => id)
  if (!materialFiles.value.length || productIds.length === 0) { toast({ message: '请选择文件和关联产品', type: 'error' }); return }
  const files = [...materialFiles.value]
  // 关闭弹窗，异步逐个上传
  materialUploadOpen.value = false
  let ok = 0
  let fail = 0
  for (const f of files) {
    const path = f.path
    const baseName = f.name || path.replace(/^.*[\\\/]/, '').replace(/\.[^.]+$/, '')
    const ext = f.ext || ''
    const name = ext ? `${baseName}.${ext}` : baseName
    const task = upStart(name)
    try {
      await uploadFile({ filePath: path, fileName: name, fileExt: ext, entityType: 'product', entityIds: productIds, customName: name, note: materialType.value, creator: '' })
      ok++
      upDone(task)
    } catch (e: any) {
      fail++
      upFail(task, e.message)
    } finally {
      // 粘贴产生的临时文件，上传后清理
      if (f._temp && path) { try { await removeTempFile(path) } catch {} }
    }
  }
  await loadMaterials()
  if (ok > 0 && fail === 0) toast({ message: `物料上传成功 ${ok} 个`, type: 'success' })
  else if (ok > 0) toast({ message: `物料上传完成：成功 ${ok}，失败 ${fail}`, type: 'warning' })
  else toast({ message: '物料上传失败', type: 'error' })
}

/** 移除待上传物料项（粘贴产生的同步删除临时文件） */
function removeMaterialFile(idx: number) {
  const f = materialFiles.value[idx]
  if (f && f._temp && f.path) {
    try { removeTempFile(f.path).catch(() => {}) } catch {}
  }
  materialFiles.value.splice(idx, 1)
}
function clearMaterialFiles() {
  for (const f of materialFiles.value) {
    if (f._temp && f.path) {
      try { removeTempFile(f.path).catch(() => {}) } catch {}
    }
  }
  materialFiles.value = []
}
async function handleDeleteMaterial(mid: string) {
  const ok = await dangerConfirm({ title: '删除物料', message: '确定删除此物料？', confirmText: '删除' })
  if (!ok) return
  try { await deleteMaterial(mid); await loadMaterials(); toast({ message: '物料已删除', type: 'success' }) } catch (e: any) { toast({ message: '删除失败: ' + (e.message || ''), type: 'error' }) }
}
function startRenameMaterial(m: any) {
  editingMaterialId.value = m.id
  editingMaterialName.value = m.global_name || m.custom_name || ''
  import('vue').then(({ nextTick }) => nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-mat-rename-input]')
    el?.focus(); el?.select()
  }))
}
async function confirmRenameMaterial(m: any) {
  const n = editingMaterialName.value.trim()
  if (n) { try { await updateFileMeta(m.id, { customName: n, relationId: m.relation_id }); await loadMaterials() } catch (e) { console.error('[Product] renameMaterial failed:', e) } }
  editingMaterialId.value = null
}
function cancelRenameMaterial() { editingMaterialId.value = null }
async function handleDownloadMaterial(mid: string) { materialDownloading.value = mid; try { const r = await exportFile(mid); if (!r?.success) { toast({ message: '下载失败: ' + (r?.error || ''), type: 'error' }) } } catch (e: any) { toast({ message: '下载失败: ' + (typeof e === 'string' ? e : (e.message || '未知错误')), type: 'error' }) }; materialDownloading.value = null }
async function handlePreviewMaterial(mid: string) {
  try { const r = await previewFile(mid); if (!r?.success) { toast({ message: '预览失败: ' + (r?.error || ''), type: 'error' }) } } catch (e: any) { toast({ message: '预览失败: ' + (typeof e === 'string' ? e : (e.message || '未知错误')), type: 'error' }) }
}
async function openAssocMaterial(m: any) {
  assocMaterialId.value = m.id
  materialFiles.value = []
  // 加载已有的产品关联
  let ids: string[] = []
  try {
    const r = await getMaterialAssociations(m.id)
    // file_get_product_associations 返回 { success, products: [{id,...}] }
    if (r?.products && Array.isArray(r.products)) {
      ids = r.products.map((p: any) => p.entity_id || p.id).filter(Boolean)
    }
  } catch (e) { console.error('getMaterialAssociations failed:', e); ids = [] }
  assocProductIds.value = ids
  materialAssoc.value = { productIds: [...ids], seriesIds: [] }
  modalCollapsed.value = Object.fromEntries((store.series as any[]).map((s: any) => [s.id, true]))
  materialUploadOpen.value = true
}
async function saveAssociations() {
  if (!assocMaterialId.value) return
  try {
    await updateMaterialAssociations(assocMaterialId.value, materialAssoc.value.productIds)
    materialUploadOpen.value = false
    await loadMaterials()
    toast({ message: '关联已更新', type: 'success' })
  } catch (e: any) {
    toast({ message: '更新失败: ' + (e.message || ''), type: 'error' })
  }
}

const assocProductSearch = computed(() => {
  const q = materialSearch.value.toLowerCase().trim()
  return (store.products as any[]).filter((p: any) => !q || p.name.toLowerCase().includes(q))
})

let unlistenDragDrop: (() => void) | null = null

onMounted(async () => {
  await store.fetchSeries(); collapsedSeries.value = new Set(store.series.map(s => s.id)); await store.fetchProducts()
  selectedSeriesId.value = store.selectedSeriesId
  // Use Tauri's native file drop event to get actual file paths
  try {
    const webview = getCurrentWebview()
    unlistenDragDrop = await webview.onDragDropEvent((event) => {
      if (event.payload.type === 'drop' && materialDragOver.value) {
        const paths = event.payload.paths
        if (paths && paths.length > 0) {
          for (const filePath of paths) {
            const fullName = filePath.replace(/^.*[\\\/]/, '')
            if (materialFiles.value.some(f => f.path === filePath)) continue
            const { name, ext } = splitNameExt(fullName)
            materialFiles.value.push({ path: filePath, name: name || '未命名', ext })
          }
        }
        materialDragOver.value = false
      } else if (event.payload.type === 'leave') {
        materialDragOver.value = false
      }
    })
  } catch (e) { console.warn('[ProductCenter] Failed to listen drag-drop:', e) }
  handleRouteQuery()
})
onActivated(() => { store.fetchProducts(); handleRouteQuery() })

function handleRouteQuery() {
  const action = route.query.action as string
  const focus = route.query.focus as string
  if (action === 'create') {
    const name = route.query.name as string || ''
    // ★ 先消费后清 query：首挂时 onMounted+onActivated 双触发，重复执行会建两个产品
    router.replace({ path: route.path, query: { ...(focus ? { focus } : {}) } })
    createProductPrefilled(name)
  }
  if (focus) {
    selectedProductId.value = focus
  }
}
// ★ 同路由内 query 变化不触发 onActivated（KeepAlive 已激活），通知快捷操作跳转时在此消费
watch(() => route.query, () => {
  if (route.path === '/products') handleRouteQuery()
})

async function createProductPrefilled(name: string) {
  try {
    const productName = name || '新产品'
    const d: any = { name: productName }
    if (selectedSeriesId.value && selectedSeriesId.value !== store.uncategorizedSeriesId) {
      d.seriesId = selectedSeriesId.value
    }
    const newProduct = await store.createProduct(d)
    await store.fetchProducts()
    if (newProduct?.id) {
      selectedProductId.value = newProduct.id
    }
  } catch (e: any) { toast({ message: e.message || '创建失败', type: 'error' }) }
}
// ★ 响应全局搜索跳转
let processingSeq = 0
watch(() => selectionStore.current.seq, async () => {
  if (selectionStore.current.type !== 'product') return
  const id = selectionStore.current.id!
  const mySeq = ++processingSeq
  if (store.products.length === 0) await store.fetchProducts()
  if (mySeq !== processingSeq) return  // 已被新 jump 取代
  const product = (store.products as any[]).find((p: any) => p.id === id)
  if (!product) { toast({ message: '产品不存在或已删除', type: 'warning' }); selectionStore.clear(); return }
  selectedSeriesId.value = product.seriesId || null
  selectedProductId.value = id
  selectionStore.clear()
}, { immediate: true })
watch(() => selectedProductId.value, pid => { if (pid) loadMaterials() })
watch(() => selectedSeriesId.value, () => store.fetchProducts())
</script>

<template>
<div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm">
  <!-- Sidebar: Series tree (unchanged structure) -->
  <aside class="flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden relative" :style="{width:sidebarWidth+'px'}">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>
    <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
      <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">目录</span>
      <button @click="startAddSeries()" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"><AppIcon name="plus" size="12" stroke="2.5"/></button>
    </div>
    <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
      <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
        <AppIcon name="search" size="12" class="text-surface-400"/><input v-model="seriesSearch" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索产品..."/>
      </div>
    </div>
    <div class="flex-1" style="overflow-y:auto;overflow-x:clip">
      <div class="px-2 py-1">
        <div><button @click="selectSeries(null)" :data-drop-target="'__root__'" class="flex items-center gap-2 w-full py-1.5 px-2 text-xs text-left transition-all duration-150 rounded-lg relative overflow-hidden" style="padding-left:12px" :class="[selectedSeriesId===null?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragTargetId==='__root__' ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']"><div v-if="selectedSeriesId===null" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div><AppIcon name="grid" size="13" class="flex-shrink-0"/>全部产品</button></div>
        <template v-for="(s) in seriesTree" :key="s.id">
          <!-- 未分类上方分隔线 -->
          <div v-if="s.name === UNCATEGORIZED_NAME" class="mb-1 mt-2 pt-1.5 border-t border-surface-100"></div>
          <!-- Insertion line: above -->
          <div v-if="isDragging && dragType==='series' && dragInsert?.targetId===s.id && dragInsert?.side==='above'" class="insertion-line" :style="{ marginLeft: `${4 + (s._depth||0) * 16}px` }"></div>
          <div v-if="editingSeriesId===s.id" class="flex items-center gap-1.5 py-1 min-w-0 pr-2" :style="{ paddingLeft: `${12 + (s._depth||0) * 16}px` }"><span v-if="s.name !== UNCATEGORIZED_NAME && s._hasChildren" class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedSeries.has(s.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span><span v-else-if="s.name !== UNCATEGORIZED_NAME" class="w-3 h-3 flex-shrink-0"></span><span v-else class="w-3 h-3 flex-shrink-0"></span><AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/><input data-series-edit v-model="seriesEditName" class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text" @keyup.enter="saveSeriesName(s.id)" @blur="saveSeriesName(s.id)"/></div>
          <div v-else :data-drop-target="s.id" class="flex items-center gap-1.5 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg group relative overflow-hidden select-none" :class="[isDragging ? 'cursor-grabbing' : 'cursor-pointer', selectedSeriesId===s.id?'bg-blue-50/70 text-brand-700 font-medium':'text-surface-600 hover:bg-surface-100', isDragging && dragSeriesId===s.id?'opacity-30':'', isDragging && dragTargetId===s.id && dragSeriesId!==s.id ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700' : '']" @click="selectSeries(s.id)" @mousedown="startSeriesDrag($event, s.id)" :style="{ paddingLeft: `${12 + (s._depth||0) * 16}px` }">
            <div v-if="dragType==='series' && dragInsert?.targetId===s.id && dragInsert?.side==='inside'" class="absolute inset-0 border-2 border-brand-400 rounded-lg pointer-events-none"></div>
            <div v-if="selectedSeriesId===s.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
            <span v-if="s.name !== UNCATEGORIZED_NAME && s._hasChildren" @click.stop="toggleSeries(s.id)" class="w-3 h-3 flex items-center justify-center rounded text-surface-400"><svg class="w-2.5 h-2.5 transition-transform duration-150" :class="!collapsedSeries.has(s.id)?'rotate-90':''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg></span>
            <span v-else-if="s.name !== UNCATEGORIZED_NAME" class="w-3 h-3 flex-shrink-0"></span>
            <span v-else class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400"/><span class="truncate flex-1">{{ s.name }}</span>
            <template v-if="s.name !== UNCATEGORIZED_NAME"><span @mousedown.stop @click.stop="startAddSeries(s.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="plus" size="10"/></span><span @mousedown.stop @click.stop="startEditSeries(s.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-brand-600 opacity-0 group-hover:opacity-100"><AppIcon name="edit" size="10"/></span><span @mousedown.stop @click.stop="removeSeries(s.id)" class="w-4 h-4 flex items-center justify-center rounded hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="10"/></span></template>
          </div>
          <!-- Insertion line: below -->
          <div v-if="isDragging && dragType==='series' && dragInsert?.targetId===s.id && dragInsert?.side==='below'" class="insertion-line" :style="{ marginLeft: `${4 + (s._depth||0) * 16}px` }"></div>
        </template>
      </div>
    </div>
  </aside>

  <div class="w-px cursor-col-resize flex-shrink-0 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>

  <!-- Product list -->
  <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{width:listWidth+'px'}">
    <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('list', $event)"></div>
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0"><span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">产品</span><div class="flex items-center gap-1"><button @click="handleExportProducts" :disabled="exportingProducts" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="导出产品到 Excel"><AppIcon name="download" size="12" stroke="2.5"/></button><button @click="showProductImportWizard = true" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="从 Excel 导入产品"><AppIcon name="upload" size="12" stroke="2.5"/></button><button @click="createProduct" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"><AppIcon name="plus" size="12" stroke="2.5"/></button></div></div>
    <div ref="productListRef" class="flex-1 overflow-y-auto p-2 space-y-1">
      <div v-for="p in filteredProducts" :key="p.id" :data-product-id="p.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden select-none" :class="[selectedProductId===p.id || isSelected(p.id)?'bg-blue-50/70 border-brand-200 shadow-md scale-[1.02] z-10':'border-surface-100 hover:border-surface-300 hover:shadow-md', isDragging && dragProductId===p.id?'opacity-30':'']" @click="onProductCardClick($event, p.id)" @mousedown="onProductMouseDown($event, p.id)">
        <div v-if="selectedProductId===p.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>
        <div class="flex-1 min-w-0"><p class="text-[12px] font-medium text-surface-800 truncate">{{ p.name||'未命名产品' }}</p><p class="text-[9px] text-surface-400 mt-0.5">{{ (p.updated_at||p.updatedAt||p.created_at||p.createdAt||'').slice(0,16) }}</p></div>
        <button @click.stop="deleteProduct(p.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 flex-shrink-0"><AppIcon name="trash" size="13"/></button>
      </div>
    </div>
  </div>

  <!-- ★ Product Detail — redesigned with materials ──────────────────── -->
  <div class="flex-1 flex flex-col min-w-0 overflow-hidden">
    <template v-if="selectedProductId">
      <!-- Title bar (h-10 matches product list header) -->
      <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">产品详情</span>
      </div>

      <!-- Scrollable content -->
      <div class="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        <!-- Product model (editable via double-click) -->
        <div>
          <div class="flex items-center gap-2" @dblclick="startEditName">
            <template v-if="editingName"><input id="pname" v-model="editName" class="flex-1 text-xs font-semibold text-surface-600 bg-transparent outline-none border-b border-brand-400 pb-0.5 select-text" @keyup.enter="saveName" @blur="saveName"/></template>
            <template v-else><h2 class="text-lg font-semibold text-surface-600">{{ currentProduct?.name||'未命名' }}</h2><button @mousedown.stop @click="startEditName" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600"><AppIcon name="edit" size="11"/></button></template>
          </div>
          <hr class="border-surface-200 mt-1.5"/>
        </div>

        <!-- Description -->
        <div>
          <p class="text-[10px] text-surface-400 uppercase tracking-wider mb-1">产品描述</p>
          <textarea v-model="localDesc" rows="2" @input="onDescInput" class="w-full border border-surface-200 rounded-lg px-3 py-2 text-xs text-surface-600 outline-none focus:border-brand-400 bg-white resize-none placeholder-surface-300" placeholder="产品简介..."/>
        </div>

        <!-- Tech specs -->
        <div>
          <p class="text-[10px] text-surface-400 uppercase tracking-wider mb-1">技术参数</p>
          <textarea v-model="localFullSpecs" rows="16" @input="onFullSpecsInput" class="w-full border border-surface-200 rounded-lg px-3 py-2 text-xs text-surface-600 outline-none focus:border-brand-400 bg-white resize-none placeholder-surface-300" placeholder="输入关键技术参数..."/>
        </div>

          <!-- ★ Materials section -->
          <div>
            <div class="flex items-center justify-between mb-2">
              <p class="text-[10px] text-surface-400 uppercase tracking-wider">物料清单 ({{ materials.length }})</p>
              <button @click="openMaterialUpload" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"><AppIcon name="plus" size="10" stroke="2"/>添加物料</button>
            </div>
            <div v-if="materials.length > 0" class="space-y-1.5">
              <div v-for="m in materials" :key="m.id"
                class="flex items-center gap-2 px-3 py-2.5 bg-surface-50 rounded-lg group hover:bg-surface-100 transition-colors"
                :class="editingMaterialId === m.id ? 'cursor-default' : ''"
              >
                <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-medium flex-shrink-0" :class="materialTypeColor(m)">{{ materialTypeLabel(m) }}</span>
                <div class="flex-1 min-w-0">
                  <template v-if="editingMaterialId === m.id">
                    <input
                      v-model="editingMaterialName"
                      data-mat-rename-input
                      class="w-full text-[12px] font-medium text-surface-800 bg-transparent outline-none border-b border-brand-400 pb-0.5 select-text"
                      @keyup.enter="confirmRenameMaterial(m)"
                      @blur="confirmRenameMaterial(m)"
                      @click.stop
                      @mousedown.stop
                      @keyup.escape="cancelRenameMaterial()"
                    />
                  </template>
                  <template v-else>
                    <p class="text-[12px] font-medium text-surface-800 truncate cursor-pointer hover:text-blue-600 transition-colors" @click="handlePreviewMaterial(m.id)">{{ m.global_name || m.custom_name || '未命名' }}</p>
                  </template>
                  <p class="text-[9px] text-surface-400 mt-0.5">{{ formatDateTime(m.created_at) }}</p>
                </div>
                <button @mousedown.stop @click.stop="startRenameMaterial(m)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100" title="重命名"><AppIcon name="edit" size="13"/></button>
                <button @click.stop="handleDownloadMaterial(m.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100" title="下载"><AppIcon name="download" size="13"/></button>
                <button @click.stop="openAssocMaterial(m)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 opacity-0 group-hover:opacity-100" title="关联产品"><AppIcon name="link" size="13"/></button>
                <button @click.stop="handleDeleteMaterial(m.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100"><AppIcon name="trash" size="13"/></button>
              </div>
            </div>
            <p v-else class="text-xs text-surface-300 py-2 text-center">暂无物料，点击右上角添加</p>
          </div>
      </div>

      <div class="border-t border-surface-100 shrink-0"></div>
    </template>
    <div v-else class="flex-1"></div>
  </div>

  <!-- Drag ghost -->
  <Teleport to="body"><div v-if="isDragging" class="fixed px-3 py-1.5 bg-brand-600 text-white text-[11px] rounded-lg shadow-xl pointer-events-none z-[9999] opacity-90 whitespace-nowrap" :style="{left: (dragX + 12) + 'px', top: (dragY + 12) + 'px'}"><template v-if="dragType === 'series'">{{ store.series.find(s => s.id === dragSeriesId)?.name || '目录' }}</template><template v-else>{{ selectedCount > 1 ? `${selectedCount} 个产品` : `${store.products.find(p => p.id === dragProductId)?.name || '产品'} · Shift+点击可多选` }}</template></div></Teleport>

  <!-- Excel Import Wizard -->
  <ExcelImportWizard
    v-model="showProductImportWizard"
    title="导入产品 Excel"
    :fields="productImportFields"
    :rowActions="[
      { key: 'import',   label: '导入', color: 'emerald' },
      { key: 'ignore',   label: '忽略', color: 'slate' },
      { key: 'problem',  label: '需要选择', color: 'amber', isProblem: true },
    ]"
    :classify="classifyProductRow"
    @confirm="onProductImportConfirm"
  />

  <!-- Material upload dialog -->
  <Teleport to="body"><Transition name="modal-fade"><div v-if="materialUploadOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm">
    <!-- 隐藏 textarea：macOS 上 paste 事件只在可编辑元素触发，用它捕获粘贴 -->
    <textarea ref="materialPasteArea" class="fixed top-0 left-0 w-px h-px opacity-0" tabindex="-1" @paste.prevent="onMaterialPaste"></textarea>
    <div class="bg-white rounded-2xl w-[500px] max-h-[85vh] overflow-hidden shadow-2xl flex flex-col border border-surface-200">
      <div class="flex items-center px-5 py-2 border-b border-surface-100"><h3 class="text-sm font-semibold text-surface-800">{{ assocMaterialId ? '设置物料关联' : '上传产品物料' }}</h3></div>

      <div class="flex-1 overflow-y-auto p-5 space-y-4">
        <!-- File pick — only in upload mode -->
        <template v-if="!assocMaterialId">
        <div>
          <div class="relative">
            <div
              @click="handleFilePick"
              @dragover="onMaterialDragOver"
              @dragleave="onMaterialDragLeave"
              @drop.prevent="onMaterialDrop"
              class="w-full border-2 border-dashed rounded-xl py-5 px-4 text-center transition-all duration-200 cursor-pointer flex items-center justify-center gap-2.5"
              :class="materialDragOver ? 'border-brand-400 bg-brand-50/50' : materialFiles.length ? 'border-emerald-300 bg-emerald-50/30' : 'border-surface-200 hover:border-brand-300 hover:bg-surface-50'"
            >
              <template v-if="materialFiles.length">
                <div class="w-8 h-8 rounded-full bg-emerald-100 text-emerald-600 flex items-center justify-center shrink-0">
                  <AppIcon name="file-doc" size="14" />
                </div>
                <div class="text-left">
                  <p class="text-[12px] font-medium text-surface-700">{{ materialFiles.length }} 个文件已选择</p>
                  <p class="text-[12px] text-surface-600 mt-0.5">点击或粘贴追加文件</p>
                </div>
              </template>
              <template v-else>
                <div class="w-8 h-8 rounded-full flex items-center justify-center shrink-0 transition-colors" :class="materialDragOver ? 'bg-brand-100 text-brand-600' : 'bg-surface-100 text-surface-400'">
                  <AppIcon name="upload" size="14" />
                </div>
                <div class="text-left">
                  <p class="text-[12px] font-medium text-surface-700" :class="materialDragOver ? 'text-brand-600' : 'text-surface-700'">
                    {{ materialDragOver ? '释放以上传文件' : `点击选择或按 ${pasteShortcut} 粘贴` }}
                  </p>
                </div>
              </template>
            </div>
          </div>
          <!-- 已选文件列表 -->
          <div v-if="materialFiles.length" class="mt-2 space-y-1.5 max-h-[180px] overflow-y-auto">
            <div class="flex items-center justify-between">
              <span class="text-[11px] font-medium text-surface-500">已选择 {{ materialFiles.length }} 个文件</span>
              <button @click="clearMaterialFiles" class="text-[11px] text-surface-400 hover:text-red-500 transition-colors">清空</button>
            </div>
            <div v-for="(f, idx) in materialFiles" :key="f.path" class="group flex items-center gap-2 px-3 py-1.5 rounded-lg border border-surface-200 bg-white hover:border-surface-300 transition-colors">
              <div class="w-7 h-7 rounded-md bg-surface-50 border border-surface-100 flex items-center justify-center flex-shrink-0">
                <AppIcon name="file-doc" size="13" class="text-surface-500" />
              </div>
              <div class="flex-1 min-w-0 flex items-center gap-1">
                <input
                  v-model="f.name"
                  @focus="$event.target.select()"
                  @blur="() => materialPasteArea?.focus()"
                  class="flex-1 min-w-0 text-[12px] font-medium text-surface-800 bg-transparent border border-transparent focus:border-brand-400 focus:bg-white rounded px-1 py-0.5 outline-none transition-colors"
                  title="编辑文件名"
                />
                <span v-if="f.ext" class="text-[10px] text-surface-400 flex-shrink-0">.{{ f.ext }}</span>
              </div>
              <button @click="removeMaterialFile(idx)" class="w-5 h-5 flex items-center justify-center rounded text-surface-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 opacity-0 group-hover:opacity-100" title="移除"><svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 6l12 12M6 18L18 6"/></svg></button>
            </div>
          </div>
        </div>

        <!-- Material type — only in upload mode -->
        <div><p class="text-[10px] text-surface-400 uppercase tracking-wider mb-2">物料类型</p><div class="flex gap-2"><button v-for="mt in materialTypeOptions" :key="mt" @click="materialType = mt" class="px-3 py-1.5 text-xs rounded-lg transition-colors" :class="materialType===mt?'bg-brand-50 text-brand-700 font-medium border border-brand-200':'border border-surface-200 text-surface-600 hover:bg-surface-50'">{{ mt }}</button></div></div>
        </template>

        <!-- Associated products — collapsible tree -->
        <div><p class="text-[10px] text-surface-400 uppercase tracking-wider mb-2">关联产品</p>
          <div class="border border-surface-200 rounded-lg h-[240px] overflow-y-auto">
            <div class="px-3 py-2 border-b border-surface-100"><div class="flex items-center gap-1.5 bg-surface-50 rounded-lg px-2.5 py-1"><AppIcon name="search" size="12" class="text-surface-400"/><input v-model="materialSearch" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索产品..."/></div></div>
            <div class="p-2">
              <template v-for="s in modalTree" :key="'mdl-'+s.id">
                <div class="flex items-center gap-1.5 w-full px-2 py-1 text-xs cursor-pointer rounded hover:bg-surface-50" @click="toggleModalSeries(s.id)" :style="{ paddingLeft: `${8 + (s._depth||0) * 16}px` }">
                  <span class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0" @click.stop="toggleModalSeries(s.id)">
                    <svg class="w-2.5 h-2.5 transition-transform duration-150" :class="modalCollapsed[s.id] ? '' : 'rotate-90'" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
                  </span>
                  <div class="w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0" :class="materialAssoc.seriesIds.includes(s.id)?'bg-brand-500 border-brand-500 text-white':'border-surface-300'" @click.stop="toggleAssocSeries(s.id)">
                    <svg v-if="materialAssoc.seriesIds.includes(s.id)" class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M5 13l4 4L19 7"/></svg>
                  </div>
                  <AppIcon name="folder" size="13" class="text-surface-400"/><span class="truncate flex-1">{{ s.name }}</span><span class="text-[10px] text-surface-400">全部</span>
                </div>
                <template v-if="!modalCollapsed[s.id]">
                  <div v-for="p in (store.products as any[]).filter((x: any) => (s.name === UNCATEGORIZED_NAME ? !x.seriesId : x.seriesId === s.id) && (!materialSearch || x.name.toLowerCase().includes(materialSearch.toLowerCase())))" :key="'mp-'+p.id" class="flex items-center gap-1.5 w-full py-1 text-[11px] cursor-pointer rounded hover:bg-surface-50" :style="{ paddingLeft: `${36 + (s._depth||0) * 16}px` }" :class="p.id === selectedProductId ? 'text-surface-400 cursor-default' : ''" @click="p.id !== selectedProductId && toggleAssocProduct(p.id)">
                    <div class="w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0" :class="materialAssoc.productIds.includes(p.id) || p.id === selectedProductId ?'bg-brand-500 border-brand-500 text-white':'border-surface-300'">
                      <svg v-if="materialAssoc.productIds.includes(p.id) || p.id === selectedProductId" class="w-2.5 h-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M5 13l4 4L19 7"/></svg>
                    </div>
                    <span class="truncate">{{ p.name||'未命名' }}</span><span v-if="p.id === selectedProductId" class="text-[10px] text-surface-400 ml-1">(当前)</span>
                  </div>
                </template>
              </template>
            </div>
          </div>
        </div>
      </div>

      <div class="flex justify-end gap-2 px-5 py-4 border-t border-surface-100">
        <button @click="materialUploadOpen=false; assocMaterialId=null" class="btn-secondary text-xs">取消</button>
        <button v-if="assocMaterialId" @click="saveAssociations" class="btn-primary text-xs">保存关联</button>
        <button v-else @click="confirmUpload" class="btn-primary text-xs">上传物料</button>
      </div>
    </div>
  </div></Transition></Teleport>
</div>
</template>

<style>
.insertion-line {
  height: 1.5px;
  background: #3b82f6;
  border-radius: 1px;
  opacity: 0.8;
  margin: 1px 0;
}
</style>