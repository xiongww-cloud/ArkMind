<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { toast } from '@/composables/useToast'
import { contactsListChannels, contactsListVendors, customerList, createPartner, createPartnerContact } from '@/api/db-client'

type TabType = 'channels' | 'vendors' | 'customers'
interface Prefill { name?: string; position?: string; email?: string; phone?: string; partnerId?: string; partnerName?: string }

const props = defineProps<{ visible: boolean; prefill?: Prefill }>()
const emit = defineEmits<{ close: []; created: [payload: { partnerId: string; tab: TabType }] }>()

const form = ref({ name: '', phone: '', email: '', position: '', remark: '' })
const partnerPick = ref('')
const newCompany = ref<{ name: string; type: TabType }>({ name: '', type: 'customers' })
const saving = ref(false)

const channels = ref<any[]>([])
const vendors = ref<any[]>([])
const customers = ref<any[]>([])

const TAB_LABELS: [TabType, string, string][] = [['channels', '渠道伙伴', '渠道'], ['vendors', '供应商', '供应商'], ['customers', '客户', '客户']]
const presetPositions = ['总经理', '副总', '经理', '总监', '主管', '工程师', '技术负责人', '采购', '财务', '行政', '其他']

async function loadPartners() {
  try {
    const [ch, vd, cs] = await Promise.all([contactsListChannels(), contactsListVendors(), customerList()])
    channels.value = ch || []; vendors.value = vd || []; customers.value = cs || []
  } catch (e) { console.error('[ContactQuickCreate] load partners failed:', e) }
}

const groups = computed(() => {
  const listMap: Record<TabType, any[]> = { channels: channels.value, vendors: vendors.value, customers: customers.value }
  const out: { header: string; items: { value: string; label: string }[] }[] = []
  for (const [tab, header] of TAB_LABELS) {
    const items = listMap[tab].map((e: any) => ({ value: `${tab}:${e.id}`, label: e.name }))
    if (items.length) out.push({ header, items })
  }
  return out
})

const selectedLabel = computed(() => {
  if (partnerPick.value === '__new__') return '＋ 新建公司'
  for (const g of groups.value) { const hit = g.items.find(i => i.value === partnerPick.value); if (hit) return hit.label }
  return ''
})

function matchPartner(prefill: Prefill): string {
  const all: [TabType, any[]][] = [['channels', channels.value], ['vendors', vendors.value], ['customers', customers.value]]
  if (prefill.partnerId) {
    for (const [tab, list] of all) { if (list.some(e => e.id === prefill.partnerId)) return `${tab}:${prefill.partnerId}` }
  }
  if (prefill.partnerName) {
    const pn = prefill.partnerName.toLowerCase().trim()
    for (const [tab, list] of all) {
      const found = list.find((e: any) => {
        const en = (e.name || '').toLowerCase().trim()
        return !!en && (en === pn || (en.length >= 3 && pn.includes(en)) || (pn.length >= 3 && en.includes(pn)))
      })
      if (found) return `${tab}:${found.id}`
    }
  }
  return ''
}

watch(() => props.visible, async (show) => {
  if (!show) { dropdownOpen.value = false; return }
  await loadPartners()
  const pf = props.prefill || {}
  form.value = { name: pf.name || '', phone: pf.phone || '', email: pf.email || '', position: pf.position || '', remark: '' }
  const hit = matchPartner(pf)
  partnerPick.value = hit || '__new__'
  newCompany.value = { name: hit ? '' : (pf.partnerName || ''), type: 'customers' }
})

// ── 自定义归属下拉 ──
const dropdownOpen = ref(false)
const dropdownRef = ref<HTMLElement | null>(null)
function onDocMouseDown(e: MouseEvent) {
  if (dropdownRef.value && !dropdownRef.value.contains(e.target as Node)) dropdownOpen.value = false
}
watch(dropdownOpen, (o) => {
  if (o) document.addEventListener('mousedown', onDocMouseDown)
  else document.removeEventListener('mousedown', onDocMouseDown)
})
onUnmounted(() => document.removeEventListener('mousedown', onDocMouseDown))

function pick(value: string) {
  partnerPick.value = value
  dropdownOpen.value = false
}

async function save() {
  if (!form.value.name.trim()) { toast({ message: '请输入联系人姓名', type: 'error' }); return }
  if (partnerPick.value === '__new__' && !newCompany.value.name.trim()) { toast({ message: '请输入公司名称，或选择已有公司', type: 'error' }); return }
  if (partnerPick.value !== '__new__' && !partnerPick.value.includes(':')) { toast({ message: '请选择归属公司', type: 'error' }); return }
  saving.value = true
  try {
    let partnerId = ''
    let tab: TabType = 'customers'
    if (partnerPick.value === '__new__') {
      const typeEnum: Record<TabType, string> = { channels: 'CHANNEL', vendors: 'VENDOR', customers: 'CUSTOMER' }
      const r: any = await createPartner({ name: newCompany.value.name.trim(), type: typeEnum[newCompany.value.type] })
      partnerId = r?.id || ''
      tab = newCompany.value.type
      if (!partnerId) throw new Error('公司创建失败')
    } else {
      const [t, pid] = partnerPick.value.split(':')
      partnerId = pid; tab = t as TabType
    }
    const cr: any = await createPartnerContact({ partnerId, ...form.value })
    if (!cr?.id && !cr?.success) throw new Error('联系人创建失败')
    const compName = partnerPick.value === '__new__' ? newCompany.value.name.trim() : selectedLabel.value
    toast({ message: `联系人「${form.value.name.trim()}」已创建（归属：${compName}）`, type: 'success' })
    emit('created', { partnerId, tab })
    emit('close')
  } catch (e: any) {
    toast({ message: '创建失败: ' + (e?.message || e), type: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="visible" class="fixed inset-0 z-[120] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="emit('close')" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-sm mx-4 p-5 z-10 max-h-[85vh] overflow-y-auto">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">新建联系人</h3>
          <div class="space-y-2.5">
            <div ref="dropdownRef" class="relative">
              <label class="text-[11px] text-surface-500 mb-1 block">归属公司 *</label>
              <button type="button" @click="dropdownOpen = !dropdownOpen"
                class="w-full flex items-center justify-between gap-1 h-7 px-3 text-[12px] rounded-lg border bg-white transition-colors"
                :class="dropdownOpen ? 'border-brand-500 ring-2 ring-brand-500/20' : 'border-surface-200 hover:border-surface-300'">
                <span class="truncate" :class="selectedLabel ? 'text-surface-800' : 'text-surface-400'">{{ selectedLabel || '选择公司' }}</span>
                <AppIcon name="chevron-down" size="12" class="shrink-0 text-surface-400 transition-transform" :class="dropdownOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="dropdownOpen" class="absolute top-full left-0 mt-1 w-full bg-white rounded-lg shadow-lg border border-surface-100 py-1 z-20 max-h-[220px] overflow-y-auto">
                <template v-for="g in groups" :key="g.header">
                  <div class="px-3 pt-1.5 pb-0.5 text-[10px] font-medium text-surface-400">{{ g.header }}</div>
                  <button v-for="o in g.items" :key="o.value" type="button" @click="pick(o.value)"
                    class="w-full flex items-center justify-between px-3 py-1.5 text-left text-xs transition-colors"
                    :class="partnerPick === o.value ? 'text-brand-600 bg-brand-50/50 font-medium' : 'text-surface-700 hover:bg-surface-50'">
                    <span class="truncate">{{ o.label }}</span>
                    <AppIcon v-if="partnerPick === o.value" name="check" size="12" class="shrink-0" />
                  </button>
                </template>
                <div class="border-t border-surface-100 my-1" />
                <button type="button" @click="pick('__new__')"
                  class="w-full flex items-center justify-between px-3 py-1.5 text-left text-xs transition-colors"
                  :class="partnerPick === '__new__' ? 'text-brand-600 bg-brand-50/50 font-medium' : 'text-brand-600 hover:bg-surface-50'">
                  <span>＋ 新建公司</span>
                  <AppIcon v-if="partnerPick === '__new__'" name="check" size="12" class="shrink-0" />
                </button>
              </div>
            </div>
            <div v-if="partnerPick === '__new__'" class="flex gap-1.5 items-center">
              <input v-model="newCompany.name" class="input-field !h-7 !py-1 !text-[12px] flex-1 min-w-0" placeholder="公司名称" />
              <div class="flex gap-1 shrink-0">
                <button v-for="t in TAB_LABELS" :key="t[0]" type="button" @click="newCompany.type = t[0]"
                  class="px-2 h-7 text-[10px] rounded border transition-colors"
                  :class="newCompany.type === t[0] ? 'bg-brand-50 border-brand-300 text-brand-700' : 'border-surface-200 text-surface-500 hover:border-surface-300'">
                  {{ t[2] }}
                </button>
              </div>
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">姓名 *</label>
              <input v-model="form.name" class="input-field !h-7 !py-1 !text-[12px]" placeholder="联系人姓名" />
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">职位</label>
              <div class="flex flex-wrap gap-1 mb-2">
                <button v-for="p in presetPositions" :key="p" type="button" @click="form.position = form.position === p ? '' : p"
                  class="px-2 py-0.5 text-[10px] rounded border transition-colors"
                  :class="form.position === p ? 'bg-brand-50 border-brand-300 text-brand-700' : 'border-surface-200 text-surface-500 hover:border-surface-300'">
                  {{ p }}
                </button>
              </div>
              <input v-model="form.position" class="input-field !h-7 !py-1 !text-[12px]" placeholder="或输入自定义职位" />
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">电话</label>
              <input v-model="form.phone" class="input-field !h-7 !py-1 !text-[12px]" placeholder="联系电话" />
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">邮箱</label>
              <input v-model="form.email" class="input-field !h-7 !py-1 !text-[12px]" placeholder="电子邮箱" />
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
              <input v-model="form.remark" class="input-field !h-7 !py-1 !text-[12px]" placeholder="备注信息" />
            </div>
          </div>
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="emit('close')" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="save" :disabled="saving" class="py-1 px-3 text-[12px] text-white bg-brand-600 hover:bg-brand-700 rounded transition-colors disabled:opacity-50">{{ saving ? '创建中…' : '创建' }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
