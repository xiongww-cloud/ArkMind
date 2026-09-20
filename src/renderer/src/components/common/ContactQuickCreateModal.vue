<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue'
import { toast } from '@/composables/useToast'
import { contactsListChannels, contactsListVendors, customerList, createPartner, createPartnerContact } from '@/api/db-client'

type TabType = 'channels' | 'vendors' | 'customers'
interface Prefill { name?: string; position?: string; email?: string; phone?: string; partnerId?: string; partnerName?: string }
interface Resolved { value: string; label: string; tab: TabType; partnerId: string }

const props = defineProps<{ visible: boolean; prefill?: Prefill }>()
const emit = defineEmits<{ close: []; created: [payload: { partnerId: string; tab: TabType }] }>()

const form = ref({ name: '', phone: '', email: '', position: '', remark: '' })
const companyText = ref('')
const resolved = ref<Resolved | null>(null)
const saving = ref(false)

const channels = ref<any[]>([])
const vendors = ref<any[]>([])
const customers = ref<any[]>([])

const TYPE_META: [TabType, string, string][] = [['channels', '渠道', 'CHANNEL'], ['vendors', '供应商', 'VENDOR'], ['customers', '客户', 'CUSTOMER']]
const presetPositions = ['总经理', '副总', '经理', '总监', '主管', '工程师', '技术负责人', '采购', '财务', '行政', '其他']

async function loadPartners() {
  try {
    const [ch, vd, cs] = await Promise.all([contactsListChannels(), contactsListVendors(), customerList()])
    channels.value = ch || []; vendors.value = vd || []; customers.value = cs || []
  } catch (e) { console.error('[ContactQuickCreate] load partners failed:', e) }
}

const allCompanies = computed<Resolved[]>(() => {
  const out: Resolved[] = []
  const lists: [TabType, any[]][] = [['channels', channels.value], ['vendors', vendors.value], ['customers', customers.value]]
  for (const [tab, list] of lists) {
    for (const e of list as any[]) out.push({ value: `${tab}:${e.id}`, label: e.name, tab, partnerId: e.id })
  }
  return out
})

const suggestions = computed(() => {
  const q = companyText.value.trim().toLowerCase()
  const hit = q ? allCompanies.value.filter(o => o.label.toLowerCase().includes(q)) : allCompanies.value
  return hit.slice(0, 60)
})

function typeLabel(tab: TabType): string {
  return TYPE_META.find(t => t[0] === tab)?.[1] || tab
}

function matchPartner(pf: Prefill): Resolved | null {
  if (pf.partnerId) {
    const hit = allCompanies.value.find(o => o.partnerId === pf.partnerId)
    if (hit) return hit
  }
  if (pf.partnerName) {
    const pn = pf.partnerName.toLowerCase().trim()
    const found = allCompanies.value.find(o => {
      const en = (o.label || '').toLowerCase().trim()
      return !!en && (en === pn || (en.length >= 3 && pn.includes(en)) || (pn.length >= 3 && en.includes(pn)))
    })
    if (found) return found
  }
  return null
}

watch(() => props.visible, async (show) => {
  if (!show) { dropdownOpen.value = false; newCompanyVisible.value = false; return }
  await loadPartners()
  const pf = props.prefill || {}
  form.value = { name: pf.name || '', phone: pf.phone || '', email: pf.email || '', position: pf.position || '', remark: '' }
  const hit = matchPartner(pf)
  if (hit) { companyText.value = hit.label; resolved.value = hit }
  else { companyText.value = ''; resolved.value = null }
})

// ── 公司输入 + 建议下拉 ──
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

function onCompanyInput() {
  resolved.value = null
  dropdownOpen.value = true
  const t = companyText.value.trim().toLowerCase()
  const exact = allCompanies.value.find(o => (o.label || '').toLowerCase().trim() === t)
  if (exact) { resolved.value = exact; dropdownOpen.value = false }
}
function selectSuggestion(o: Resolved) {
  companyText.value = o.label
  resolved.value = o
  dropdownOpen.value = false
}

// ── 新建公司（嵌套小弹窗） ──
const newCompanyVisible = ref(false)
const newCompanyName = ref('')
const newCompanyType = ref<TabType>('customers')
const newCompanyBusy = ref(false)
function openNewCompany() {
  newCompanyName.value = companyText.value.trim()
  newCompanyType.value = 'customers'
  dropdownOpen.value = false
  newCompanyVisible.value = true
}
async function confirmCreateCompany() {
  const cn = newCompanyName.value.trim()
  if (!cn) { toast({ message: '请输入公司名称', type: 'error' }); return }
  newCompanyBusy.value = true
  try {
    const enumType = TYPE_META.find(t => t[0] === newCompanyType.value)?.[2] || 'CUSTOMER'
    const r: any = await createPartner({ name: cn, type: enumType })
    if (!r?.id) throw new Error('公司创建失败')
    resolved.value = { value: `${newCompanyType.value}:${r.id}`, label: cn, tab: newCompanyType.value, partnerId: r.id }
    companyText.value = cn
    newCompanyVisible.value = false
    toast({ message: `公司「${cn}」已创建，请点击"创建"完成联系人`, type: 'success' })
  } catch (e: any) {
    toast({ message: '公司创建失败: ' + (e?.message || e), type: 'error' })
  } finally {
    newCompanyBusy.value = false
  }
}

async function save() {
  if (!form.value.name.trim()) { toast({ message: '请输入联系人姓名', type: 'error' }); return }
  if (!resolved.value) {
    if (!companyText.value.trim()) { toast({ message: '请输入或选择归属公司', type: 'error' }); return }
    openNewCompany()
    toast({ message: '未找到匹配公司，请确认后新建', type: 'info' })
    return
  }
  saving.value = true
  try {
    const cr: any = await createPartnerContact({ partnerId: resolved.value.partnerId, ...form.value })
    if (!cr?.id && !cr?.success) throw new Error('联系人创建失败')
    toast({ message: `联系人「${form.value.name.trim()}」已创建（归属：${resolved.value.label}）`, type: 'success' })
    emit('created', { partnerId: resolved.value.partnerId, tab: resolved.value.tab })
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
              <div class="flex gap-1.5">
                <input v-model="companyText" @input="onCompanyInput" @focus="dropdownOpen = true"
                  class="input-field !h-7 !py-1 !text-[12px] flex-1 min-w-0" placeholder="输入公司名或从下拉选择" />
                <button type="button" @click="openNewCompany"
                  class="shrink-0 h-7 px-2.5 text-[11px] rounded-lg border border-surface-200 text-surface-600 hover:border-brand-300 hover:text-brand-600 transition-colors">新建公司</button>
              </div>
              <div v-if="dropdownOpen && suggestions.length" class="absolute top-full left-0 mt-1 w-full bg-white rounded-lg shadow-lg border border-surface-100 py-1 z-20 max-h-[220px] overflow-y-auto">
                <button v-for="o in suggestions" :key="o.value" type="button" @mousedown.prevent="selectSuggestion(o)"
                  class="w-full flex items-center justify-between px-3 py-1.5 text-left text-xs transition-colors"
                  :class="resolved?.value === o.value ? 'text-brand-600 bg-brand-50/50 font-medium' : 'text-surface-700 hover:bg-surface-50'">
                  <span class="truncate">{{ o.label }}</span>
                  <span class="text-[10px] text-surface-400 shrink-0 ml-2">{{ typeLabel(o.tab) }}</span>
                </button>
              </div>
              <p v-if="resolved" class="mt-1 text-[10px] text-brand-600">已匹配到现有公司</p>
              <p v-else-if="companyText.trim()" class="mt-1 text-[10px] text-amber-600">未匹配到现有公司，点击右侧"新建公司"创建</p>
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

    <!-- 新建公司（嵌套确认弹窗） -->
    <Transition name="confirm-fade">
      <div v-if="newCompanyVisible" class="fixed inset-0 z-[130] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="newCompanyVisible = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-[320px] mx-4 p-5 z-10">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">新建公司</h3>
          <div class="space-y-2.5">
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">公司名称 *</label>
              <input v-model="newCompanyName" class="input-field !h-7 !py-1 !text-[12px]" placeholder="公司全称" />
            </div>
            <div>
              <label class="text-[11px] text-surface-500 mb-1 block">公司类型</label>
              <div class="flex gap-1.5">
                <button v-for="t in TYPE_META" :key="t[0]" type="button" @click="newCompanyType = t[0]"
                  class="flex-1 h-7 text-[11px] rounded border transition-colors"
                  :class="newCompanyType === t[0] ? 'bg-brand-50 border-brand-300 text-brand-700' : 'border-surface-200 text-surface-500 hover:border-surface-300'">
                  {{ t[1] }}
                </button>
              </div>
            </div>
          </div>
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="newCompanyVisible = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="confirmCreateCompany" :disabled="newCompanyBusy" class="py-1 px-3 text-[12px] text-white bg-brand-600 hover:bg-brand-700 rounded transition-colors disabled:opacity-50">{{ newCompanyBusy ? '创建中…' : '创建公司' }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
