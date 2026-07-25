<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'
import { fetchPartners, createPartner, updatePartner, deletePartner, partnerLinkProject, partnerUnlinkProject, partnerUpdateRemark, createPartnerContact, updatePartnerContact, deletePartnerContact, contactLinkProject, contactsListAll } from '@/api/db-client'
import { useProjectStore } from '@/stores/project'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'

const props = withDefaults(defineProps<{
  project: any
  isArchived?: boolean
  projectId: string
}>(), { isArchived: false })

const emit = defineEmits<{
  (e: 'update'): void
}>()

const projectStore = useProjectStore()

// 客户管理状态
const customers = ref<any[]>([])
const expandedCustomerId = ref<string | null>(null)

// 客户选择弹窗
const showCustomerSelectModal = ref(false)
const allCustomers = ref<any[]>([])
const selectedCustomerId = ref('')
const selectedCustomerContactIds = ref<string[]>([])
const customerContactSearch = ref('')
const customerRemark = ref('')

// 备注编辑
const editingCustomerRemarkId = ref('')
const editingCustomerRemark = ref('')

// 联系人选择弹窗
const showContactSelectModal = ref(false)
const contactSelectCustomerId = ref('')
const selectedContactIds = ref<string[]>([])
const availableContacts = ref<any[]>([])
const contactSelectSearch = ref('')

const filteredAvailableContacts = computed(() => {
  const q = contactSelectSearch.value.toLowerCase().trim()
  if (!q) return availableContacts.value
  return availableContacts.value.filter((c: any) =>
    (c.name || '').toLowerCase().includes(q) || (c.phone || '').toLowerCase().includes(q) || (c.position || '').toLowerCase().includes(q)
  )
})

// 创建联系人弹窗
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
  if (!name || !selectedCustomerId.value) return
  try {
    const ct = await createPartnerContact({ partnerId: selectedCustomerId.value, name, phone: createContactPhone.value.trim(), email: createContactEmail.value.trim(), position: createContactPosition.value.trim() })
    await contactLinkProject(ct.id, props.projectId)
    const cust = customers.value.find(c => c.id === selectedCustomerId.value)
    if (cust) {
      if (!Array.isArray(cust.contacts)) cust.contacts = []
      cust.contacts.push({ id: ct.id, name: ct.name || name, phone: ct.phone || '', email: ct.email || '', position: ct.position || '' })
    }
    selectedCustomerContactIds.value = [ct.id]
    showCreateContactModal.value = false
    await projectStore.addProjectDynamic(props.projectId, `添加了联系人「${name}」`)
    toast({ message: `已创建联系人「${name}」`, type: 'success' })
  } catch (e) {
    console.warn('创建联系人失败', e)
    toast({ message: '创建联系人失败', type: 'error' })
  }
}

// 联系人编辑状态
const editingContactNameOf = ref('')
const contactEditNameValue = ref('')

const selectedCustomerForModal = computed(() => {
  return allCustomers.value.find(c => c.id === selectedCustomerId.value) || null
})

const customerContactsForModal = computed(() => {
  return selectedCustomerForModal.value?.contacts || []
})

const filteredCustomerContacts = computed(() => {
  const q = customerContactSearch.value.toLowerCase().trim()
  if (!q) return customerContactsForModal.value
  return customerContactsForModal.value.filter((c: any) =>
    (c.name || '').toLowerCase().includes(q) || (c.phone || '').toLowerCase().includes(q) || (c.position || '').toLowerCase().includes(q)
  )
})

// 加载客户列表
async function loadCustomers() {
  try { customers.value = await fetchPartners(props.projectId, 'CUSTOMER') || [] } catch { customers.value = [] }
}

// 打开客户选择弹窗
function openCustomerSelectModal() {
  loadAllCustomers()
  showCustomerSelectModal.value = true
}

async function loadAllCustomers() {
  try { allCustomers.value = await contactsListAll('CUSTOMER') || [] } catch { allCustomers.value = [] }
}

// 确认选择客户
async function confirmCustomerSelect() {
  if (!selectedCustomerForModal.value) return
  
  const customer = selectedCustomerForModal.value
  const customerName = customer.name
  const contactIds = [...selectedCustomerContactIds.value]
  const remark = customerRemark.value || null
  
  showCustomerSelectModal.value = false
  selectedCustomerId.value = ''
  selectedCustomerContactIds.value = ''
  customerContactSearch.value = ''
  customerRemark.value = ''
  
  try {
    if (contactIds.length > 0) {
      await partnerLinkProject(customer.id, props.projectId, contactIds[0], remark)
      for (const cid of contactIds.slice(1)) {
        await contactLinkProject(cid, props.projectId)
      }
    } else {
      await partnerLinkProject(customer.id, props.projectId, null, remark)
    }
    customers.value = await fetchPartners(props.projectId, 'CUSTOMER') || []
    
    await projectStore.addProjectDynamic(props.projectId, `添加了客户「${customerName}」`)
    toast({ message: `已添加客户「${customerName}」`, type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('添加客户失败', e)
    toast({ message: '添加客户失败', type: 'error' })
  }
}

// 删除客户
async function removeCustomer(customerId: string) {
  const ok = await dangerConfirm({
    title: '删除客户',
    message: '确定将此客户从项目中移除？',
    confirmText: '移除'
  })
  if (!ok) return
  try {
    await partnerUnlinkProject(customerId, props.projectId)
    customers.value = customers.value.filter(c => c.id !== customerId)
    await projectStore.addProjectDynamic(props.projectId, '移除了客户')
    toast({ message: '已移除客户', type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('删除客户失败', e)
    toast({ message: '删除客户失败', type: 'error' })
  }
}

// 展开/折叠客户卡片
function toggleCustomer(customerId: string) {
  expandedCustomerId.value = expandedCustomerId.value === customerId ? null : customerId
}

// 备注编辑
function startEditCustomerRemark(cd: any) {
  editingCustomerRemarkId.value = cd.id
  editingCustomerRemark.value = cd.project_remark || ''
}
function cancelEditCustomerRemark() { editingCustomerRemarkId.value = '' }
async function saveCustomerRemark(customerId: string) {
  const r = editingCustomerRemark.value.trim()
  const cd = customers.value.find(c => c.id === customerId)
  if (cd && r === (cd.project_remark || '')) { editingCustomerRemarkId.value = ''; return }
  editingCustomerRemarkId.value = ''
  try {
    await partnerUpdateRemark(customerId, props.projectId, r || null)
    if (cd) cd.project_remark = r
  } catch (e) { console.warn('更新客户备注失败', e) }
}

// ── 联系人管理 ──
async function startAddCustomerContact(customerId: string) {
  contactSelectCustomerId.value = customerId
  // 加载所有客户数据以获取联系人
  if (allCustomers.value.length === 0) {
    await loadAllCustomers()
  }
  // 从 allCustomers 中查找当前客户的联系人
  const cd = customers.value.find(c => c.id === customerId)
  const systemCustomer = allCustomers.value.find(sc => sc.name === cd?.name)
  availableContacts.value = systemCustomer?.contacts || []
  showContactSelectModal.value = true
}

async function confirmContactSelect() {
  if (selectedContactIds.value.length === 0 || !contactSelectCustomerId.value) return
  const contactIds = [...selectedContactIds.value]
  const customerId = contactSelectCustomerId.value
  showContactSelectModal.value = false
  selectedContactIds.value = []
  contactSelectSearch.value = ''
  try {
    for (const cid of contactIds) {
      await contactLinkProject(cid, props.projectId)
    }
    const cd = customers.value.find(c => c.id === customerId)
    if (cd) {
      if (!Array.isArray(cd.contacts)) cd.contacts = []
      for (const cid of contactIds) {
        const ct = availableContacts.value.find(c => c.id === cid)
        if (ct && !cd.contacts.find(c => c.id === ct.id)) {
          cd.contacts.push({ id: ct.id, name: ct.name, phone: ct.phone, email: ct.email, position: ct.position })
        }
      }
    }
    await projectStore.addProjectDynamic(props.projectId, `添加了客户联系人`)
    toast({ message: `已添加 ${contactIds.length} 个联系人`, type: 'success' })
  } catch (e) {
    console.warn('添加联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

async function removeCustomerContact(customerId: string, contactId: string) {
  const ok = await dangerConfirm({
    title: '删除联系人',
    message: '确定将此联系人从项目中移除？',
    confirmText: '移除'
  })
  if (!ok) return
  try {
    await contactUnlinkProject(contactId, props.projectId)
    const cd = customers.value.find(c => c.id === customerId)
    if (cd?.contacts) { cd.contacts = cd.contacts.filter(c => c.id !== contactId) }
    await projectStore.addProjectDynamic(props.projectId, '移除了一个联系人')
    toast({ message: '已移除联系人', type: 'success' })
  } catch (e) {
    console.warn('移除联系人失败', e)
  }
}

// 初始化加载
watch(() => props.projectId, () => { loadCustomers() }, { immediate: true })
</script>

<template>
  <div class="mt-4 border-t border-surface-100 pt-4">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">客户信息</h3>
      <button v-if="!isArchived" @click="openCustomerSelectModal" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
        <AppIcon name="plus" size="10" stroke="2.5" /> 添加客户
      </button>
    </div>
    
    <div v-if="customers.length === 0" class="text-sm text-slate-400 italic py-2">暂未添加客户</div>
    
    <div v-for="cd in customers" :key="cd.id" class="card !p-0 overflow-hidden mb-3">
      <!-- Card header -->
      <div @click="toggleCustomer(cd.id)" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/cd-card select-none">
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 bg-green-50 text-green-500">
          <AppIcon name="user" class="h-4 w-4" />
        </div>
        <div class="flex-1 min-w-0 flex items-center gap-2">
          <p class="text-xs font-medium text-surface-600 whitespace-nowrap flex-shrink-0">{{ cd.name || '未命名' }}</p>
          <template v-if="editingCustomerRemarkId === cd.id">
            <input v-model="editingCustomerRemark" @blur="saveCustomerRemark(cd.id)" @keyup.enter="saveCustomerRemark(cd.id)" @keyup.escape="cancelEditCustomerRemark" @click.stop
              class="text-[11px] text-slate-500 bg-transparent border-b border-slate-300 outline-none focus:border-blue-400 min-w-[80px] px-0.5" />
          </template>
          <template v-else>
            <span class="text-[11px] text-slate-400 whitespace-nowrap shrink-0">{{ cd.project_remark || '无备注' }}</span>
            <button v-if="!isArchived" @click.stop="startEditCustomerRemark(cd)" class="text-slate-300 hover:text-blue-500 shrink-0" title="编辑备注">
              <AppIcon name="edit" size="10" />
            </button>
          </template>
        </div>
        <div class="flex items-center gap-0.5 flex-shrink-0">
          <span class="text-[11px] text-slate-400">{{ (cd.contacts || []).length }} 个联系人</span>
          <button v-if="!isArchived" @click.stop="removeCustomer(cd.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" title="删除客户">
            <AppIcon name="trash" class="w-3 h-3" />
          </button>
          <AppIcon name="chevron-right" class="h-3.5 w-3.5 text-surface-300 transition-transform duration-200" :class="expandedCustomerId === cd.id ? 'rotate-90' : ''" />
        </div>
      </div>

      <!-- Collapsible contacts section -->
      <div v-show="expandedCustomerId === cd.id" class="border-t border-surface-100 bg-gray-50/50">
        <div class="p-4">
          <div v-if="cd.contacts && cd.contacts.length > 0">
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
                <tr v-for="ct in (cd.contacts || [])" :key="ct.id" class="border-b border-slate-100 group/ct-row hover:bg-white/60 transition-colors">
                  <td class="py-2 pr-2"><span class="text-xs text-slate-600 truncate block">{{ ct.name || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.phone || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.position || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.email || '-' }}</span></td>
                  <td class="py-2 text-center">
                    <button v-if="!isArchived" @click.stop="removeCustomerContact(cd.id, ct.id)" class="text-slate-300 hover:text-red-500 transition-colors opacity-0 group-hover/ct-row:opacity-100" title="删除联系人">
                      <AppIcon name="trash" size="12" />
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <p v-else class="text-xs text-slate-400 italic">暂无联系人</p>
          <button v-if="!isArchived" @click.stop="startAddCustomerContact(cd.id)" class="mt-3 btn-secondary">
            <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
          </button>
        </div>
      </div>
    </div>

    <!-- 客户选择弹窗 -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showCustomerSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" @click="showCustomerSelectModal = false" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 520px;">
            <!-- 固定头部 -->
            <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
              <h3 class="text-sm font-semibold text-surface-800 mb-3">选择客户</h3>
              <label class="text-[11px] text-surface-500 mb-1 block">客户</label>
              <SelectWithSearch
                v-model="selectedCustomerId"
                :options="allCustomers.map(c => ({ id: c.id, name: c.name }))"
                placeholder="选择客户..."
              />
              <div v-if="selectedCustomerForModal" class="mt-3">
                <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
                <input v-model="customerRemark" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
              </div>
            </div>
            <!-- 联系人区块 -->
            <div v-if="selectedCustomerForModal" class="flex-1 flex flex-col min-h-0 px-5 py-3">
              <div class="flex items-center justify-between mb-2">
                <span class="text-[11px] text-surface-500 font-medium">联系人</span>
                <button @click="openCreateContactModal" class="text-[10px] text-blue-600 hover:underline">+ 添加联系人</button>
              </div>
              <div v-if="customerContactsForModal.length > 0" class="flex-1 flex flex-col min-h-0">
                <input v-model="customerContactSearch" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400 mb-2 shrink-0" placeholder="搜索联系人..." />
                <div class="flex-1 overflow-y-auto border border-surface-200 rounded-lg">
                  <label v-for="ct in filteredCustomerContacts" :key="ct.id" class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-50 last:border-b-0">
                    <input type="checkbox" :value="ct.id" v-model="selectedCustomerContactIds" class="w-3 h-3 rounded border-slate-200 text-blue-500 focus:ring-blue-400" />
                    <span class="text-xs text-slate-600 flex-1 truncate">{{ ct.name || '-' }}</span>
                    <span class="text-[10px] text-slate-400">{{ ct.position || '' }}</span>
                    <span class="text-[10px] text-slate-400">{{ ct.phone || '' }}</span>
                    <span class="text-[10px] text-slate-400">{{ ct.email || '' }}</span>
                  </label>
                </div>
              </div>
              <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该客户暂无联系人</div>
            </div>
            <!-- 操作按钮 -->
            <div class="px-5 py-3 border-t border-surface-100 shrink-0 flex items-center justify-end gap-2">
              <button @click="showCustomerSelectModal = false" class="btn-secondary">取消</button>
              <button @click="confirmCustomerSelect" :disabled="!selectedCustomerForModal" class="btn-primary">确定{{ selectedCustomerContactIds.length > 0 ? ` (${selectedCustomerContactIds.length})` : '' }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- 联系人选择弹窗 -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" @click="showContactSelectModal = false" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 420px;">
            <!-- 固定头部 -->
            <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
              <h3 class="text-sm font-semibold text-surface-800 mb-3">选择联系人</h3>
              <div v-if="availableContacts.length > 0">
                <input v-model="contactSelectSearch" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="搜索联系人..." />
              </div>
            </div>
            <!-- 联系人列表区块 -->
            <div class="flex-1 min-h-0 px-5 py-3 flex flex-col">
              <div v-if="availableContacts.length > 0" class="flex-1 overflow-y-auto border border-surface-200 rounded-lg">
                <label v-for="ct in filteredAvailableContacts" :key="ct.id" class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-50 last:border-b-0">
                  <input type="checkbox" :value="ct.id" v-model="selectedContactIds" class="w-3 h-3 rounded border-slate-200 text-blue-500 focus:ring-blue-400" />
                  <span class="text-xs text-slate-600 flex-1 truncate">{{ ct.name || '-' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.position || '' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.phone || '' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.email || '' }}</span>
                </label>
              </div>
              <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该客户暂无联系人</div>
            </div>
            <!-- 底部按钮 -->
            <div class="px-5 py-3 border-t border-surface-100 shrink-0 flex items-center justify-end gap-2">
              <button @click="showContactSelectModal = false" class="btn-secondary">取消</button>
              <button @click="confirmContactSelect" :disabled="selectedContactIds.length === 0" class="btn-primary">确定{{ selectedContactIds.length > 0 ? ` (${selectedContactIds.length})` : '' }}</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>

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
