<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { toast } from '@/composables/useToast'
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
const selectedVendorContactId = ref('')
const vendorRemark = ref('')

const selectedVendor = computed(() => allVendors.value.find(v => v.id === selectedVendorId.value) || null)
const vendorContacts = computed(() => selectedVendor.value?.contacts || [])

async function confirmVendorSelect() {
  if (!selectedVendor.value) return
  const vendor = selectedVendor.value
  const vendorName = vendor.name
  const selectedContactId = selectedVendorContactId.value || null
  const remark = vendorRemark.value || null
  showVendorSelectModal.value = false
  selectedVendorId.value = ''
  selectedVendorContactId.value = ''
  vendorRemark.value = ''
  try {
    await partnerLinkProject(vendor.id, props.projectId, selectedContactId, remark)
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

const showVendorContactSelectModal = ref(false)
const vendorContactSelectVendorId = ref('')
const selectedVendorContactToAdd = ref('')
const availableVendorContacts = ref<any[]>([])

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
  if (!selectedVendorContactToAdd.value || !vendorContactSelectVendorId.value) return
  const contact = availableVendorContacts.value.find(c => c.id === selectedVendorContactToAdd.value)
  if (!contact) return
  const vendorId = vendorContactSelectVendorId.value
  showVendorContactSelectModal.value = false
  selectedVendorContactToAdd.value = ''
  try {
    await contactLinkProject(contact.id, props.projectId)
    const vd = vendors.value.find(v => v.id === vendorId)
    if (vd) {
      if (!Array.isArray(vd.contacts)) vd.contacts = []
      if (!vd.contacts.find(c => c.id === contact.id)) {
        vd.contacts.push({ id: contact.id, name: contact.name, phone: contact.phone, email: contact.email, position: contact.position })
      }
    }
    await projectStore.addProjectDynamic(props.projectId, `添加了供应商联系人「${contact.name}」`)
    toast({ message: `已添加联系人「${contact.name}」`, type: 'success' })
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
        <div class="flex-1 min-w-0 flex items-center gap-3">
          <div class="flex items-center gap-1 flex-shrink-0">
            <p class="text-xs font-medium text-surface-600 whitespace-nowrap">{{ vd.name || '未命名' }}</p>
          </div>
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
                  <th class="text-left font-medium py-2 w-[20%]">联系人</th>
                  <th class="text-left font-medium py-2 w-[18%]">电话</th>
                  <th class="text-left font-medium py-2 w-[18%]">职位</th>
                  <th class="text-left font-medium py-2">邮箱</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="ct in (vd.contacts || [])" :key="ct.id" class="border-b border-slate-100 hover:bg-white/60 transition-colors">
                  <td class="py-2 pr-2"><span class="text-xs text-slate-600">{{ ct.name || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.phone || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.position || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500">{{ ct.email || '-' }}</span></td>
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
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">选择供应商</h3>
          <div class="mb-4">
            <label class="text-[11px] text-surface-500 mb-1 block">供应商</label>
            <SelectWithSearch
              v-model="selectedVendorId"
              :options="allVendors.map(v => ({ id: v.id, name: v.name, address: v.address }))"
              placeholder="选择供应商..."
            />
          </div>
          <div v-if="selectedVendor && vendorContacts.length > 0" class="mb-4">
            <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
            <SelectWithSearch
              v-model="selectedVendorContactId"
              :options="vendorContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
              placeholder="选择联系人..."
            />
          </div>
          <div class="mb-4">
            <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
            <input v-model="vendorRemark" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
          </div>
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="showVendorSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="confirmVendorSelect" :disabled="!selectedVendor" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showVendorContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showVendorContactSelectModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">选择联系人</h3>
          <div v-if="availableVendorContacts.length > 0" class="mb-4">
            <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
            <SelectWithSearch
              v-model="selectedVendorContactToAdd"
              :options="availableVendorContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
              placeholder="选择联系人..."
            />
          </div>
          <div v-else class="text-[12px] text-surface-400 py-4 text-center">该供应商暂无联系人</div>
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="showVendorContactSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="confirmVendorContactSelect" :disabled="!selectedVendorContactToAdd" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
