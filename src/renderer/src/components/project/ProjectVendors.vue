<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import {
  fetchPartners, updatePartner, deletePartner, partnerLinkProject, partnerUnlinkProject,
  partnerUpdateRemark, contactsListAll, contactLinkProject, contactUnlinkProject,
  createPartnerContact, updatePartnerContact,
} from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'

const props = defineProps<{ project: any; projectId: string }>()
const emit = defineEmits<{ update: [] }>()

const projectStore = useProjectStore()

const vendors = ref<any[]>([])
const vendorEditNameId = ref('')
const vendorEditRemarkId = ref('')
const vendorEditName = ref('')
const vendorEditRemark = ref('')
const expandedVendorId = ref<string | null>(null)

const showVendorSelectModal = ref(false)
const allVendors = ref<any[]>([])
const selectedVendorId = ref('')
const selectedVendorContactIds = ref<string[]>([])
const vendorRemark = ref('')
const vendorContactSearch = ref('')

const selectedVendor = computed(() => allVendors.value.find(v => v.id === selectedVendorId.value) || null)
const vendorContacts = computed(() => selectedVendor.value?.contacts || [])
const filteredVendorContacts = computed(() => {
  const q = vendorContactSearch.value.toLowerCase().trim()
  if (!q) return vendorContacts.value
  return vendorContacts.value.filter((c: any) =>
    (c.name || '').toLowerCase().includes(q) || (c.phone || '').toLowerCase().includes(q) || (c.position || '').toLowerCase().includes(q)
  )
})

async function confirmVendorSelect() {
  if (!selectedVendor.value) return
  const vendor = selectedVendor.value
  const vendorName = vendor.name
  const contactIds = [...selectedVendorContactIds.value]
  const remark = vendorRemark.value || null
  showVendorSelectModal.value = false
  selectedVendorId.value = ''
  selectedVendorContactIds.value = []
  vendorRemark.value = ''
  vendorContactSearch.value = ''
  try {
    if (contactIds.length > 0) {
      await partnerLinkProject(vendor.id, props.projectId, contactIds[0], remark)
      for (const cid of contactIds.slice(1)) {
        await contactLinkProject(cid, props.projectId)
      }
    } else {
      await partnerLinkProject(vendor.id, props.projectId, null, remark)
    }
    vendors.value = await fetchPartners(props.projectId, 'VENDOR') || []
    await projectStore.addProjectDynamic(props.projectId, `添加了供应商「${vendorName}」`)
    toast({ message: `已添加供应商「${vendorName}」`, type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('添加供应商失败', e)
    toast({ message: '添加供应商失败', type: 'error' })
  }
}

function toggleVendor(vendorId: string) {
  expandedVendorId.value = expandedVendorId.value === vendorId ? null : vendorId
}

async function loadVendors() {
  try { vendors.value = await fetchPartners(props.projectId, 'VENDOR') } catch { vendors.value = [] }
}

function startEditVendorName(vd: any) {
  vendorEditNameId.value = vd.id
  vendorEditName.value = vd.name || ''
}
async function cancelEditVendorName() {
  const vdId = vendorEditNameId.value
  const vd = vendors.value.find(v => v.id === vdId)
  vendorEditNameId.value = ''
  if (vd && !vd.name) {
    try {
      await deletePartner(vdId)
      vendors.value = vendors.value.filter(v => v.id !== vdId)
    } catch (e) { console.warn('取消添加供应商', e) }
  }
}
async function saveVendorName(vendorId: string) {
  if (vendorEditNameId.value !== vendorId) return
  const n = vendorEditName.value.trim()
  const vd = vendors.value.find(v => v.id === vendorId)
  if (vd && n === (vd.name || '')) { vendorEditNameId.value = ''; return }
  vendorEditNameId.value = ''
  if (!n && vd && !vd.name) {
    try {
      await deletePartner(vendorId)
      vendors.value = vendors.value.filter(v => v.id !== vendorId)
    } catch (e) { console.warn('取消添加供应商', e) }
    return
  }
  try {
    const oldName = vd?.name || '未命名'
    await updatePartner({ id: vendorId, name: n || '' })
    if (vd) vd.name = n
    await projectStore.addProjectDynamic(props.projectId, `供应商「${oldName}」更名为「${n || '未命名'}」`)
    emit('update')
  } catch (e) { console.warn('更新供应商名称失败', e) }
}

function startEditVendorRemark(vd: any) {
  vendorEditRemarkId.value = vd.id
  vendorEditRemark.value = vd.project_remark || vd.remark || ''
}
function cancelEditVendorRemark() { vendorEditRemarkId.value = '' }
async function saveVendorRemark(vendorId: string) {
  const r = vendorEditRemark.value.trim()
  const vd = vendors.value.find(v => v.id === vendorId)
  if (vd && r === (vd.project_remark || vd.remark || '')) { vendorEditRemarkId.value = ''; return }
  vendorEditRemarkId.value = ''
  try {
    await partnerUpdateRemark(vendorId, props.projectId, r || null)
    if (vd) vd.project_remark = r
  } catch (e) { console.warn('更新供应商备注失败', e) }
}

async function addVendor() {
  showVendorSelectModal.value = true
  try {
    allVendors.value = await contactsListAll('VENDOR') || []
  } catch (e) { console.warn('加载供应商列表失败', e) }
}
async function removeVendor(vendorId: string) {
  const ok = await dangerConfirm({
    title: '删除供应商',
    message: '确定将此供应商从项目中移除？',
    confirmText: '移除'
  })
  if (!ok) return
  try {
    await partnerUnlinkProject(vendorId, props.projectId)
    vendors.value = vendors.value.filter(v => v.id !== vendorId)
    await projectStore.addProjectDynamic(props.projectId, '移除了一个供应商')
    toast({ message: '已移除供应商', type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('移除供应商失败', e)
    toast({ message: '移除失败', type: 'error' })
  }
}

const editingVendorContactNameOf = ref('')
const editingVendorContactPhoneOf = ref('')
const addingContactToVendorId = ref('')
const vendorContactEditName = ref('')
const vendorContactEditPhone = ref('')
const vendorContactEditEmail = ref('')
const vendorContactEditPosition = ref('')

const showCreateContactModal = ref(false)
const createContactName = ref('')
const createContactPhone = ref('')
const createContactEmail = ref('')
const createContactPosition = ref('')

function openCreateContactModal() {
  createContactName.value = ''
  createContactPhone.value = ''
  createContactEmail.value = ''
  createContactPosition.value = ''
  showCreateContactModal.value = true
}

async function confirmCreateContact() {
  const name = createContactName.value.trim()
  if (!name || !selectedVendorId.value) return
  try {
    const ct = await createPartnerContact({ partnerId: selectedVendorId.value, name, phone: createContactPhone.value.trim(), email: createContactEmail.value.trim(), position: createContactPosition.value.trim() })
    await contactLinkProject(ct.id, props.projectId)
    const vd = vendors.value.find(v => v.id === selectedVendorId.value)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      vd.contacts.push({ id: ct.id, name: ct.name || name, phone: ct.phone || '', email: ct.email || '', position: ct.position || '' })
    }
    selectedVendorContactIds.value = [ct.id]
    showCreateContactModal.value = false
    await projectStore.addProjectDynamic(props.projectId, `添加了联系人「${name}」`)
    toast({ message: `已创建联系人「${name}」`, type: 'success' })
  } catch (e) {
    console.warn('创建联系人失败', e)
    toast({ message: '创建联系人失败', type: 'error' })
  }
}

const showVendorContactSelectModal = ref(false)
const vendorContactSelectVendorId = ref('')
const selectedVendorContactToAddIds = ref<string[]>([])
const availableVendorContacts = ref<any[]>([])
const vendorContactSelectSearch = ref('')

const filteredAvailableVendorContacts = computed(() => {
  const q = vendorContactSelectSearch.value.toLowerCase().trim()
  if (!q) return availableVendorContacts.value
  return availableVendorContacts.value.filter((c: any) =>
    (c.name || '').toLowerCase().includes(q) || (c.phone || '').toLowerCase().includes(q) || (c.position || '').toLowerCase().includes(q)
  )
})

async function startAddVendorContact(vendorId: string) {
  vendorContactSelectVendorId.value = vendorId
  if (allVendors.value.length === 0) {
    try { allVendors.value = await contactsListAll('VENDOR') || [] } catch {}
  }
  const vd = vendors.value.find(v => v.id === vendorId)
  const systemVendor = allVendors.value.find(sv => sv.name === vd?.name)
  availableVendorContacts.value = systemVendor?.contacts || []
  showVendorContactSelectModal.value = true
}

async function confirmVendorContactSelect() {
  if (selectedVendorContactToAddIds.value.length === 0 || !vendorContactSelectVendorId.value) return
  const contactIds = [...selectedVendorContactToAddIds.value]
  const vendorId = vendorContactSelectVendorId.value
  showVendorContactSelectModal.value = false
  selectedVendorContactToAddIds.value = []
  vendorContactSelectSearch.value = ''
  try {
    for (const cid of contactIds) {
      await contactLinkProject(cid, props.projectId)
    }
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      for (const cid of contactIds) {
        const ct = availableVendorContacts.value.find(c => c.id === cid)
        if (ct && !vd.contacts.find(c => c.id === ct.id)) {
          vd.contacts.push({ id: ct.id, name: ct.name, phone: ct.phone, email: ct.email, position: ct.position })
        }
      }
    }
    await projectStore.addProjectDynamic(props.projectId, `添加了供应商联系人`)
    toast({ message: `已添加 ${contactIds.length} 个联系人`, type: 'success' })
  } catch (e) {
    console.warn('添加供应商联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

function startEditVendorContactName(ct: any) {
  editingVendorContactNameOf.value = ct.id
  vendorContactEditName.value = ct.name || ''
}
function cancelEditVendorContactName() { editingVendorContactNameOf.value = '' }
async function saveVendorContactName(vendorId: string, contactId: string) {
  if (editingVendorContactNameOf.value !== contactId) return
  const n = vendorContactEditName.value.trim()
  editingVendorContactNameOf.value = ''
  if (!n) return
  try {
    await updatePartnerContact({ id: contactId, name: n })
    const vd = vendors.value.find(v => v.id === vendorId)
    const ct = vd?.contacts?.find((c: any) => c.id === contactId)
    const oldName = ct?.name || ''
    if (ct) ct.name = n
    await projectStore.addProjectDynamic(props.projectId, `供应商联系人姓名从「${oldName}」改为「${n}」`)
  } catch (e) { console.warn('更新供应商联系人姓名失败', e) }
}

function startEditVendorContactPhone(ct: any) {
  editingVendorContactPhoneOf.value = ct.id
  vendorContactEditPhone.value = ct.phone || ''
}
function cancelEditVendorContactPhone() { editingVendorContactPhoneOf.value = '' }
async function saveVendorContactPhone(vendorId: string, contactId: string) {
  if (editingVendorContactPhoneOf.value !== contactId) return
  const p = vendorContactEditPhone.value.trim()
  editingVendorContactPhoneOf.value = ''
  try {
    await updatePartnerContact({ id: contactId, phone: p || '' })
    const vd = vendors.value.find(v => v.id === vendorId)
    const ct = vd?.contacts?.find((c: any) => c.id === contactId)
    const op = ct?.phone || ''
    if (ct) ct.phone = p
    const cname = ct?.name || '未命名'
    await projectStore.addProjectDynamic(props.projectId, `供应商联系人「${cname}」电话从「${op}」改为「${p}」`)
  } catch (e) { console.warn('更新供应商联系人电话失败', e) }
}

async function saveNewVendorContact(vendorId: string) {
  if (!addingContactToVendorId.value || addingContactToVendorId.value !== 'new-' + vendorId) return
  const n = vendorContactEditName.value.trim()
  const p = vendorContactEditPhone.value.trim()
  const e = vendorContactEditEmail.value.trim()
  const pos = vendorContactEditPosition.value.trim()
  addingContactToVendorId.value = ''
  if (!n && !p && !e && !pos) return
  try {
    const ct = await createPartnerContact({
      partnerId: vendorId,
      name: n, phone: p || '', email: e || '', position: pos || ''
    })
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      vd.contacts.push({ id: ct.id, name: ct.name || n, phone: ct.phone || p, email: ct.email || e, position: ct.position || pos })
    }
    await contactLinkProject(ct.id, props.projectId)
    await projectStore.addProjectDynamic(props.projectId, `添加了供应商联系人「${n}」`)
  } catch (e) { console.warn('添加供应商联系人失败', e) }
}

async function removeVendorContact(vendorId: string, contactId: string) {
  try {
    await contactUnlinkProject(contactId, props.projectId)
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd?.contacts) { vd.contacts = vd.contacts.filter((c: any) => c.id !== contactId) }
    await projectStore.addProjectDynamic(props.projectId, '移除了一个供应商联系人')
    toast({ message: '已移除联系人', type: 'success' })
  } catch (e) { console.warn('移除供应商联系人失败', e) }
}

watch(() => props.projectId, () => { loadVendors() }, { immediate: true })
</script>

<template>
  <div class="mt-4 border-t border-surface-100 pt-4">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">供应商信息</h3>
      <button v-if="!project.isArchived" @click="addVendor" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
        <AppIcon name="plus" size="10" stroke="2.5" /> 添加供应商
      </button>
    </div>
    <div v-if="vendors.length === 0" class="text-sm text-slate-400 italic py-2">暂未添加供应商</div>
    <div v-for="vd in vendors" :key="vd.id" class="card !p-0 overflow-hidden mb-3">
      <div @click="toggleVendor(vd.id)" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/vd-card select-none">
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 bg-amber-50 text-amber-500">
          <AppIcon name="building" class="h-4 w-4" />
        </div>
        <div class="flex-1 min-w-0 flex items-center gap-2">
          <p class="text-xs font-medium text-surface-600 whitespace-nowrap flex-shrink-0">{{ vd.name || '未命名' }}</p>
          <template v-if="vendorEditRemarkId === vd.id">
            <input v-model="vendorEditRemark" @blur="saveVendorRemark(vd.id)" @keyup.enter="saveVendorRemark(vd.id)" @keyup.escape="cancelEditVendorRemark" @click.stop
              class="text-[11px] text-slate-500 bg-transparent border-b border-slate-300 outline-none focus:border-blue-400 min-w-[80px] px-0.5" />
          </template>
          <template v-else>
            <span class="text-[11px] text-slate-400 whitespace-nowrap shrink-0">{{ vd.project_remark || '无备注' }}</span>
            <button v-if="!project.isArchived" @click.stop="startEditVendorRemark(vd)" class="text-slate-300 hover:text-blue-500 shrink-0" title="编辑备注">
              <AppIcon name="edit" size="10" />
            </button>
          </template>
        </div>
        <div class="flex items-center gap-0.5 flex-shrink-0">
          <span class="text-[11px] text-slate-400">{{ (vd.contacts || []).length }} 个联系人</span>
          <button v-if="!project.isArchived" @click.stop="removeVendor(vd.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" title="删除供应商">
            <AppIcon name="trash" class="w-3 h-3" />
          </button>
          <AppIcon name="chevron-right" class="h-4 w-4 text-surface-300 transition-transform duration-200" :class="expandedVendorId === vd.id ? 'rotate-90' : ''" />
        </div>
      </div>

      <div v-show="expandedVendorId === vd.id" class="border-t border-surface-100 bg-gray-50/50">
        <div class="p-4">
          <div v-if="vd.contacts && vd.contacts.length > 0">
            <table class="w-full text-xs">
              <thead>
                <tr class="text-slate-400 border-b border-slate-200">
                  <th class="text-left font-medium py-2 w-[22%]">联系人</th>
                  <th class="text-left font-medium py-2 w-[22%]">电话</th>
                  <th class="text-left font-medium py-2 w-[18%]">职位</th>
                  <th class="text-left font-medium py-2 w-[22%]">邮箱</th>
                  <th class="w-[15%]"></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="ct in (vd.contacts || [])" :key="ct.id" class="border-b border-slate-100 group/ct-row hover:bg-white/60 transition-colors">
                  <td class="py-2 pr-2"><span class="text-xs text-slate-600 truncate block">{{ ct.name || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.phone || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.position || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.email || '-' }}</span></td>
                  <td class="py-2 text-center"><button v-if="!project.isArchived" @click.stop="removeVendorContact(vd.id, ct.id)" class="text-slate-300 hover:text-red-500 transition-colors opacity-0 group-hover/ct-row:opacity-100" title="删除联系人"><AppIcon name="trash" size="12"/></button></td>
                </tr>
              </tbody>
            </table>
          </div>
          <p v-else class="text-xs text-slate-400 italic">暂无联系人</p>
          <button v-if="!project.isArchived" @click="startAddVendorContact(vd.id)" class="mt-3 btn-secondary">
            <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
          </button>
        </div>
      </div>
    </div>
  </div>

  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showVendorSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showVendorSelectModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 520px;">
          <!-- 固定头部 -->
          <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <h3 class="text-sm font-semibold text-surface-800 mb-3">选择供应商</h3>
            <label class="text-[11px] text-surface-500 mb-1 block">供应商</label>
            <SelectWithSearch
              v-model="selectedVendorId"
              :options="allVendors.map(v => ({ id: v.id, name: v.name, address: v.address }))"
              placeholder="选择供应商..."
            />
          </div>
          <!-- 联系人区块 -->
          <div v-if="selectedVendor" class="flex-1 flex flex-col min-h-0 px-5 py-3">
            <div class="flex items-center justify-between mb-2">
              <span class="text-[11px] text-surface-500 font-medium">联系人</span>
              <button @click="openCreateContactModal" class="text-[10px] text-blue-600 hover:underline">+ 添加联系人</button>
            </div>
            <div v-if="vendorContacts.length > 0" class="flex-1 flex flex-col min-h-0">
              <input v-model="vendorContactSearch" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400 mb-2 shrink-0" placeholder="搜索联系人..." />
              <div class="flex-1 overflow-y-auto border border-surface-200 rounded-lg">
                <label v-for="ct in filteredVendorContacts" :key="ct.id" class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-50 last:border-b-0">
                  <input type="checkbox" :value="ct.id" v-model="selectedVendorContactIds" class="w-3 h-3 rounded border-slate-200 text-blue-500 focus:ring-blue-400" />
                  <span class="text-xs text-slate-600 flex-1 truncate">{{ ct.name || '-' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.position || '' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.phone || '' }}</span>
                </label>
              </div>
            </div>
            <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该供应商暂无联系人</div>
          </div>
          <!-- 备注 + 按钮 -->
          <div class="px-5 py-3 border-t border-surface-100 shrink-0">
            <div class="mb-3">
              <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
              <input v-model="vendorRemark" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
            </div>
            <div class="flex items-center justify-end gap-2">
              <button @click="showVendorSelectModal = false" class="btn-secondary">取消</button>
              <button @click="confirmVendorSelect" :disabled="!selectedVendor" class="btn-primary">确定{{ selectedVendorContactIds.length > 0 ? ` (${selectedVendorContactIds.length})` : '' }}</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showVendorContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showVendorContactSelectModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 420px;">
          <!-- 固定头部 -->
          <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <h3 class="text-sm font-semibold text-surface-800 mb-3">选择联系人</h3>
            <div v-if="availableVendorContacts.length > 0">
              <input v-model="vendorContactSelectSearch" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="搜索联系人..." />
            </div>
          </div>
          <!-- 联系人列表区块 -->
          <div class="flex-1 min-h-0 px-5 py-3 flex flex-col">
            <div v-if="availableVendorContacts.length > 0" class="flex-1 overflow-y-auto border border-surface-200 rounded-lg">
              <label v-for="ct in filteredAvailableVendorContacts" :key="ct.id" class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-50 last:border-b-0">
                <input type="checkbox" :value="ct.id" v-model="selectedVendorContactToAddIds" class="w-3 h-3 rounded border-slate-200 text-blue-500 focus:ring-blue-400" />
                <span class="text-xs text-slate-600 flex-1 truncate">{{ ct.name || '-' }}</span>
                <span class="text-[10px] text-slate-400">{{ ct.position || '' }}</span>
                <span class="text-[10px] text-slate-400">{{ ct.phone || '' }}</span>
                <span class="text-[10px] text-slate-400">{{ ct.email || '' }}</span>
              </label>
            </div>
            <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该供应商暂无联系人</div>
          </div>
          <!-- 底部按钮 -->
          <div class="px-5 py-3 border-t border-surface-100 shrink-0 flex items-center justify-end gap-2">
            <button @click="showVendorContactSelectModal = false" class="btn-secondary">取消</button>
            <button @click="confirmVendorContactSelect" :disabled="selectedVendorContactToAddIds.length === 0" class="btn-primary">确定{{ selectedVendorContactToAddIds.length > 0 ? ` (${selectedVendorContactToAddIds.length})` : '' }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- 创建联系人弹窗 -->
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showCreateContactModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showCreateContactModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">创建联系人</h3>
          <div class="mb-3">
            <label class="text-[11px] text-surface-500 mb-1 block">姓名 <span class="text-red-400">*</span></label>
            <input v-model="createContactName" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="联系人姓名" />
          </div>
          <div class="mb-3">
            <label class="text-[11px] text-surface-500 mb-1 block">电话</label>
            <input v-model="createContactPhone" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="手机号码" />
          </div>
          <div class="mb-3">
            <label class="text-[11px] text-surface-500 mb-1 block">邮箱</label>
            <input v-model="createContactEmail" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="电子邮箱" />
          </div>
          <div class="mb-4">
            <label class="text-[11px] text-surface-500 mb-1 block">职位</label>
            <input v-model="createContactPosition" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="职位名称" />
          </div>
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="showCreateContactModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="confirmCreateContact" :disabled="!createContactName.trim()" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">创建</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
