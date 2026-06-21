<script setup lang="ts">
import { ref, computed } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { useProjectStore } from '@/stores/project'

const props = withDefaults(defineProps<{
  project: any
  isArchived?: boolean
}>(), { isArchived: false })

const projectStore = useProjectStore()
const projectId = computed(() => props.project?.id)

const editingCustomerName = ref(false)
const editingCustomerContact = ref(false)
const editingCustomerPhone = ref(false)
const customerInfoName = ref('')
const customerInfoContact = ref('')
const customerInfoPhone = ref('')

function startEditCustomerField(field: string) {
  if (field === 'name') {
    customerInfoName.value = props.project?.customer || ''
    editingCustomerName.value = true
  } else if (field === 'contact') {
    customerInfoContact.value = props.project?.customerContact || props.project?.customer_contact || ''
    editingCustomerContact.value = true
  } else if (field === 'phone') {
    customerInfoPhone.value = props.project?.customerPhone || props.project?.customer_phone || ''
    editingCustomerPhone.value = true
  }
}

async function saveCustomerField(field: string) {
  const updates: Record<string, string> = {}
  let detailMsg = ''
  if (field === 'name') {
    const v = customerInfoName.value.trim()
    const old = props.project?.customer || ''
    if (v === old) { editingCustomerName.value = false; return }
    updates.customer = v || ''
    if (props.project) props.project.customer = v || ''
    editingCustomerName.value = false
    detailMsg = v ? `客户名称从「${old}」改为「${v}」` : '清除了客户名称'
  } else if (field === 'contact') {
    const v = customerInfoContact.value.trim()
    const old = props.project?.customerContact || props.project?.customer_contact || ''
    if (v === old) { editingCustomerContact.value = false; return }
    updates.customer_contact = v || ''
    if (props.project) props.project.customerContact = v || ''
    editingCustomerContact.value = false
    detailMsg = v ? `客户联系人从「${old}」改为「${v}」` : '清除了客户联系人'
  } else if (field === 'phone') {
    const v = customerInfoPhone.value.trim()
    const old = props.project?.customerPhone || props.project?.customer_phone || ''
    if (v === old) { editingCustomerPhone.value = false; return }
    updates.customer_phone = v || ''
    if (props.project) props.project.customerPhone = v || ''
    editingCustomerPhone.value = false
    detailMsg = v ? `客户电话从「${old}」改为「${v}」` : '清除了客户电话'
  }
  if (Object.keys(updates).length === 0) return
  projectStore.updateProjectInList(projectId.value, updates)
  try {
    await projectStore.updateProject(projectId.value, updates)
    await projectStore.addProjectDynamic(projectId.value, detailMsg)
  } catch (e) { console.warn('保存客户信息失败', e) }
}
</script>

<template>
  <div class="mt-4 border-t border-surface-100 pt-4">
    <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide mb-4">客户信息</h3>
    <div class="grid grid-cols-1 md:grid-cols-3 gap-y-4 gap-x-6">
      <!-- 客户名称 -->
      <div class="flex flex-col gap-0.5 group/cust-name">
        <span class="text-xs text-slate-400">名称</span>
        <div class="flex items-center gap-1">
          <template v-if="editingCustomerName">
            <input v-model="customerInfoName" type="text" class="input-field !h-6 !text-xs w-full" placeholder="客户名称"
              @blur="saveCustomerField('name')" @keyup.enter="saveCustomerField('name')" @keyup.escape="editingCustomerName = false" />
          </template>
          <template v-else>
            <span class="text-sm" :class="project.customer ? 'text-slate-600' : 'text-slate-400 italic'">{{ project.customer || '暂未指定客户' }}</span>
            <button v-if="!isArchived" @click="startEditCustomerField('name')" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/cust-name:opacity-100 flex-shrink-0" title="编辑名称">
              <AppIcon name="edit" class="w-3 h-3" />
            </button>
          </template>
        </div>
      </div>
      <!-- 联系人 -->
      <div class="flex flex-col gap-0.5 group/cust-contact">
        <span class="text-xs text-slate-400">联系人</span>
        <div class="flex items-center gap-1">
          <template v-if="editingCustomerContact">
            <input v-model="customerInfoContact" type="text" class="input-field !h-6 !text-xs w-full" placeholder="联系人"
              @blur="saveCustomerField('contact')" @keyup.enter="saveCustomerField('contact')" @keyup.escape="editingCustomerContact = false" />
          </template>
          <template v-else>
            <span class="text-sm" :class="(project.customerContact || project.customer_contact) ? 'text-slate-600' : 'text-slate-400 italic'">{{ project.customerContact || project.customer_contact || '暂未指定联系人' }}</span>
            <button v-if="!isArchived" @click="startEditCustomerField('contact')" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/cust-contact:opacity-100 flex-shrink-0" title="编辑联系人">
              <AppIcon name="edit" class="w-3 h-3" />
            </button>
          </template>
        </div>
      </div>
      <!-- 电话 -->
      <div class="flex flex-col gap-0.5 group/cust-phone">
        <span class="text-xs text-slate-400">电话</span>
        <div class="flex items-center gap-1">
          <template v-if="editingCustomerPhone">
            <input v-model="customerInfoPhone" type="text" class="input-field !h-6 !text-xs w-full" placeholder="电话"
              @blur="saveCustomerField('phone')" @keyup.enter="saveCustomerField('phone')" @keyup.escape="editingCustomerPhone = false" />
          </template>
          <template v-else>
            <span class="text-sm" :class="(project.customerPhone || project.customer_phone) ? 'text-slate-600' : 'text-slate-400 italic'">{{ project.customerPhone || project.customer_phone || '暂未指定电话' }}</span>
            <button v-if="!isArchived" @click="startEditCustomerField('phone')" class="p-0.5 rounded text-gray-300 hover:text-[#007aff] transition-colors opacity-0 group-hover/cust-phone:opacity-100 flex-shrink-0" title="编辑电话">
              <AppIcon name="edit" class="w-3 h-3" />
            </button>
          </template>
        </div>
      </div>
    </div>
  </div>
</template>
