<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import {
  contactsListChannels, contactsListVendors,
  channelCreate, channelUpdate, channelDelete,
  channelContactCreate, channelContactUpdate, channelContactDelete,
  vendorCreate, vendorUpdate, vendorDelete,
  vendorContactCreate, vendorContactUpdate, vendorContactDelete,
  customerList, customerCreate, customerUpdate, customerDelete,
  customerContactCreate, customerContactUpdate, customerContactDelete,
} from '@/api/db-client'

type TabType = 'channels' | 'vendors' | 'customers'
const activeTab = ref<TabType>('channels')
const searchQuery = ref('')

const channels = ref<any[]>([])
const vendors = ref<any[]>([])
const customers = ref<any[]>([])

async function loadChannels() { try { channels.value = await contactsListChannels() || [] } catch (e) { console.error(e) } }
async function loadVendors() { try { vendors.value = await contactsListVendors() || [] } catch (e) { console.error(e) } }
async function loadCustomers() { try { customers.value = await customerList() || [] } catch (e) { console.error(e) } }
async function loadAll() { await Promise.all([loadChannels(), loadVendors(), loadCustomers()]) }

const currentList = computed(() => {
  return activeTab.value === 'channels' ? channels.value
    : activeTab.value === 'vendors' ? vendors.value
    : customers.value
})

const filteredEntities = computed(() => {
  if (!searchQuery.value.trim()) return currentList.value
  const q = searchQuery.value.toLowerCase()
  return currentList.value.filter((e: any) =>
    (e.name || '').toLowerCase().includes(q) ||
    (e.company || '').toLowerCase().includes(q) ||
    (e.phone || '').toLowerCase().includes(q) ||
    (e.contact_person || '').toLowerCase().includes(q)
  )
})

const selectedId = ref<string|null>(null)
const selectedEntity = computed(() => filteredEntities.value.find((e: any) => e.id === selectedId.value) || null)
const contacts = computed(() => selectedEntity.value?.contacts || [])
const contactSearch = ref('')
const filteredContacts = computed(() => {
  if (!contactSearch.value.trim()) return contacts.value
  const q = contactSearch.value.toLowerCase()
  return contacts.value.filter((ct: any) =>
    (ct.name || '').toLowerCase().includes(q) ||
    (ct.phone || '').toLowerCase().includes(q) ||
    (ct.email || '').toLowerCase().includes(q) ||
    (ct.remark || '').toLowerCase().includes(q)
  )
})

function selectEntity(id: string) { selectedId.value = id; contactSearch.value = '' }

// ─── Inline edit ───
const editingField = ref('') // "entityId:field" or "contactId:field"
const editValue = ref('')

function startEdit(entityId: string, field: string, current: string) {
  editingField.value = entityId + ':' + field
  editValue.value = current || ''
}
function cancelEdit() { editingField.value = ''; editValue.value = '' }
function isEditing(entityId: string, field: string) { return editingField.value === entityId + ':' + field }

async function saveEntityField(entity: any, field: string) {
  entity[field] = editValue.value
  try {
    if (activeTab.value === 'channels') await channelUpdate({ id: entity.id, [field]: editValue.value })
    else if (activeTab.value === 'vendors') await vendorUpdate({ id: entity.id, [field]: editValue.value })
    else if (activeTab.value === 'customers') await customerUpdate({ id: entity.id, [field]: editValue.value })
  } catch (e) { console.error(e) }
  cancelEdit()
}

// ─── Add contact ───
const showContactModal = ref(false)
const showEditContactModal = ref(false)
const editingContact = ref<any>(null)
const newContact = ref({ name: '', phone: '', email: '', position: '', remark: '' })

const presetPositions = ['总经理', '副总', '经理', '总监', '主管', '工程师', '技术负责人', '采购', '财务', '行政', '其他']

function startAddContact() {
  showContactModal.value = true
  newContact.value = { name: '', phone: '', email: '', position: '', remark: '' }
}
function cancelAddContact() { showContactModal.value = false; newContact.value = { name: '', phone: '', email: '', position: '', remark: '' } }

async function saveNewContact() {
  if (!newContact.value.name.trim()) { toast.error('请输入联系人姓名'); return }
  if (!selectedEntity.value) return
  try {
    if (activeTab.value === 'channels') await channelContactCreate({ channelId: selectedEntity.value.id, ...newContact.value })
    else if (activeTab.value === 'vendors') await vendorContactCreate({ vendorId: selectedEntity.value.id, ...newContact.value })
    else if (activeTab.value === 'customers') await customerContactCreate({ customerId: selectedEntity.value.id, ...newContact.value })
    toast.success('联系人已添加')
    if (activeTab.value === 'channels') await loadChannels()
    else if (activeTab.value === 'vendors') await loadVendors()
    else if (activeTab.value === 'customers') await loadCustomers()
    cancelAddContact()
  } catch (e) { toast.error('添加失败') }
}

function openEditContact(ct: any) {
  editingContact.value = { ...ct }
  showEditContactModal.value = true
}
function cancelEditContact() { showEditContactModal.value = false; editingContact.value = null }

async function saveEditContact() {
  if (!editingContact.value) return
  try {
    if (activeTab.value === 'channels') await channelContactUpdate(editingContact.value)
    else if (activeTab.value === 'vendors') await vendorContactUpdate(editingContact.value)
    else if (activeTab.value === 'customers') await customerContactUpdate(editingContact.value)
    toast.success('联系人已更新')
    if (activeTab.value === 'channels') await loadChannels()
    else if (activeTab.value === 'vendors') await loadVendors()
    else if (activeTab.value === 'customers') await loadCustomers()
    cancelEditContact()
  } catch (e) { toast.error('更新失败') }
}

// ─── Delete ───
async function deleteEntity(item: any) {
  if (!(await dangerConfirm({ title: '删除确认', message: `确定删除「${item.name}」？将同时删除关联的所有联系人。`, confirmText: '删除' }))) return
  try {
    if (activeTab.value === 'channels') { await channelDelete(item.id); await loadChannels() }
    else if (activeTab.value === 'vendors') { await vendorDelete(item.id); await loadVendors() }
    else if (activeTab.value === 'customers') { await customerDelete(item.id); await loadCustomers() }
    if (selectedId.value === item.id) selectedId.value = null
    toast.success('已删除')
  } catch (e) { toast.error('删除失败') }
}

async function deleteContact(ct: any) {
  if (!(await dangerConfirm({ title: '删除联系人', message: `确定删除联系人「${ct.name}」？`, confirmText: '删除' }))) return
  try {
    if (activeTab.value === 'channels') { await channelContactDelete(ct.id); await loadChannels() }
    else if (activeTab.value === 'vendors') { await vendorContactDelete(ct.id); await loadVendors() }
    else if (activeTab.value === 'customers') { await customerContactDelete(ct.id); await loadCustomers() }
    toast.success('已删除')
  } catch (e) { toast.error('删除失败') }
}

// ─── Create entity ───
const showCreateModal = ref(false)
const newEntity = ref({ name: '', phone: '', company: '', remark: '', contact_person: '', contact_phone: '' })

function openCreate() {
  newEntity.value = { name: '', phone: '', company: '', remark: '', contact_person: '', contact_phone: '' }
  showCreateModal.value = true
}

async function createEntity() {
  if (!newEntity.value.name.trim()) { toast.error('请输入名称'); return }
  try {
    if (activeTab.value === 'channels') { await channelCreate(newEntity.value); await loadChannels() }
    else if (activeTab.value === 'vendors') { await vendorCreate(newEntity.value); await loadVendors() }
    else if (activeTab.value === 'customers') { await customerCreate(newEntity.value); await loadCustomers() }
    showCreateModal.value = false
    toast.success('已创建')
  } catch (e) { toast.error('创建失败') }
}

const tabs = [
  { key: 'channels', label: '渠道伙伴', icon: 'building' },
  { key: 'vendors', label: '供应商', icon: 'server' },
  { key: 'customers', label: '客户', icon: 'user' },
]

const tabColors: Record<string, string> = {
  channels: 'bg-blue-50 text-blue-500',
  vendors: 'bg-amber-50 text-amber-500',
  customers: 'bg-green-50 text-green-500',
}

onMounted(loadAll)
</script>

<template>
  <div class="h-full flex flex-col bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden">

    <!-- Top: Tab selector -->
    <div class="flex items-center border-b border-surface-200 shrink-0 bg-surface-50">
      <nav class="flex gap-0 -mb-px">
        <button v-for="tab in tabs" :key="tab.key" @click="activeTab = tab.key as TabType; searchQuery = ''; selectedId = null"
          class="tab flex items-center gap-1.5" :class="activeTab === tab.key ? 'tab-active' : ''">
          <AppIcon :name="tab.icon" class="h-4 w-4" />{{ tab.label }}
        </button>
      </nav>
    </div>

    <!-- Bottom: two columns -->
    <div class="flex-1 flex overflow-hidden">

      <!-- Left: Company list -->
      <div class="w-[260px] border-r border-surface-200 flex flex-col shrink-0">
        <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
          <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">公司</span>
          <button @click="openCreate" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="新建">
            <AppIcon name="plus" size="12" stroke="2.5" />
          </button>
        </div>
        <div class="flex items-center gap-1.5 px-3 py-2 border-b border-surface-100 shrink-0">
          <div class="flex items-center gap-1.5 flex-1 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
            <AppIcon name="search" size="12" class="text-surface-400"/>
            <input v-model="searchQuery" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索名称..."/>
          </div>
        </div>
        <div class="flex-1 overflow-y-auto">
          <div v-if="filteredEntities.length === 0" class="text-center py-8">
            <p class="text-[11px] text-surface-400">暂无数据</p>
          </div>
          <div v-for="entity in filteredEntities" :key="entity.id" @click="selectEntity(entity.id)"
            class="flex items-center gap-2.5 px-3 py-2.5 border-b border-surface-50 cursor-pointer hover:bg-surface-50 transition-colors"
            :class="selectedId === entity.id ? 'bg-brand-50/50 border-l-2 border-l-brand-500' : ''">
            <div class="h-7 w-7 rounded-lg flex items-center justify-center flex-shrink-0 text-[10px] font-bold" :class="tabColors[activeTab]">
              {{ (entity.name || '?')[0] }}
            </div>
            <div class="flex-1 min-w-0">
              <p class="text-[11px] font-medium text-surface-800 truncate">{{ entity.name || '未命名' }}</p>
              <p class="text-[10px] text-surface-400 truncate">{{ entity.company || entity.contact_person || entity.phone || '' }}</p>
            </div>
            <span v-if="entity.contacts?.length" class="text-[9px] text-surface-400 bg-surface-100 px-1.5 py-0.5 rounded-full shrink-0">{{ entity.contacts.length }}</span>
          </div>
        </div>
      </div>

      <!-- Right: Contact list / Customer detail -->
      <div class="flex-1 flex flex-col overflow-hidden min-w-0">
        <!-- No selection -->
        <div v-if="!selectedEntity" class="flex-1 flex items-center justify-center">
          <div class="text-center">
            <AppIcon :name="tabs.find(t => t.key === activeTab)?.icon || 'user'" class="w-12 h-12 text-surface-200 mx-auto mb-3" :stroke="1" />
            <p class="text-[12px] text-surface-400">选择一项查看详情</p>
          </div>
        </div>

        <!-- Entity detail -->
        <template v-if="selectedEntity">
          <!-- Header -->
          <div class="flex items-center justify-between px-4 h-10 border-b border-surface-100 shrink-0">
            <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">
              联系人
            </span>
            <div class="flex items-center gap-1.5">
              <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
                <AppIcon name="search" size="12" class="text-surface-400"/>
                <input v-model="contactSearch" type="text" class="w-[120px] bg-transparent text-[11px] outline-none placeholder-surface-300" placeholder="搜索联系人..."/>
              </div>
              <button @click="deleteEntity(selectedEntity)" class="flex items-center gap-1 px-2 py-1 text-[10px] rounded border border-red-200 text-red-500 hover:bg-red-50 transition-colors">
                <AppIcon name="trash" size="10" /> 删除
              </button>
              <button @click="startAddContact" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded bg-brand-600 text-white hover:bg-brand-700 transition-colors">
                <AppIcon name="plus" size="10" stroke="2.5" /> 添加联系人
              </button>
            </div>
          </div>

          <!-- Content -->
          <div class="flex-1 overflow-y-auto p-4 space-y-4">
            <!-- Entity info -->
            <div class="bg-surface-50 rounded-lg p-3 space-y-2">
              <div class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">名称</span>
                <template v-if="isEditing(selectedEntity.id, 'name')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs w-48" @keyup.enter="saveEntityField(selectedEntity, 'name')" @blur="saveEntityField(selectedEntity, 'name')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700 font-medium">{{ selectedEntity.name }}</span>
                  <button @click="startEdit(selectedEntity.id, 'name', selectedEntity.name)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div v-if="activeTab !== 'customers'" class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">电话</span>
                <template v-if="isEditing(selectedEntity.id, 'phone')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs w-40" @keyup.enter="saveEntityField(selectedEntity, 'phone')" @blur="saveEntityField(selectedEntity, 'phone')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700">{{ selectedEntity.phone || '—' }}</span>
                  <button @click="startEdit(selectedEntity.id, 'phone', selectedEntity.phone)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div v-if="activeTab !== 'customers'" class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">地址</span>
                <template v-if="isEditing(selectedEntity.id, 'company')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs w-40" @keyup.enter="saveEntityField(selectedEntity, 'company')" @blur="saveEntityField(selectedEntity, 'company')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700">{{ selectedEntity.company || '—' }}</span>
                  <button @click="startEdit(selectedEntity.id, 'company', selectedEntity.company)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div v-if="activeTab === 'customers'" class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">联系人</span>
                <template v-if="isEditing(selectedEntity.id, 'contact_person')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs w-40" @keyup.enter="saveEntityField(selectedEntity, 'contact_person')" @blur="saveEntityField(selectedEntity, 'contact_person')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700">{{ selectedEntity.contact_person || '—' }}</span>
                  <button @click="startEdit(selectedEntity.id, 'contact_person', selectedEntity.contact_person)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div v-if="activeTab === 'customers'" class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">电话</span>
                <template v-if="isEditing(selectedEntity.id, 'contact_phone')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs w-40" @keyup.enter="saveEntityField(selectedEntity, 'contact_phone')" @blur="saveEntityField(selectedEntity, 'contact_phone')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700">{{ selectedEntity.contact_phone || '—' }}</span>
                  <button @click="startEdit(selectedEntity.id, 'contact_phone', selectedEntity.contact_phone)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">备注</span>
                <template v-if="isEditing(selectedEntity.id, 'remark')">
                  <input v-model="editValue" class="input-field !h-5 !text-xs flex-1" @keyup.enter="saveEntityField(selectedEntity, 'remark')" @blur="saveEntityField(selectedEntity, 'remark')" @keyup.escape="cancelEdit" />
                </template>
                <template v-else>
                  <span class="text-surface-700">{{ selectedEntity.remark || '—' }}</span>
                  <button @click="startEdit(selectedEntity.id, 'remark', selectedEntity.remark)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors"><AppIcon name="edit" class="w-2.5 h-2.5" /></button>
                </template>
              </div>
              <div v-if="selectedEntity.project_names" class="flex items-center gap-2 text-[11px]">
                <span class="text-surface-400 w-12 shrink-0">项目</span>
                <span class="text-surface-700">{{ selectedEntity.project_names }}</span>
              </div>
            </div>

            <!-- New contact form (channels/vendors) - removed, using modal instead -->

            <!-- Contacts table -->
            <table v-if="filteredContacts.length > 0" class="w-full text-[11px]">
              <thead>
                <tr class="text-left text-surface-400 border-b border-surface-200">
                  <th class="py-2 font-medium w-[18%]">姓名</th>
                  <th class="py-2 font-medium w-[12%]">职位</th>
                  <th class="py-2 font-medium w-[16%]">电话</th>
                  <th class="py-2 font-medium w-[18%]">邮箱</th>
                  <th class="py-2 font-medium w-[16%]">备注</th>
                  <th class="py-2 font-medium w-[10%]"></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="ct in filteredContacts" :key="ct.id" class="border-b border-surface-50 group/ct hover:bg-surface-50">
                  <td class="py-2 pr-2"><span class="font-medium text-surface-700 truncate">{{ ct.name || '—' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-surface-500 truncate">{{ ct.position || '—' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-surface-500 truncate">{{ ct.phone || '—' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-surface-500 truncate">{{ ct.email || '—' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-surface-400 truncate">{{ ct.remark || '—' }}</span></td>
                  <td class="py-2 text-right flex items-center justify-end gap-1">
                    <button @click="openEditContact(ct)" class="p-1 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/ct:opacity-100"><AppIcon name="edit" class="w-3 h-3" /></button>
                    <button @click="deleteContact(ct)" class="p-1 rounded text-gray-300 hover:text-red-500 transition-colors opacity-0 group-hover/ct:opacity-100"><AppIcon name="trash" class="w-3 h-3" /></button>
                  </td>
                </tr>
              </tbody>
            </table>
            <p v-if="filteredContacts.length === 0 && !showContactModal" class="text-xs text-slate-400 italic py-4 text-center">暂无联系人</p>
          </div>
        </template>
      </div>
    </div>

    <!-- Create Modal -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showCreateModal" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-sm mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">
              {{ activeTab === 'channels' ? '新建渠道' : activeTab === 'vendors' ? '新建供应商' : '新建客户' }}
            </h3>
            <div class="space-y-2.5">
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">名称 *</label>
                <input v-model="newEntity.name" class="input-field !h-7 !text-[12px]" placeholder="输入名称" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">电话</label>
                <input v-model="newEntity.phone" class="input-field !h-7 !text-[12px]" placeholder="联系电话" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">地址</label>
                <input v-model="newEntity.company" class="input-field !h-7 !text-[12px]" placeholder="公司地址" />
              </div>
            </div>
            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="showCreateModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="createEntity" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors">创建</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Add Contact Modal -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showContactModal" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-sm mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">添加联系人</h3>
            <div class="space-y-2.5">
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">姓名 *</label>
                <input v-model="newContact.name" class="input-field !h-7 !text-[12px]" placeholder="联系人姓名" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">职位</label>
                <div class="flex flex-wrap gap-1 mb-2">
                  <button v-for="p in presetPositions" :key="p" @click="newContact.position = newContact.position === p ? '' : p"
                    class="px-2 py-0.5 text-[10px] rounded border transition-colors"
                    :class="newContact.position === p ? 'bg-brand-50 border-brand-300 text-brand-700' : 'border-surface-200 text-surface-500 hover:border-surface-300'">
                    {{ p }}
                  </button>
                </div>
                <input v-model="newContact.position" class="input-field !h-7 !text-[12px]" placeholder="或输入自定义职位" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">电话</label>
                <input v-model="newContact.phone" class="input-field !h-7 !text-[12px]" placeholder="联系电话" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">邮箱</label>
                <input v-model="newContact.email" class="input-field !h-7 !text-[12px]" placeholder="电子邮箱" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
                <input v-model="newContact.remark" class="input-field !h-7 !text-[12px]" placeholder="备注信息" />
              </div>
            </div>
            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="cancelAddContact" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="saveNewContact" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors">创建</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Edit Contact Modal -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showEditContactModal && editingContact" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-sm mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">编辑联系人</h3>
            <div class="space-y-2.5">
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">姓名</label>
                <input v-model="editingContact.name" class="input-field !h-7 !text-[12px]" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">职位</label>
                <div class="flex flex-wrap gap-1 mb-2">
                  <button v-for="p in presetPositions" :key="p" @click="editingContact.position = editingContact.position === p ? '' : p"
                    class="px-2 py-0.5 text-[10px] rounded border transition-colors"
                    :class="editingContact.position === p ? 'bg-brand-50 border-brand-300 text-brand-700' : 'border-surface-200 text-surface-500 hover:border-surface-300'">
                    {{ p }}
                  </button>
                </div>
                <input v-model="editingContact.position" class="input-field !h-7 !text-[12px]" placeholder="或输入自定义职位" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">电话</label>
                <input v-model="editingContact.phone" class="input-field !h-7 !text-[12px]" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">邮箱</label>
                <input v-model="editingContact.email" class="input-field !h-7 !text-[12px]" />
              </div>
              <div>
                <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
                <input v-model="editingContact.remark" class="input-field !h-7 !text-[12px]" />
              </div>
            </div>
            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="cancelEditContact" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="saveEditContact" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors">保存</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
