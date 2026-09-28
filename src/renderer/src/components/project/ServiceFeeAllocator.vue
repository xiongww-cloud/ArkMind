<script setup lang="ts">
/**
 * ServiceFeeAllocator — 华为 SCT 报价单专用：硬件设备 + 服务费分摊
 *
 * 由 SolutionConfigurator 传入已识别的 SCT 报价单 SheetData（复用 excelRaw/SheetJS 解析）。
 * 分摊逻辑：每个服务父项的服务费总额(totalAmount)直接分摊到其匹配的硬件设备上，
 * 而不是分摊到服务细项。设备匹配可在服务级别手动增删。
 *
 *   1. parseSCTSheet → 硬件设备 + 服务父项（子项用于自动匹配设备）
 *   2. 服务费分摊 UI（按金额比例 / 平均分配，作用于服务 → 设备）
 *   3. confirm 时输出带 compositePrice 的硬件清单
 */
import { ref, computed, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { parseSCTSheet, type SCTHardwareItem, type SCTServiceItem } from '@/utils/sctExcel'
import type { SheetData } from '@/utils/excelRaw'

const props = defineProps<{
  sheet: SheetData | null
}>()

const emit = defineEmits<{
  confirm: [
    payload: {
      hardwareItems: SCTHardwareItem[]
      serviceItems: SCTServiceItem[]
      totalServiceFee: number
      allocationMethod: string
    }
  ]
  cancel: []
}>()

// ─── Parsed Data ─────────────────────────────────────────────────────────────
const hardwareItems = ref<SCTHardwareItem[]>([])
const serviceItems = ref<SCTServiceItem[]>([])
const parseError = ref('')

const totalServiceFee = computed(() =>
  serviceItems.value.reduce((s, sv) => s + (Number(sv.totalAmount) || 0), 0)
)

/** 已分摊服务费 = 有匹配设备的服务项金额之和 */
const allocatedSubAmount = computed(() => {
  let total = 0
  for (const sv of serviceItems.value) {
    if (serviceMatchedDevices(sv).length > 0) {
      total += Number(sv.totalAmount) || 0
    }
  }
  return total
})

const unallocatedAmount = computed(() => totalServiceFee.value - allocatedSubAmount.value)

const selectedServiceIndex = ref(0)

// ─── Allocation State ────────────────────────────────────────────────────────
const allocationStrategy = ref<'ratio' | 'equal'>('ratio')

const totalHardwareCost = computed(() =>
  hardwareItems.value.reduce((s, hw) => s + (hw.unitPrice || 0) * (hw.quantity || 0), 0)
)

/** 每个服务项都匹配到了设备（服务费全部可分摊）才能确认 */
const isAllAllocated = computed(() => {
  if (serviceItems.value.length === 0) return true
  return serviceItems.value.every((sv) => serviceMatchedDevices(sv).length > 0)
})

// ─── Parse on sheet mount ────────────────────────────────────────────────────
watch(
  () => props.sheet,
  (sheet) => {
    resetState()
    if (!sheet) return
    try {
      const { hardwareItems: hw, serviceItems: sv } = parseSCTSheet(sheet)
      hardwareItems.value = hw
      serviceItems.value = sv
      if (serviceItems.value.length === 0) {
        // 无服务父项：直接确认导入硬件设备
        emit('confirm', {
          hardwareItems: hw.map((item) => ({
            ...item,
            allocatedAmount: 0,
            allocPerUnit: 0,
            compositePrice: item.unitPrice || 0
          })),
          serviceItems: [],
          totalServiceFee: 0,
          allocationMethod: 'ratio'
        })
        return
      }
    } catch (err: any) {
      parseError.value = '解析失败：' + (err?.message || '未知错误')
      console.error('SCT parse error:', err)
    }
  },
  { immediate: true }
)

function resetState() {
  parseError.value = ''
  hardwareItems.value = []
  serviceItems.value = []
  selectedServiceIndex.value = 0
  allocationStrategy.value = 'ratio'
}

// ─── 服务项 → 匹配设备（聚合子项匹配，去重） ─────────────────────────────
function serviceMatchedDevices(sv: SCTServiceItem): Array<{ hwUid: string; cleanModel: string }> {
  const seen = new Set<string>()
  const out: Array<{ hwUid: string; cleanModel: string }> = []
  for (const sub of Array.isArray(sv.subItems) ? sv.subItems : []) {
    for (const md of sub.matchedDevices || []) {
      if (!seen.has(md.hwUid)) {
        seen.add(md.hwUid)
        out.push(md)
      }
    }
  }
  return out
}

// ─── 服务费 → 设备分摊（按服务总额分摊到该服务的匹配设备） ─────────────
function computeServiceAllocation(sv: SCTServiceItem): Map<string, number> {
  const amount = Number(sv.totalAmount) || 0
  const devices = serviceMatchedDevices(sv).filter((d) => d.hwUid)
  const result = new Map<string, number>()
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
  const devicesWithCost = devices.map((d) => {
    const hw = hardwareItems.value.find((h) => h.uid === d.hwUid)
    const cost = hw ? (hw.unitPrice || 0) * (hw.quantity || 0) : 0
    return { ...d, cost }
  })
  const totalCost = devicesWithCost.reduce((s, d) => s + d.cost, 0)

  if (totalCost <= 0) {
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

// ── 实时分摊预览：key="svIdx" → Map<hwUid, amount> ─────────────────────
const previewAllocation = computed(() => {
  const map: Record<string, Map<string, number>> = {}
  for (let si = 0; si < serviceItems.value.length; si++) {
    map[`${si}`] = computeServiceAllocation(serviceItems.value[si])
  }
  return map
})

const selectedService = computed(() => serviceItems.value[selectedServiceIndex.value] || null)

function findHwByUid(uid: string) {
  return hardwareItems.value.find((hw) => hw.uid === uid) || null
}

function getAllocPreview(svIdx: number, hwUid: string) {
  return previewAllocation.value[`${svIdx}`]?.get(hwUid) || 0
}

function getServiceMatchedQty(sv: SCTServiceItem) {
  return serviceMatchedDevices(sv).reduce((sum, md) => {
    const hw = findHwByUid(md.hwUid)
    return sum + (hw ? hw.quantity : 0)
  }, 0)
}

// ── 添加设备（服务级别） ─────────────────────────────────────────────────
const showDevicePicker = ref(false)

function openDevicePicker() {
  showDevicePicker.value = true
}

function onSelectDeviceForService(hw: SCTHardwareItem) {
  const sv = selectedService.value
  if (!sv) return
  if (serviceMatchedDevices(sv).some((d) => d.hwUid === hw.uid)) return
  // 写入该服务的第一个子项（聚合读取），便于持久化
  const target = Array.isArray(sv.subItems) && sv.subItems.length > 0 ? sv.subItems[0] : null
  if (!target) return
  target.matchedDevices.push({ hwUid: hw.uid, cleanModel: hw.cleanModel || hw.rawName })
  showDevicePicker.value = false
}

function onRemoveDeviceFromService(sv: SCTServiceItem, hwUid: string) {
  for (const sub of Array.isArray(sv.subItems) ? sv.subItems : []) {
    sub.matchedDevices = sub.matchedDevices.filter((d: any) => d.hwUid !== hwUid)
  }
}

// ─── Actions ─────────────────────────────────────────────────────────────────
function handleConfirm() {
  const hwAllocMap = new Map<string, number>()
  for (const sv of serviceItems.value) {
    const svAlloc = computeServiceAllocation(sv)
    for (const [hwUid, amt] of svAlloc) {
      hwAllocMap.set(hwUid, (hwAllocMap.get(hwUid) || 0) + amt)
    }
  }

  const hwItems = hardwareItems.value.map((hw) => {
    const allocated = Math.round((hwAllocMap.get(hw.uid) || 0) * 100) / 100
    const allocPerUnit = hw.quantity > 0 ? Math.round((allocated / hw.quantity) * 100) / 100 : 0
    const compositePrice = Math.round(((hw.unitPrice || 0) + allocPerUnit) * 100) / 100
    return {
      ...hw,
      allocatedAmount: allocated,
      allocPerUnit,
      compositePrice
    }
  })

  emit('confirm', {
    hardwareItems: hwItems,
    serviceItems: [...serviceItems.value],
    totalServiceFee: totalServiceFee.value,
    allocationMethod: allocationStrategy.value
  })
}

function handleCancel() {
  emit('cancel')
}

defineExpose({ handleConfirm })
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Parse Error -->
    <template v-if="parseError">
      <div class="p-6 text-center">
        <AppIcon name="alert-circle" class="w-6 h-6 mx-auto mb-2 text-red-400" />
        <p class="text-sm text-red-600 mb-4">{{ parseError }}</p>
        <button @click="emit('cancel')" class="btn-secondary text-xs">关闭</button>
      </div>
    </template>

    <!-- Empty / No service -->
    <template v-else-if="serviceItems.length === 0">
      <div class="flex-1 flex items-center justify-center">
        <p class="text-sm text-gray-400">未识别到服务费项，或已自动确认导入硬件设备</p>
      </div>
    </template>

    <!-- Allocate -->
    <template v-else>
      <!-- Summary bar -->
      <div
        class="px-4 py-3 border-b border-gray-100 bg-gray-50/50 flex items-center gap-6 text-xs shrink-0"
      >
        <div>
          <span class="text-gray-400">硬件设备</span>
          <span class="ml-1.5 font-semibold text-gray-700">{{ hardwareItems.length }} 项</span>
        </div>
        <div>
          <span class="text-gray-400">服务费</span>
          <span class="ml-1.5 font-semibold text-gray-700">{{ serviceItems.length }} 项</span>
        </div>
        <div>
          <span class="text-gray-400">待分配金额</span>
          <span
            class="ml-1.5 font-semibold"
            :class="unallocatedAmount > 0 ? 'text-amber-600' : 'text-green-600'"
            >¥{{ unallocatedAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span
          >
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

      <!-- Two-column: Service list + Device allocation panel -->
      <div class="flex flex-1 min-h-0">
        <!-- Left: Service items list -->
        <div class="w-56 border-r border-gray-100 overflow-y-auto shrink-0 bg-gray-50/30">
          <div class="text-[10px] font-medium text-gray-400 uppercase px-3 pt-3 pb-1.5">
            服务费明细
          </div>
          <div class="px-2 pb-2 space-y-0.5">
            <div
              v-for="(sv, idx) in serviceItems"
              :key="sv.uid"
              @click="selectedServiceIndex = idx"
              class="flex flex-col text-xs py-1.5 px-2 rounded cursor-pointer transition-colors"
              :class="
                selectedServiceIndex === idx
                  ? 'bg-blue-50 text-blue-700 ring-1 ring-blue-200/50'
                  : 'hover:bg-white text-gray-600'
              "
            >
              <span class="truncate text-[12px] leading-tight">{{ sv.name }}</span>
              <span
                class="text-[11px] font-mono mt-0.5"
                :class="selectedServiceIndex === idx ? 'text-blue-600' : 'text-amber-600'"
                >¥{{ sv.totalAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span
              >
            </div>
          </div>
          <div v-if="serviceItems.length === 0" class="px-3 py-4 text-xs text-gray-400 italic">
            未识别到服务费
          </div>
        </div>

        <!-- Right: Selected service → matched devices -->
        <div class="flex-1 flex flex-col min-w-0">
          <template v-if="selectedService">
            <div class="px-4 py-2 border-b border-gray-100 flex items-center gap-3">
              <span class="text-xs font-semibold text-gray-700">{{ selectedService.name }}</span>
              <span class="text-xs font-mono text-blue-600"
                >¥{{
                  selectedService.totalAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 })
                }}</span
              >
              <span class="flex-1"></span>
              <button
                @click="openDevicePicker"
                class="flex items-center gap-1 px-2 py-1 rounded-md text-[11px] text-blue-600 hover:bg-blue-50 transition-colors"
                v-tooltip="'添加设备'"
              >
                <AppIcon name="plus" class="w-3 h-3" /> 添加设备
              </button>
            </div>

            <div class="flex-1 overflow-y-auto min-h-0 px-3 py-2">
              <div
                v-if="serviceMatchedDevices(selectedService).length > 0"
                class="border border-gray-200 rounded-lg bg-white overflow-hidden"
              >
                <div
                  class="px-3 py-1.5 bg-gray-50/60 border-b border-gray-100 flex items-center gap-3 text-[11px] text-gray-500"
                >
                  <span class="flex-1">设备</span>
                  <span class="w-16 text-right">数量</span>
                  <span class="w-20 text-right">分摊金额</span>
                  <span class="w-4"></span>
                </div>
                <div
                  v-for="md in serviceMatchedDevices(selectedService)"
                  :key="md.hwUid"
                  class="flex items-center gap-3 px-3 py-2 border-b border-gray-50 last:border-b-0 text-xs"
                >
                  <div class="flex-1 min-w-0">
                    <span class="text-xs font-medium text-gray-700 truncate block">{{
                      findHwByUid(md.hwUid)?.cleanModel || md.cleanModel
                    }}</span>
                    <span
                      v-if="findHwByUid(md.hwUid)?.customName"
                      class="text-[10px] text-gray-400"
                      >{{ findHwByUid(md.hwUid)?.customName }}</span
                    >
                  </div>
                  <span class="w-16 text-right text-gray-600 flex-shrink-0"
                    >{{ findHwByUid(md.hwUid)?.quantity
                    }}{{ findHwByUid(md.hwUid)?.unit || '台' }}</span
                  >
                  <span class="w-20 text-right text-blue-600 font-mono flex-shrink-0"
                    >¥{{
                      getAllocPreview(selectedServiceIndex, md.hwUid).toLocaleString('zh-CN', {
                        minimumFractionDigits: 2
                      })
                    }}</span
                  >
                  <button
                    @click="onRemoveDeviceFromService(selectedService, md.hwUid)"
                    class="w-4 h-4 flex items-center justify-center rounded text-gray-300 hover:text-red-500 hover:bg-red-50 transition-colors flex-shrink-0"
                  >
                    <AppIcon name="x" class="w-2.5 h-2.5" />
                  </button>
                </div>
              </div>
              <div
                v-else
                class="border border-dashed border-gray-200 rounded-lg py-8 text-center text-xs text-gray-400"
              >
                该服务项尚未匹配设备，点击右上角「添加设备」
              </div>
            </div>
          </template>

          <div v-else class="flex-1 flex items-center justify-center">
            <p class="text-sm text-gray-400">选择左侧服务项查看分摊明细</p>
          </div>

          <!-- Device picker popup -->
          <Teleport to="body">
            <Transition name="confirm-fade">
              <div
                v-if="showDevicePicker"
                class="fixed inset-0 z-[100] flex items-center justify-center"
              >
                <div class="absolute inset-0 bg-black/30" />
                <div
                  class="relative bg-white rounded-xl shadow-xl w-full max-w-md mx-4 max-h-[70vh] flex flex-col z-10"
                >
                  <div
                    class="px-4 py-3 border-b border-gray-100 flex items-center justify-between shrink-0"
                  >
                    <h4 class="text-sm font-semibold text-gray-700">选择设备型号</h4>
                    <button
                      @click="showDevicePicker = false"
                      class="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-gray-600"
                    >
                      <AppIcon name="x" class="w-3.5 h-3.5" />
                    </button>
                  </div>
                  <div class="flex-1 overflow-y-auto p-2 space-y-1">
                    <div
                      v-for="hw in hardwareItems"
                      :key="hw.uid"
                      @click="onSelectDeviceForService(hw)"
                      class="flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer hover:bg-blue-50 transition-colors"
                    >
                      <div class="flex-1 min-w-0">
                        <span class="text-xs font-medium text-gray-700 truncate block">{{
                          hw.cleanModel || hw.rawName
                        }}</span>
                        <span class="text-[10px] text-gray-400">{{
                          hw.customName || hw.rawName
                        }}</span>
                      </div>
                      <span class="text-[10px] text-gray-400 flex-shrink-0"
                        >{{ hw.quantity }}{{ hw.unit }}</span
                      >
                    </div>
                    <div
                      v-if="hardwareItems.length === 0"
                      class="py-6 text-center text-xs text-gray-400"
                    >
                      未解析到硬件设备
                    </div>
                  </div>
                </div>
              </div>
            </Transition>
          </Teleport>
        </div>
      </div>

      <!-- Bottom bar -->
      <div
        class="px-4 py-2.5 border-t border-gray-200 bg-gray-50/80 flex items-center justify-between shrink-0"
      >
        <div class="flex items-center gap-3 text-xs">
          <span class="text-gray-500">
            硬件合计
            <span class="font-semibold text-gray-700 font-mono"
              >¥{{ totalHardwareCost.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span
            >
          </span>
          <span class="text-gray-300">+</span>
          <span class="text-gray-500">
            服务费总额
            <span class="font-semibold text-blue-600 font-mono"
              >¥{{ totalServiceFee.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span
            >
          </span>
          <template v-if="allocatedSubAmount > 0">
            <span class="text-gray-300">-</span>
            <span class="text-gray-500">
              已分摊
              <span class="font-semibold text-green-600 font-mono"
                >¥{{
                  allocatedSubAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 })
                }}</span
              >
            </span>
            <span class="text-gray-300">=</span>
            <span
              class="font-semibold font-mono"
              :class="unallocatedAmount > 0 ? 'text-amber-600' : 'text-green-600'"
              >¥{{ unallocatedAmount.toLocaleString('zh-CN', { minimumFractionDigits: 2 }) }}</span
            >
          </template>
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="handleConfirm"
            :disabled="!isAllAllocated"
            class="btn-primary"
            :class="!isAllAllocated ? 'opacity-50 cursor-not-allowed' : ''"
          >
            确认导入 {{ hardwareItems.length }} 项
          </button>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.15s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>
