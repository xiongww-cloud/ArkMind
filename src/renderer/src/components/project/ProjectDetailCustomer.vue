<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'
import { fetchPartners, createPartner, updatePartner, deletePartner, partnerLinkProject, partnerUnlinkProject, partnerUpdateRemark, createPartnerContact, updatePartnerContact, deletePartnerContact, contactLinkProject, contactsListAll } from '@/api/db-client'
import { useProjectStore } from '@/stores/project'
import { toast } from '@/composables/useToast'

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
const selectedCustomerContactId = ref('')

// 联系人选择弹窗
const showContactSelectModal = ref(false)
const contactSelectCustomerId = ref('')
const selectedContactId = ref('')
const availableContacts = ref<any[]>([])

// 联系人编辑状态
const editingContactNameOf = ref('')
const contactEditNameValue = ref('')

const selectedCustomerForModal = computed(() => {
  return allCustomers.value.find(c => c.id === selectedCustomerId.value) || null
})

const customerContactsForModal = computed(() => {
  return selectedCustomerForModal.value?.contacts || []
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
  const selectedContactId = selectedCustomerContactId.value || null
  
  showCustomerSelectModal.value = false
  selectedCustomerId.value = ''
  selectedCustomerContactId.value = ''
  
  try {
    await partnerLinkProject(customer.id, props.projectId, selectedContactId, null)
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
  if (!selectedContactId.value || !contactSelectCustomerId.value) return
  const contact = availableContacts.value.find(c => c.id === selectedContactId.value)
  if (!contact) return
  
  const customerId = contactSelectCustomerId.value
  showContactSelectModal.value = false
  selectedContactId.value = ''
  
  try {
    await contactLinkProject(contact.id, props.projectId)
    const cd = customers.value.find(c => c.id === customerId)
    if (cd) {
      if (!Array.isArray(cd.contacts)) cd.contacts = []
      if (!cd.contacts.find(c => c.id === contact.id)) {
        cd.contacts.push({ id: contact.id, name: contact.name, phone: contact.phone, email: contact.email, position: contact.position })
      }
    }
    await projectStore.addProjectDynamic(props.projectId, `添加了客户联系人「${contact.name}」`)
    toast({ message: `已添加联系人「${contact.name}」`, type: 'success' })
  } catch (e) {
    console.warn('添加联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

async function removeCustomerContact(customerId: string, contactId: string) {
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
        <div class="flex-1 min-w-0 flex items-center gap-3">
          <div class="flex items-center gap-1 flex-shrink-0">
            <p class="text-xs font-medium text-surface-600 whitespace-nowrap">{{ cd.name || '未命名' }}</p>
          </div>
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
                  <th class="text-left font-medium py-2 w-[20%]">联系人</th>
                  <th class="text-left font-medium py-2 w-[18%]">电话</th>
                  <th class="text-left font-medium py-2 w-[18%]">职位</th>
                  <th class="text-left font-medium py-2">邮箱</th>
                  <th class="w-10"></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="ct in (cd.contacts || [])" :key="ct.id" class="border-b border-slate-100 group/ct-row hover:bg-white/60 transition-colors">
                  <td class="py-2 pr-2"><span class="text-xs text-slate-600">{{ ct.name || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate">{{ ct.phone || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate">{{ ct.position || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate">{{ ct.email || '-' }}</span></td>
                  <td class="py-2 text-right">
                    <button v-if="!isArchived" @click.stop="removeCustomerContact(cd.id, ct.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500 opacity-0 group-hover/ct-row:opacity-100 flex-shrink-0" title="删除联系人">
                      <AppIcon name="trash" class="w-3 h-3" />
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
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">选择客户</h3>
            
            <!-- 客户选择 -->
            <div class="mb-4">
              <label class="text-[11px] text-surface-500 mb-1 block">客户</label>
              <SelectWithSearch
                v-model="selectedCustomerId"
                :options="allCustomers.map(c => ({ id: c.id, name: c.name }))"
                placeholder="选择客户..."
              />
            </div>

            <!-- 联系人选择 -->
            <div v-if="selectedCustomerForModal && customerContactsForModal.length > 0" class="mb-4">
              <label class="text-[11px] text-surface-500 mb-1 block">默认联系人</label>
              <SelectWithSearch
                v-model="selectedCustomerContactId"
                :options="customerContactsForModal.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
                placeholder="选择联系人..."
              />
            </div>

            <!-- 操作按钮 -->
            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="showCustomerSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="confirmCustomerSelect" :disabled="!selectedCustomerForModal" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
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
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">选择联系人</h3>
            
            <div class="mb-4">
              <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
              <SelectWithSearch
                v-model="selectedContactId"
                :options="availableContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
                placeholder="选择联系人..."
              />
            </div>

            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="showContactSelectModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="confirmContactSelect" :disabled="!selectedContactId" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
