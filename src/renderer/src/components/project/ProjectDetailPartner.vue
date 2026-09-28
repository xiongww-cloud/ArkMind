<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { contactsListChannels, contactsListVendors, customerList } from '@/api/db-client'

const props = withDefaults(defineProps<{
  type: 'customer' | 'channel' | 'vendor'
  entity: any
  isArchived?: boolean
}>(), { isArchived: false })

const emit = defineEmits<{
  (e: 'update', id: string, data: Record<string, string>): void
  (e: 'delete', id: string): void
  (e: 'contact-create', id: string, data: Record<string, string>): void
  (e: 'contact-update', id: string, data: Record<string, string>): void
  (e: 'contact-delete', entityId: string, contactId: string): void
  (e: 'entity-select', entityId: string): void
  (e: 'contact-select', entityId: string, contactIds: string[]): void
}>()

// 通讯录数据
const allChannels = ref<any[]>([])
const allVendors = ref<any[]>([])
const allCustomers = ref<any[]>([])
const selectedContactIds = ref<string[]>([])

async function loadContactsData() {
  try {
    if (props.type === 'channel') {
      allChannels.value = await contactsListChannels() || []
    } else if (props.type === 'vendor') {
      allVendors.value = await contactsListVendors() || []
    } else if (props.type === 'customer') {
      allCustomers.value = await customerList() || []
    }
  } catch (e) {
    console.error('加载通讯录数据失败:', e)
  }
}

onMounted(loadContactsData)

const config = computed(() => {
  const map: Record<string, { label: string; icon: string; iconBg: string; iconColor: string }> = {
    customer: { label: '客户', icon: 'user', iconBg: 'bg-emerald-50', iconColor: 'text-emerald-500' },
    channel: { label: '渠道', icon: 'building', iconBg: 'bg-blue-50', iconColor: 'text-blue-500' },
    vendor: { label: '供应商', icon: 'building', iconBg: 'bg-amber-50', iconColor: 'text-amber-500' },
  }
  return map[props.type]
})

// 联系人选择
const entityContacts = computed(() => {
  if (props.type === 'channel') {
    const ch = allChannels.value.find(c => c.id === props.entity.id)
    return ch?.contacts || []
  } else if (props.type === 'vendor') {
    const vd = allVendors.value.find(v => v.id === props.entity.id)
    return vd?.contacts || []
  }
  return []
})

function onContactSelect(contactIds: string[]) {
  selectedContactIds.value = contactIds
  emit('contact-select', props.entity.id, contactIds)
}

// ─── Inline edit state ───
const expanded = ref(false)
const editingNameId = ref('')
const editingName = ref('')
const editingRemarkId = ref('')
const editingRemark = ref('')

function startEditName() {
  editingNameId.value = props.entity.id
  editingName.value = props.entity.name || ''
}
function cancelEditName() { editingNameId.value = '' }
async function saveName() {
  if (editingNameId.value !== props.entity.id) return
  const n = editingName.value.trim()
  editingNameId.value = ''
  if (n === (props.entity.name || '')) return
  if (!n) {
    const ok = await dangerConfirm({ title: `删除${config.value.label}`, message: `名称为空将删除此${config.value.label}，确定？`, confirmText: '删除' })
    if (!ok) return
    emit('delete', props.entity.id)
    return
  }
  emit('update', props.entity.id, { name: n })
}

function startEditRemark() {
  editingRemarkId.value = props.entity.id
  editingRemark.value = props.entity.remark || ''
}
function cancelEditRemark() { editingRemarkId.value = '' }
async function saveRemark() {
  if (editingRemarkId.value !== props.entity.id) return
  editingRemarkId.value = ''
  emit('update', props.entity.id, { remark: editingRemark.value.trim() })
}

async function removeEntity() {
  const ok = await dangerConfirm({ title: `删除${config.value.label}`, message: `确定删除此${config.value.label}？将同时删除关联的所有联系人。`, confirmText: '删除' })
  if (!ok) return
  emit('delete', props.entity.id)
}

// ─── Contacts ───
const editingContactNameOf = ref('')
const editingContactPhoneOf = ref('')
const editingContactRemarkOf = ref('')
const contactEditName = ref('')
const contactEditPhone = ref('')
const contactEditRemark = ref('')

const contacts = computed(() => props.entity.contacts || [])
const isNewContactMode = computed(() => editingContactRemarkOf.value.startsWith('new-'))

function startAddContact() {
  editingContactRemarkOf.value = 'new-' + props.entity.id
  contactEditName.value = ''
  contactEditPhone.value = ''
  contactEditRemark.value = ''
}

function startEditContactName(ct: any) { editingContactNameOf.value = ct.id; contactEditName.value = ct.name || '' }
function cancelEditContactName() { editingContactNameOf.value = '' }
async function saveContactName(ctId: string) {
  if (editingContactNameOf.value !== ctId) return
  editingContactNameOf.value = ''
  emit('contact-update', props.entity.id, { id: ctId, name: contactEditName.value.trim() })
}

function startEditContactPhone(ct: any) { editingContactPhoneOf.value = ct.id; contactEditPhone.value = ct.phone || '' }
function cancelEditContactPhone() { editingContactPhoneOf.value = '' }
async function saveContactPhone(ctId: string) {
  if (editingContactPhoneOf.value !== ctId) return
  editingContactPhoneOf.value = ''
  emit('contact-update', props.entity.id, { id: ctId, phone: contactEditPhone.value.trim() })
}

function startEditContactRemark(ct: any) { editingContactRemarkOf.value = ct.id; contactEditRemark.value = ct.remark || '' }
function cancelEditContactRemark() { editingContactRemarkOf.value = '' }
async function saveContactRemark(ctId: string) {
  if (editingContactRemarkOf.value !== ctId) return
  editingContactRemarkOf.value = ''
  emit('contact-update', props.entity.id, { id: ctId, remark: contactEditRemark.value.trim() })
}

function onNewContactFocusOut(event: FocusEvent) {
  const related = event.relatedTarget as HTMLElement | null
  if (related?.closest?.('.new-contact-row')) return
  saveNewContact()
}

async function saveNewContact() {
  if (!isNewContactMode.value) return
  const n = contactEditName.value.trim()
  if (!n) { editingContactRemarkOf.value = ''; return }
  editingContactRemarkOf.value = ''
  emit('contact-create', props.entity.id, {
    name: n,
    phone: contactEditPhone.value.trim(),
    remark: contactEditRemark.value.trim(),
  })
}

async function removeContact(ctId: string) {
  const ok = await dangerConfirm({ title: '删除联系人', message: '确定删除此联系人？', confirmText: '删除' })
  if (!ok) return
  emit('contact-delete', props.entity.id, ctId)
}
</script>

<template>
  <div class="card !p-0 overflow-hidden mb-3">
    <!-- Card header -->
    <div @click="expanded = !expanded" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/card select-none">
      <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0" :class="[config.iconBg, config.iconColor]">
        <AppIcon :name="config.icon" class="h-4 w-4" />
      </div>
      <div class="flex-1 min-w-0 flex items-center gap-3">
        <!-- Name - 使用 SelectWithSearch -->
        <div class="flex-1 min-w-0">
          <SelectWithSearch
            v-if="!isArchived"
            :modelValue="entity.id"
            :options="type === 'channel' ? allChannels : type === 'vendor' ? allVendors : allCustomers"
            placeholder="选择..."
            @update:modelValue="(v) => emit('entity-select', v)"
          />
          <span v-else class="text-xs font-medium text-surface-600 whitespace-nowrap">{{ entity.name || '未命名' }}</span>
        </div>
        <!-- Remark -->
        <div class="flex items-center gap-1 group/rm flex-shrink-0">
          <template v-if="editingRemarkId === entity.id">
            <input v-model="editingRemark" type="text" class="input-field !h-6 !text-xs w-32" placeholder="备注"
              @keyup.enter.stop="saveRemark" @blur="saveRemark" @keyup.escape.stop="cancelEditRemark" @click.stop />
          </template>
          <template v-else>
            <span class="text-xs truncate" :class="entity.remark ? 'text-surface-500' : 'text-surface-400 italic'">{{ entity.remark || '无备注' }}</span>
            <button v-if="!isArchived" @click.stop="startEditRemark" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/rm:opacity-100 flex-shrink-0" v-tooltip="'编辑备注'">
              <AppIcon name="edit" class="w-2.5 h-2.5" />
            </button>
          </template>
        </div>
      </div>
      <div class="flex items-center gap-0.5 flex-shrink-0">
        <span class="text-[11px] text-slate-400">{{ contacts.length }} 个联系人</span>
        <button v-if="!isArchived" @click.stop="removeEntity" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" v-tooltip="`删除${config.label}`">
          <AppIcon name="trash" class="w-3 h-3" />
        </button>
        <AppIcon name="chevron-right" class="h-3.5 w-3.5 text-surface-300 transition-transform duration-200" :class="expanded ? 'rotate-90' : ''" />
      </div>
    </div>

    <!-- Collapsible contacts section -->
    <div v-show="expanded" class="border-t border-surface-100 bg-gray-50/50">
      <div class="p-4">
        <!-- 联系人选择器 -->
        <div v-if="!isArchived && entityContacts.length > 0" class="mb-3">
          <label class="text-[10px] text-surface-400 mb-1 block">从通讯录选择联系人</label>
          <SelectWithSearch
            :modelValue="selectedContactIds"
            :options="entityContacts"
            placeholder="选择联系人..."
            :multiple="true"
            @update:modelValue="onContactSelect"
          />
        </div>

        <div v-if="contacts.length > 0 || isNewContactMode">
          <table class="w-full text-xs">
            <thead>
              <tr class="text-slate-400 border-b border-slate-200">
                <th class="text-left font-medium py-2 w-[25%]">联系人</th>
                <th class="text-left font-medium py-2 w-[22%]">电话</th>
                <th class="text-left font-medium py-2 w-[25%]">备注</th>
                <th class="w-10"></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="ct in contacts" :key="ct.id" class="border-b border-slate-100 group/ct-row hover:bg-white/60 transition-colors">
                <!-- Name -->
                <td class="py-2 pr-2">
                  <div class="flex items-center gap-0.5 group/ct-nm">
                    <template v-if="editingContactNameOf === ct.id">
                      <input v-model="contactEditName" type="text" class="input-field !h-5 !text-xs w-full" placeholder="姓名"
                        @keyup.enter="saveContactName(ct.id)" @blur="saveContactName(ct.id)" @keyup.escape="cancelEditContactName" />
                    </template>
                    <template v-else>
                      <span class="text-xs truncate" :class="ct.name ? 'text-slate-600' : 'text-slate-400 italic'">{{ ct.name || '暂未指定' }}</span>
                      <button v-if="!isArchived" @click.stop="startEditContactName(ct)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/ct-nm:opacity-100 flex-shrink-0" v-tooltip="'编辑姓名'">
                        <AppIcon name="edit" class="w-2.5 h-2.5" />
                      </button>
                    </template>
                  </div>
                </td>
                <!-- Phone -->
                <td class="py-2 pr-2">
                  <div class="flex items-center gap-0.5 group/ct-ph">
                    <template v-if="editingContactPhoneOf === ct.id">
                      <input v-model="contactEditPhone" type="text" class="input-field !h-5 !text-xs w-full" placeholder="电话"
                        @keyup.enter="saveContactPhone(ct.id)" @blur="saveContactPhone(ct.id)" @keyup.escape="cancelEditContactPhone" />
                    </template>
                    <template v-else>
                      <span class="text-xs text-slate-500 truncate">{{ ct.phone || '无' }}</span>
                      <button v-if="!isArchived" @click.stop="startEditContactPhone(ct)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/ct-ph:opacity-100 flex-shrink-0" v-tooltip="'编辑电话'">
                        <AppIcon name="edit" class="w-2.5 h-2.5" />
                      </button>
                    </template>
                  </div>
                </td>
                <!-- Remark -->
                <td class="py-2 pr-2">
                  <div class="flex items-center gap-0.5 group/ct-rm">
                    <template v-if="editingContactRemarkOf === ct.id">
                      <input v-model="contactEditRemark" type="text" class="input-field !h-5 !text-xs w-full" placeholder="备注"
                        @keyup.enter="saveContactRemark(ct.id)" @blur="saveContactRemark(ct.id)" @keyup.escape="cancelEditContactRemark" />
                    </template>
                    <template v-else>
                      <span class="text-xs whitespace-normal break-words" :class="ct.remark ? 'text-surface-500' : 'text-surface-400 italic'">{{ ct.remark || '无备注' }}</span>
                      <button v-if="!isArchived" @click.stop="startEditContactRemark(ct)" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/ct-rm:opacity-100 flex-shrink-0" v-tooltip="'编辑备注'">
                        <AppIcon name="edit" class="w-2.5 h-2.5" />
                      </button>
                    </template>
                  </div>
                </td>
                <!-- Delete -->
                <td class="py-2 text-right">
                  <button v-if="!isArchived" @click.stop="removeContact(ct.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500 opacity-0 group-hover/ct-row:opacity-100 flex-shrink-0" v-tooltip="'删除联系人'">
                    <AppIcon name="trash" class="w-3 h-3" />
                  </button>
                </td>
              </tr>
              <!-- New contact form row -->
              <tr v-if="isNewContactMode" class="border-b border-blue-100 bg-blue-50/30 new-contact-row" @focusout="onNewContactFocusOut">
                <td class="py-2 pr-2">
                  <input v-model="contactEditName" type="text" class="input-field !h-5 !text-xs w-full" placeholder="姓名"
                    @keyup.enter="saveNewContact" @keyup.escape="editingContactRemarkOf = ''" />
                </td>
                <td class="py-2 pr-2">
                  <input v-model="contactEditPhone" type="text" class="input-field !h-5 !text-xs w-full" placeholder="电话"
                    @keyup.enter="saveNewContact" @keyup.escape="editingContactRemarkOf = ''" />
                </td>
                <td class="py-2 pr-2">
                  <input v-model="contactEditRemark" type="text" class="input-field !h-5 !text-xs w-full" placeholder="备注"
                    @keyup.enter="saveNewContact" @keyup.escape="editingContactRemarkOf = ''" />
                </td>
                <td></td>
              </tr>
            </tbody>
          </table>
        </div>
        <!-- Add contact button -->
        <button v-if="!isArchived && !isNewContactMode" @click="startAddContact" class="mt-3 btn-secondary">
          <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
        </button>
        <p v-if="contacts.length === 0 && !isNewContactMode" class="text-xs text-slate-400 italic mt-2">暂无联系人</p>
      </div>
    </div>
  </div>
</template>
