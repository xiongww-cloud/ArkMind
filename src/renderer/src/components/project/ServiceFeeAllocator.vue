<script setup lang="ts">
import { ref, computed } from 'vue'
import * as ExcelJSModule from 'exceljs/dist/exceljs.min.js'
import { parseHardwareName, extractModelByWords } from '@/utils/parseHardwareName'
import AppIcon from '@/components/common/AppIcon.vue'

const ExcelJS = ExcelJSModule.default || ExcelJSModule.default?.default || ExcelJSModule

const emit = defineEmits(['confirm', 'cancel'])

// ─── File Parsing State ─────────────────────────────────────────────────────
const fileInput = ref(null)
const fileName = ref('')
const parseError = ref('')
const isParsing = ref(false)
const dragActive = ref(false)

// ─── Parsed Data ─────────────────────────────────────────────────────────────
const rawRows = ref([])          // all parsed rows
const hardwareItems = ref([])    // { uid, rawName, cleanModel, customName, specs, quantity, unit, unitPrice, subtotal }
const serviceItems = ref([])     // { uid, label, amount, details: [{ content, matchedModel, matchedHwUid }] }
// 服务父项金额总和（Excel 解析出的大项合计）
const totalServiceFee = computed(() =>
  serviceItems.value.reduce((s, sv) => s + (Number(sv.totalAmount) || 0), 0)
)

// 已填写的子项金额总和
const allocatedSubAmount = computed(() => {
  let total = 0
  for (const sv of serviceItems.value) {
    for (const sub of (Array.isArray(sv.subItems) ? sv.subItems : [])) {
      total += Number(sub.serviceAmount) || 0
    }
  }
  return total
})

// 待分配金额 = 父项总额 - 已填子项金额
const unallocatedAmount = computed(() =>
  totalServiceFee.value - allocatedSubAmount.value
)
const selectedServiceIndex = ref(0) // currently selected service for detail panel

// ─── Allocation State ────────────────────────────────────────────────────────
const stage = ref('upload')      // 'upload' | 'parsing' | 'allocate' | 'review'
const allocationStrategy = ref('ratio') // 'ratio' | 'equal'

const totalHardwareCost = computed(() =>
  hardwareItems.value.reduce((s, hw) => s + (hw.unitPrice || 0) * (hw.quantity || 0), 0)
)

// 待分配金额 ≤ 0 且有已分配的子项 → 可确认导入
const isAllAllocated = computed(() => {
  return allocatedSubAmount.value > 0 && unallocatedAmount.value <= 0
})

// ─── Header Detection ────────────────────────────────────────────────────────
const HEADER_ALIASES = {
  seq: ['序号', '序', 'seq', 'no', 'No.', '#'],
  name: ['设备名称', '产品名称', '名称', '部件名称', 'name', 'product name', 'device name'],
  model: ['产品型号', '型号', 'model', 'type', '物料编码', '物料', 'item', 'part number', 'part no'],
  specs: ['描述', '配置', '规格', '技术参数', 'description', 'specs', 'specification', '产品描述', '物料描述', '功能描述'],
  quantity: ['数量', '单套数量', 'qty', 'quantity', 'count', '供货数量', '需求数量'],
  unit: ['单位', 'unit', 'uom', '计量单位'],
  unitPrice: ['单价', '单套单价', 'price', 'unit price', '内部单价', '成本单价', '供货单价', '含税单价', '成交单价'],
  subtotal: ['合计', '小计', '总价', 'subtotal', 'total', 'amount', '总金额', '金额'],
}

const ALL_HEADER_KEYWORDS = Object.values(HEADER_ALIASES).flat()

/**
 * Parse a cell value into a clean number, handling currency-formatted strings
 * like "¥1,234.56" or "1,234.56".
 */
function parseAmount(raw) {
  if (raw == null) return 0
  if (typeof raw === 'number') return isNaN(raw) ? 0 : raw
  const cleaned = String(raw).replace(/[¥$,，\s]/g, '').trim()
  const num = Number(cleaned)
  return isNaN(num) ? 0 : num
}

function scoreHeaderRow(headers) {
  // Count how many header keywords match in this row
  let score = 0
  const cleanHeaders = headers.map(h => String(h ?? '').trim().toLowerCase()).filter(Boolean)
  for (const h of cleanHeaders) {
    for (const kw of ALL_HEADER_KEYWORDS) {
      if (h === kw.toLowerCase() || h.includes(kw.toLowerCase())) {
        score++
        break
      }
    }
  }
  return score
}

function findHeaderRow(allRows, maxScanRows) {
  const limit = Math.min(maxScanRows, allRows.length - 1)
  let bestRowIndex = 0
  let bestScore = 0

  for (let i = 0; i < limit; i++) {
    const rowValues = allRows[i].values.map(v => String(v ?? ''))
    const score = scoreHeaderRow(rowValues)
    if (score > bestScore) {
      bestScore = score
      bestRowIndex = i
    }
  }

  // Require at least 2 header keyword matches to trust a non-first row
  if (bestRowIndex > 0 && bestScore < 2) {
    return 0
  }

  return bestRowIndex
}

// ── 序号列检测（华为 BOM 特征：序号列通常是纯数字，1-3 位）───────────
const SEQ_ALIASES = ['序号', '序', 'seq', 'no', 'No.', '#']

/**
 * 判断一行是否包含"序号"值 → 该行是父行级别的分组起始行
 */
function hasSequenceNumber(vals, colMap) {
  // 如果检测到了独立的序号列
  if (colMap.seq !== undefined) {
    const v = vals[colMap.seq]
    return v != null && v !== '' && !isNaN(Number(v))
  }
  // Fallback: 第一列如果是短数字（1-999），可能为序号
  if (vals.length > 0) {
    const firstVal = String(vals[0] ?? '').trim()
    const num = Number(firstVal)
    if (!isNaN(num) && num > 0 && num < 1000 && String(parseInt(firstVal)) === firstVal) {
      // 确认它不是标题行的一部分
      return firstVal.length <= 3
    }
  }
  return false
}


/**
 * 合并单元格金额查找：当前行金额为 0 时，向上扫描同列
 * 直到找到非零值（合并区域的起始行）或到达数据起始行
 */
function getMergedCellAmount(vals, colMap, rowIndex, allRows) {
  // Try current row first
  let amount = (colMap.subtotal !== undefined ? parseAmount(vals[colMap.subtotal]) : 0)
    || (colMap.unitPrice !== undefined ? parseAmount(vals[colMap.unitPrice]) : 0)
  if (amount > 0) return amount

  // 向上扫描（合并单元格值在起始行）
  for (let r = rowIndex - 1; r >= 0; r--) {
    const prevVals = allRows[r]?.values
    if (!prevVals) break
    const prev = (colMap.subtotal !== undefined ? parseAmount(prevVals[colMap.subtotal]) : 0)
      || (colMap.unitPrice !== undefined ? parseAmount(prevVals[colMap.unitPrice]) : 0)
    if (prev > 0) return prev

    // 遇到有序号的行（下一个分组起始）→ 停止，避免跨分组取值
    if (hasSequenceNumber(prevVals, colMap)) break
  }
  return 0
}

function detectHeaders(headers) {
  const mapping = {}
  // First pass: exact match
  for (const [key, aliases] of Object.entries(HEADER_ALIASES)) {
    for (const alias of aliases) {
      const idx = headers.findIndex(h => {
        if (!h) return false
        const clean = String(h).trim().toLowerCase()
        return clean === alias.toLowerCase()
      })
      if (idx >= 0 && mapping[key] === undefined) {
        mapping[key] = idx
        break
      }
    }
  }
  // Second pass: contains match (for headers like "供货单价(元)")
  for (const [key, aliases] of Object.entries(HEADER_ALIASES)) {
    if (mapping[key] !== undefined) continue
    for (const alias of aliases) {
      const idx = headers.findIndex(h => {
        if (!h) return false
        const clean = String(h).trim().toLowerCase()
        return clean.includes(alias.toLowerCase())
      })
      if (idx >= 0) {
        mapping[key] = idx
        break
      }
    }
  }
  return mapping
}

// ─── File Handling ───────────────────────────────────────────────────────────
function triggerFileInput() {
  fileInput.value?.click()
}

async function processFile(file) {
  fileName.value = file.name
  parseError.value = ''
  isParsing.value = true
  stage.value = 'parsing'

  try {
    const isCSV = file.name.toLowerCase().endsWith('.csv')
    let allRows = []

    if (isCSV) {
      // Parse CSV manually
      let text = await file.text()
      // Strip UTF-8 BOM if present (common in Chinese CSV exports)
      if (text.charCodeAt(0) === 0xFEFF) text = text.slice(1)
      const lines = text.split(/\r?\n/).filter(line => line.trim())
      if (lines.length < 2) {
        parseError.value = 'CSV 文件为空或格式不正确'
        isParsing.value = false
        return
      }
      allRows = lines.map((line, idx) => {
        // Simple CSV parsing: handle quoted fields
        const values = []
        let current = ''
        let inQuotes = false
        for (const ch of line) {
          if (ch === '"') {
            inQuotes = !inQuotes
          } else if (ch === ',' && !inQuotes) {
            values.push(current.trim())
            current = ''
          } else {
            current += ch
          }
        }
        values.push(current.trim())
        return { rowNumber: idx + 1, values }
      })
    } else {
      // Parse Excel (xlsx/xls)
      const buffer = await file.arrayBuffer()
      const workbook = new ExcelJS.Workbook()
      await workbook.xlsx.load(buffer)

      const ws = workbook.worksheets[0]
      if (!ws || ws.rowCount < 2) {
        parseError.value = '文件为空或格式不正确'
        isParsing.value = false
        return
      }

      ws.eachRow((row, rowNumber) => {
        const values = []
        row.eachCell({ includeEmpty: true }, (cell) => {
          values.push(cell.value)
        })
        allRows.push({ rowNumber, values })
      })
    }

    if (allRows.length < 2) {
      parseError.value = '文件没有数据行'
      isParsing.value = false
      return
    }

    // Smart header detection — scan first 8 rows for the best header match
    const headerRowIndex = findHeaderRow(allRows, 8)
    const headerRow = allRows[headerRowIndex].values.map(v => String(v ?? ''))
    const colMap = detectHeaders(headerRow)

    if (colMap.model === undefined && colMap.name === undefined) {
      parseError.value = '未找到"型号"或"设备名称"列，无法识别设备列，请检查文件标题行'
      isParsing.value = false
      return
    }
    if (colMap.unitPrice === undefined && colMap.subtotal === undefined) {
      parseError.value = '未找到"单价"或"合计"列，无法计算金额，请检查文件标题行'
      isParsing.value = false
      return
    }

    const hwItems = []
    const serviceList = []
    let uidCounter = 0

    const dataStartIndex = headerRowIndex + 1

    function cellVal(vals, key) {
      if (colMap[key] === undefined) return ''
      const raw = vals[colMap[key]]
      if (raw == null) return ''
      if (typeof raw === 'object') {
        // ExcelJS rich text / formula result
        if (raw.richText && Array.isArray(raw.richText)) {
          return raw.richText.map(t => t.text || '').join('').trim()
        }
        if (raw.result !== undefined) return String(raw.result).trim()
        return ''
      }
      return String(raw).trim()
    }

    // ── Find all seq rows (rows with non-empty 序号) ───────────────────────
    const seqRows = []
    for (let i = dataStartIndex; i < allRows.length; i++) {
      if (hasSequenceNumber(allRows[i].values, colMap)) {
        seqRows.push(i)
      }
    }

    // Helper: cell immediately to the right of the seq column
    function rightOfSeq(vals) {
      // 如果检测到 seq 列，取其右边一列
      if (colMap.seq !== undefined) {
        const rightIdx = colMap.seq + 1
        if (rightIdx < vals.length) return String(vals[rightIdx] ?? '').trim()
      }
      // Fallback: 第二列（第一列是序号）
      if (vals.length > 1) return String(vals[1] ?? '').trim()
      return ''
    }

    const serviceKeywords = ['技术应用支持', '实施']

    // ── Process each seq row ──────────────────────────────────────────────
    for (let si = 0; si < seqRows.length; si++) {
      const rowIdx = seqRows[si]
      const vals = allRows[rowIdx].values
      const rightText = rightOfSeq(vals)

      // 判定：序号行右侧单元格是否包含服务关键字
      const isService = serviceKeywords.some(kw => rightText.includes(kw))

      if (isService) {
        // ── 服务父项 ──────────────────────────────────────────────────────
        const name = cellVal(vals, 'specs') || rightText || ''
        const totalAmount = getMergedCellAmount(vals, colMap, rowIdx, allRows)

        const svc = {
          uid: `svc-${++uidCounter}`,
          name,
          totalAmount,
          subItems: [],
        }

        // 读取下面所有行直到下一个序号行，取其"描述"作为子项
        const nextSeqIdx = (si + 1 < seqRows.length) ? seqRows[si + 1] : allRows.length
        for (let j = rowIdx + 1; j < nextSeqIdx; j++) {
          const subVals = allRows[j].values
          const content = cellVal(subVals, 'specs') || cellVal(subVals, 'name') || cellVal(subVals, 'model') || ''
          if (content) {
            svc.subItems.push({
              content,
              serviceAmount: 0,
              isExpanded: false,
              matchedDevices: [], // { hwUid, cleanModel }
            })
          }
        }

        if (svc.subItems.length > 0) {
          serviceList.push(svc)
        }
      } else {
        // ── 设备行 ────────────────────────────────────────────────────────
        const rawNameFromCol = cellVal(vals, 'name')
        const modelFromCol = cellVal(vals, 'model')
        const sourceStr = rawNameFromCol || modelFromCol
        const specs = cellVal(vals, 'specs')

        // ── 散配包：遍历下面 E 列单元格文本作为独立设备型号 ──────────────
        const isSanPeiBao = (rightText + sourceStr + specs).includes('散配包')
        if (isSanPeiBao) {
          const nextSeqIdx = (si + 1 < seqRows.length) ? seqRows[si + 1] : allRows.length
          // 散配包行单价（列 I）
          const pkgUnitPrice = vals.length > 8 ? parseAmount(vals[8]) : 0
          // 散配包行数量（下一行列 H）
          let pkgQty = 0
          if (rowIdx + 1 < allRows.length) {
            const nextVals = allRows[rowIdx + 1].values
            pkgQty = nextVals.length > 7 ? (Number(nextVals[7]) || 0) : 0
          }
          // 自定义名称：含"散配包"文本最后一个「_」后的部分
          const spbSourceText = [rightText, sourceStr, specs].find(t => t.includes('散配包')) || ''
          const spbCustomName = spbSourceText.includes('_') ? spbSourceText.slice(spbSourceText.lastIndexOf('_') + 1).trim() : ''
          for (let j = rowIdx + 1; j < nextSeqIdx; j++) {
            const subVals = allRows[j].values
            // 固定取 E 列（索引 4）
            const spbText = subVals.length > 4 ? String(subVals[4] ?? '').trim() : ''
            if (!spbText) continue
            hwItems.push({
              uid: `hw-${++uidCounter}`,
              rawName: spbText,
              cleanModel: spbText,
              customName: spbCustomName,
              specs: spbText,
              quantity: pkgQty || 1,
              unit: colMap.unit !== undefined ? cellVal(vals, 'unit') || '台' : '台',
              unitPrice: pkgUnitPrice,
              subtotal: 0,
            })
          }
          continue  // 跳过正常硬件处理，继续下一个序号行
        }

        // 单价固定取本行 I 列（索引 8）
        const unitPrice = vals.length > 8 ? parseAmount(vals[8]) : 0

        // 数量取下一行 H 列（索引 7）
        let quantity = 0
        if (rowIdx + 1 < allRows.length) {
          const nextVals = allRows[rowIdx + 1].values
          quantity = nextVals.length > 7 ? (Number(nextVals[7]) || 0) : 0
        }

        // 型号提取：描述列「前两词」法优先，型号/名称列「先净后分」法兜底
        const wordModel = extractModelByWords(specs)
        const colParsed = sourceStr ? parseHardwareName(sourceStr) : { cleanModel: '', customName: null }
        const modelParsed = modelFromCol ? parseHardwareName(modelFromCol) : { cleanModel: '', customName: null }

        // cleanModel: 前两词 > 型号列结果 > 名称列结果
        const cleanModel = wordModel || modelParsed.cleanModel || colParsed.cleanModel
        // 自定义名称：描述列 _ 右侧 > 名称列 _ 右侧（parseHardwareName 已清洗）
        const specsParsed = specs ? parseHardwareName(specs) : null
        const finalCustomName = specsParsed?.customName || colParsed.customName

        if (sourceStr || cleanModel || rightText) {
          hwItems.push({
            uid: `hw-${++uidCounter}`,
            rawName: sourceStr || rightText || specs,
            cleanModel: cleanModel || rightText,
            customName: finalCustomName,
            specs,
            quantity,
            unit: colMap.unit !== undefined ? cellVal(vals, 'unit') || '台' : '台',
            unitPrice,
            subtotal: 0, // 由后端计算或取合计列
          })
        }

        // 跳过之后所有行直到下一个序号行（已在循环中自动处理）
      }
    }

    hardwareItems.value = hwItems

    // ── 模糊匹配：取子项第一个空格前的文本，与硬件型号做 token 级匹配 ──
    // 「x」为通配符，匹配任意单个 token
    function fuzzyMatchTokens(query, target) {
      const qTokens = query.split(/\s+/).filter(Boolean)
      const tTokens = target.split(/\s+/).filter(Boolean)
      if (qTokens.length === 0 || tTokens.length === 0) return false
      if (qTokens.length > tTokens.length) return false

      // Slide query over target tokens
      for (let start = 0; start <= tTokens.length - qTokens.length; start++) {
        let match = true
        for (let k = 0; k < qTokens.length; k++) {
          if (qTokens[k].toLowerCase() === 'x') continue // wildcard
          if (!tTokens[start + k].toLowerCase().includes(qTokens[k].toLowerCase())) {
            match = false
            break
          }
        }
        if (match) return true
      }
      return false
    }

    for (const sv of serviceList) {
      for (const detail of sv.subItems) {
        // 取第一个空格前的文本作为查询串
        const query = detail.content.split(/\s+/)[0] || ''
        if (!query) continue

        for (const hw of hwItems) {
          if (fuzzyMatchTokens(query, hw.cleanModel || hw.rawName || '')) {
            detail.matchedDevices.push({ hwUid: hw.uid, cleanModel: hw.cleanModel || hw.rawName })
          }
        }
      }
    }

    serviceItems.value = serviceList

    // 无服务父项 → 跳过分摊界面，直接添加设备到方案配置
    if (serviceList.length === 0) {
      isParsing.value = false
      emit('confirm', {
        hardwareItems: hwItems.map(hw => ({
          ...hw,
          allocatedAmount: 0,
          allocPerUnit: 0,
          compositePrice: hw.unitPrice || 0,
        })),
        serviceItems: [],
        totalServiceFee: 0,
        allocationMethod: 'ratio',
      })
      return
    }

    stage.value = 'allocate'
  } catch (err) {
    parseError.value = '解析文件失败：' + (err.message || '未知错误')
    console.error('Excel parse error:', err)
  } finally {
    isParsing.value = false
  }
}

function onFileSelected(e) {
  const file = e.target.files?.[0]
  if (file) processFile(file)
}

function onDragOver(e) { e.preventDefault(); dragActive.value = true }
function onDragLeave() { dragActive.value = false }
function onDragDrop(e) {
  e.preventDefault()
  dragActive.value = false
  const file = e.dataTransfer?.files?.[0]
  if (file) processFile(file)
}

// ─── Allocation Logic (per sub-item → matched devices) ───────────────────────
/**
 * Distribute a sub-item's serviceAmount across its matched devices
 * according to the selected strategy.
 * Returns a Map of hwUid → allocatedAmount.
 */
function computeSubItemAllocation(subItem) {
  const amount = Number(subItem.serviceAmount) || 0
  const devices = (subItem.matchedDevices || []).filter(d => d.hwUid)
  const result = new Map()
  if (amount <= 0 || devices.length === 0) return result

  if (allocationStrategy.value === 'equal') {
    const perDevice = Math.round((amount / devices.length) * 100) / 100
    let remaining = amount
    devices.forEach((d, i) => {
      if (i === devices.length - 1) {
        result.set(d.hwUid, Math.round(remaining * 100) / 100)
      } else {
        result.set(d.hwUid, perDevice)
        remaining -= perDevice
      }
    })
    return result
  }

  // ratio: proportional to (unitPrice × quantity)
  const devicesWithCost = devices.map(d => {
    const hw = hardwareItems.value.find(h => h.uid === d.hwUid)
    const cost = hw ? (hw.unitPrice || 0) * (hw.quantity || 0) : 0
    return { ...d, cost }
  })
  const totalCost = devicesWithCost.reduce((s, d) => s + d.cost, 0)

  if (totalCost <= 0) {
    // fallback to equal
    const perDevice = Math.round((amount / devices.length) * 100) / 100
    let remaining = amount
    devices.forEach((d, i) => {
      if (i === devices.length - 1) {
        result.set(d.hwUid, Math.round(remaining * 100) / 100)
      } else {
        result.set(d.hwUid, perDevice)
        remaining -= perDevice
      }
    })
    return result
  }

  let remaining = amount
  devicesWithCost.forEach((d, i) => {
    if (i === devicesWithCost.length - 1) {
      result.set(d.hwUid, Math.round(remaining * 100) / 100)
    } else {
      const share = amount * (d.cost / totalCost)
      const rounded = Math.round(share * 100) / 100
      result.set(d.hwUid, rounded)
      remaining -= rounded
    }
  })
  return result
}

// ── 实时分摊预览：key="svIdx-subIdx" → Map<hwUid, amount> ──────────────
const previewAllocation = computed(() => {
  const map = {}
  for (let si = 0; si < serviceItems.value.length; si++) {
    const sv = serviceItems.value[si]
    for (let di = 0; di < (Array.isArray(sv.subItems) ? sv.subItems.length : 0); di++) {
      const sub = sv.subItems[di]
      map[`${si}-${di}`] = computeSubItemAllocation(sub)
    }
  }
  return map
})

// ── Detail panel helpers ───────────────────────────────────────────────
const selectedService = computed(() =>
  serviceItems.value[selectedServiceIndex.value] || null
)

function findHwByUid(uid) {
  return hardwareItems.value.find(hw => hw.uid === uid) || null
}

function getAllocPreview(svIdx, subIdx, hwUid) {
  return previewAllocation.value[`${svIdx}-${subIdx}`]?.get(hwUid) || 0
}

function totalMatchedQty(detail) {
  return (detail.matchedDevices || []).reduce((sum, md) => {
    const hw = findHwByUid(md.hwUid)
    return sum + (hw ? hw.quantity : 0)
  }, 0)
}

// ── 添加设备弹窗 ──────────────────────────────────────────────────────
const addDeviceTarget = ref(null) // 当前正在添加设备的子项索引
const showDevicePicker = ref(false)

function openDevicePicker(subItemIdx) {
  addDeviceTarget.value = subItemIdx
  showDevicePicker.value = true
}

function onSelectDeviceForSubItem(hw) {
  if (addDeviceTarget.value === null) return
  const sv = selectedService.value
  if (!sv?.subItems) return
  const detail = sv.subItems[addDeviceTarget.value]
  if (!detail) return
  // 避免重复添加
  if (detail.matchedDevices.some(d => d.hwUid === hw.uid)) return
  detail.matchedDevices.push({ hwUid: hw.uid, cleanModel: hw.cleanModel || hw.rawName })
  showDevicePicker.value = false
  addDeviceTarget.value = null
}

function onRemoveDeviceFromSubItem(detail, hwUid) {
  detail.matchedDevices = detail.matchedDevices.filter(d => d.hwUid !== hwUid)
}

function toggleSubItem(detail) {
  detail.isExpanded = !detail.isExpanded
}

// ─── Actions ─────────────────────────────────────────────────────────────────
function handleConfirm() {
  // Aggregate allocations from all sub-items per hardware device
  const hwAllocMap = new Map() // hwUid → totalAllocatedAmount
  for (const sv of serviceItems.value) {
    for (const sub of (Array.isArray(sv.subItems) ? sv.subItems : [])) {
      const subAlloc = computeSubItemAllocation(sub)
      for (const [hwUid, amt] of subAlloc) {
        hwAllocMap.set(hwUid, (hwAllocMap.get(hwUid) || 0) + amt)
      }
    }
  }

  // Build final hardware items with service fees added to unit price
  const hwItems = hardwareItems.value.map(hw => {
    const allocated = Math.round((hwAllocMap.get(hw.uid) || 0) * 100) / 100
    const allocPerUnit = hw.quantity > 0 ? Math.round((allocated / hw.quantity) * 100) / 100 : 0
    const compositePrice = Math.round(((hw.unitPrice || 0) + allocPerUnit) * 100) / 100
    return {
      ...hw,
      allocatedAmount: allocated,
      allocPerUnit,
      compositePrice,
    }
  })

  emit('confirm', {
    hardwareItems: hwItems,
    serviceItems: [...serviceItems.value],
    totalServiceFee: totalServiceFee.value,
    allocationMethod: allocationStrategy.value,
  })
}

function handleCancel() {
  emit('cancel')
}

defineExpose({ stage, handleConfirm })

function resetState() {
  fileName.value = ''
  parseError.value = ''
  rawRows.value = []
  hardwareItems.value = []
  serviceItems.value = []
  totalServiceFee.value = 0
  selectedServiceIndex.value = 0
  stage.value = 'upload'
  allocationStrategy.value = 'ratio'
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Stage: Upload -->
    <template v-if="stage === 'upload'">
      <div class="p-6 space-y-4">
        <p class="text-xs text-gray-500">
          选择华为 SCT / eDesigner 导出的报价单文件（.xlsx），系统将自动解析设备名称、
          提取服务费项并按比例分摊到各硬件设备。
        </p>
        <div
          @click="triggerFileInput"
          @dragover.prevent="onDragOver"
          @dragenter.prevent="onDragOver"
          @dragleave="onDragLeave"
          @drop.prevent="onDragDrop"
          class="border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-colors"
          :class="dragActive ? 'border-blue-400 bg-blue-50/40' : 'border-gray-200 hover:border-blue-400 hover:bg-blue-50/30'"
        >
          <input
            ref="fileInput"
            type="file"
            accept=".xlsx,.xls,.csv"
            class="hidden"
            @change="onFileSelected"
          />
          <AppIcon name="upload" class="w-8 h-8 mx-auto mb-3 text-gray-300" />
          <p class="text-xs text-gray-400">拖拽 .xlsx / .csv 文件到此处或点击选择</p>
        </div>
      </div>
    </template>

    <!-- Stage: Parsing -->
    <template v-else-if="stage === 'parsing'">
      <div class="flex-1 flex items-center justify-center py-16">
        <div class="text-center space-y-3">
          <AppIcon name="spinner" class="w-6 h-6 mx-auto text-blue-500 animate-spin" />
          <p class="text-sm text-gray-500">正在解析 {{ fileName }}...</p>
        </div>
      </div>
    </template>

    <!-- Stage: Allocate -->
    <template v-else-if="stage === 'allocate'">
      <!-- Summary bar -->
      <div class="px-4 py-3 border-b border-gray-100 bg-gray-50/50 flex items-center gap-6 text-xs shrink-0">
        <div>
          <span class="text-gray-400">硬件设备</span>
          <span class="ml-1.5 font-semibold text-gray-700">{{ hardwareItems.length }} 项</span>
        </div>
        <div>
          <span class="text-gray-400">服务项</span>
          <span class="ml-1.5 font-semibold text-gray-700">{{ serviceItems.length }} 项</span>
        </div>
        <div>
          <span class="text-gray-400">待分配金额</span>
          <span class="ml-1.5 font-semibold" :class="unallocatedAmount > 0 ? 'text-amber-600' : 'text-green-600'">¥{{ unallocatedAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
        </div>
        <div class="ml-auto flex items-center gap-2">
          <label class="flex items-center gap-1 cursor-pointer select-none">
            <input
              type="checkbox"
              :checked="allocationStrategy === 'ratio'"
              @change="allocationStrategy = 'ratio'"
              class="w-3 h-3 rounded border-gray-300 text-blue-600 focus:ring-blue-500 cursor-pointer"
            />
            <span class="text-[11px] text-gray-500">按金额比例</span>
          </label>
          <label class="flex items-center gap-1 cursor-pointer select-none">
            <input
              type="checkbox"
              :checked="allocationStrategy === 'equal'"
              @change="allocationStrategy = 'equal'"
              class="w-3 h-3 rounded border-gray-300 text-blue-600 focus:ring-blue-500 cursor-pointer"
            />
            <span class="text-[11px] text-gray-500">平均分配</span>
          </label>
        </div>
      </div>

      <!-- Two-column: Service list + Detail panel -->
      <div class="flex flex-1 min-h-0">
        <!-- Left: Service items list -->
        <div class="w-56 border-r border-gray-100 overflow-y-auto shrink-0 bg-gray-50/30">
          <div class="text-[10px] font-medium text-gray-400 uppercase px-3 pt-3 pb-1.5">服务费明细</div>
          <div class="px-2 pb-2 space-y-0.5">
            <div
              v-for="(sv, idx) in serviceItems"
              :key="sv.uid"
              @click="selectedServiceIndex = idx"
              class="flex flex-col text-xs py-1.5 px-2 rounded cursor-pointer transition-colors"
              :class="selectedServiceIndex === idx ? 'bg-blue-50 text-blue-700 ring-1 ring-blue-200/50' : 'hover:bg-white text-gray-600'"
            >
              <span class="truncate text-[12px] leading-tight">{{ sv.name }}</span>
              <span class="text-[11px] font-mono mt-0.5" :class="selectedServiceIndex === idx ? 'text-blue-600' : 'text-amber-600'">¥{{ sv.totalAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
            </div>
          </div>
          <div v-if="serviceItems.length === 0" class="px-3 py-4 text-xs text-gray-400 italic">未识别到服务项</div>
        </div>

        <!-- Right: Sub-items panel -->
        <div class="flex-1 flex flex-col min-w-0">
          <template v-if="selectedService">
            <!-- Selected service header -->
            <div class="px-4 py-2 border-b border-gray-100 flex items-center gap-3">
              <span class="text-xs font-semibold text-gray-700">{{ selectedService.name }}</span>
              <span class="text-xs font-mono text-blue-600">¥{{ selectedService.totalAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
            </div>

            <!-- Sub-items list -->
            <div class="flex-1 overflow-y-auto min-h-0 px-3 py-2 space-y-2 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
              <template v-if="Array.isArray(selectedService.subItems) && selectedService.subItems.length > 0">
                <div
                  v-for="(detail, di) in selectedService.subItems"
                  :key="di"
                  class="border border-gray-200 rounded-lg bg-white overflow-hidden transition-colors"
                  :class="detail.isExpanded ? 'shadow-sm border-blue-200' : ''"
                >
                  <!-- Sub-item top row -->
                  <div class="flex items-center gap-1.5 px-2.5 py-1.5">
                    <!-- Amount input -->
                    <div class="relative w-20">
                      <span class="absolute left-1.5 top-1/2 -translate-y-1/2 text-[10px] text-gray-400 font-mono pointer-events-none">¥</span>
                      <input
                        v-model.number="detail.serviceAmount"
                        type="number"
                        step="100"
                        min="0"
                        placeholder="0"
                        class="w-full text-right bg-gray-50/80 border border-gray-200 rounded pl-4 pr-1.5 py-0.5 text-xs text-gray-700 outline-none focus:border-blue-400 focus:ring-1 focus:ring-blue-200 [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none font-mono"
                      />
                    </div>
                    <!-- Content text -->
                    <span class="flex-1 min-w-0 text-xs text-gray-600 truncate">{{ detail.content }}</span>
                    <!-- Matched count badge -->
                    <span v-if="detail.matchedDevices.length > 0" class="text-[10px] text-green-600 bg-green-50 px-1.5 py-0.5 rounded font-medium flex-shrink-0">
                      {{ totalMatchedQty(detail) }} 台
                    </span>
                    <!-- Collapse toggle -->
                    <button @click="toggleSubItem(detail)" class="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors flex-shrink-0">
                      <AppIcon name="chevron-down" class="w-3 h-3 transition-transform duration-150" :class="detail.isExpanded ? '' : '-rotate-90'" />
                    </button>
                    <!-- Add device button -->
                    <button @click="openDevicePicker(di)" class="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-blue-600 hover:bg-blue-50 transition-colors flex-shrink-0" title="添加设备匹配">
                      <AppIcon name="plus" class="w-3 h-3" />
                    </button>
                  </div>

                  <!-- Expanded: matched devices -->
                  <div v-if="detail.isExpanded && detail.matchedDevices.length > 0" class="border-t border-gray-100 px-2.5 py-1.5 space-y-1 bg-gray-50/50">
                    <div
                      v-for="(md, mi) in detail.matchedDevices"
                      :key="mi"
                      class="flex items-center text-xs"
                    >
                      <div class="grid items-center gap-1.5 flex-1 min-w-0" style="grid-template-columns: 100px 140px 36px 72px">
                        <span v-if="findHwByUid(md.hwUid)?.customName" class="text-[11px] text-gray-500 truncate">
                          {{ findHwByUid(md.hwUid).customName }}
                        </span>
                        <span v-else class="text-[11px] text-gray-500 truncate" />
                        <span class="text-[11px] font-medium text-gray-700 font-mono truncate">{{ md.cleanModel }}</span>
                        <span v-if="findHwByUid(md.hwUid)" class="text-[10px] text-gray-400 text-right">
                          {{ findHwByUid(md.hwUid).quantity }}{{ findHwByUid(md.hwUid).unit || '台' }}
                        </span>
                        <span
                          class="text-[10px] text-blue-600 font-mono text-right"
                        >¥{{ getAllocPreview(selectedServiceIndex, di, md.hwUid).toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
                      </div>
                      <button @click="onRemoveDeviceFromSubItem(detail, md.hwUid)" class="w-4 h-4 flex items-center justify-center rounded text-gray-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0 ml-1">
                        <AppIcon name="x" class="w-2.5 h-2.5" />
                      </button>
                    </div>
                  </div>
                  <div v-else-if="detail.isExpanded" class="border-t border-gray-100 px-2.5 py-2 text-[10px] text-gray-400 italic bg-gray-50/50">
                    未匹配设备，点击 + 添加
                  </div>
                </div>
              </template>
              <p v-else class="text-xs text-gray-400 italic py-4 text-center">此服务项下暂无明细子项</p>
            </div>
          </template>

          <!-- No service selected -->
          <div v-else class="flex-1 flex items-center justify-center">
            <p class="text-sm text-gray-400">选择左侧服务项查看子项明细</p>
          </div>

        <!-- Device picker popup -->
        <Teleport to="body">
          <Transition name="confirm-fade">
            <div v-if="showDevicePicker" class="fixed inset-0 z-[80] flex items-center justify-center">
              <div class="absolute inset-0 bg-black/30" />
              <div class="relative bg-white rounded-xl shadow-xl w-full max-w-md mx-4 max-h-[70vh] flex flex-col z-10">
                <div class="px-4 py-3 border-b border-gray-100 flex items-center justify-between shrink-0">
                  <h4 class="text-sm font-semibold text-gray-700">选择设备型号</h4>
                  <button @click="showDevicePicker = false" class="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-gray-600"><AppIcon name="x" class="w-3.5 h-3.5" /></button>
                </div>
                <div class="flex-1 overflow-y-auto p-2 space-y-1">
                  <div
                    v-for="hw in hardwareItems"
                    :key="hw.uid"
                    @click="onSelectDeviceForSubItem(hw)"
                    class="flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer hover:bg-blue-50 transition-colors"
                  >
                    <div class="flex-1 min-w-0">
                      <span class="text-xs font-medium text-gray-700 truncate block">{{ hw.cleanModel || hw.rawName }}</span>
                      <span class="text-[10px] text-gray-400">{{ hw.customName || hw.rawName }}</span>
                    </div>
                    <span class="text-[10px] text-gray-400 flex-shrink-0">{{ hw.quantity }}{{ hw.unit }}</span>
                  </div>
                  <div v-if="hardwareItems.length === 0" class="py-6 text-center text-xs text-gray-400">未解析到硬件设备</div>
                </div>
              </div>
            </div>
          </Transition>
        </Teleport>
      </div>
      </div>

      <!-- Bottom bar -->
      <div class="px-4 py-2.5 border-t border-gray-200 bg-gray-50/80 flex items-center justify-between shrink-0">
        <div class="flex items-center gap-3 text-xs">
          <span class="text-gray-500">
            硬件合计 <span class="font-semibold text-gray-700 font-mono">¥{{ totalHardwareCost.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
          </span>
          <span class="text-gray-300">+</span>
          <span class="text-gray-500">
            服务费总额 <span class="font-semibold text-blue-600 font-mono">¥{{ totalServiceFee.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
          </span>
          <template v-if="allocatedSubAmount > 0">
            <span class="text-gray-300">-</span>
            <span class="text-gray-500">
              已分配 <span class="font-semibold text-green-600 font-mono">¥{{ allocatedSubAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
            </span>
            <span class="text-gray-300">=</span>
            <span class="font-semibold font-mono" :class="unallocatedAmount > 0 ? 'text-amber-600' : 'text-green-600'">¥{{ unallocatedAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span>
          </template>
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="handleConfirm"
            :disabled="!isAllAllocated"
            class="btn-primary"
            :class="!isAllAllocated ? 'opacity-50 cursor-not-allowed' : ''"
          >确认导入 {{ hardwareItems.length }} 项</button>
        </div>
      </div>
    </template>

    <!-- Parse Error -->
    <template v-if="parseError">
      <div class="p-6 text-center">
        <AppIcon name="alert-circle" class="w-6 h-6 mx-auto mb-2 text-red-400" />
        <p class="text-sm text-red-600 mb-4">{{ parseError }}</p>
        <button @click="resetState" class="btn-secondary text-xs">重新选择文件</button>
      </div>
    </template>
  </div>
</template>
