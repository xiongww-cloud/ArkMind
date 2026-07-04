<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'
import { customerList } from '@/api/db-client'

const props = withDefaults(defineProps<{
  project: any
  isArchived?: boolean
}>(), { isArchived: false })

const emit = defineEmits<{
  (e: 'update-customer', data: { customer: string; customer_contact: string; customer_phone: string }): void
}>()

// 客户选择弹窗
const showCustomerModal = ref(false)
const allCustomers = ref<any[]>([])
const selectedCustomerId = ref('')
const selectedContactId = ref('')

async function loadCustomers() {
  try {
    allCustomers.value = await customerList() || []
  } catch (e) {
    console.error('加载客户列表失败:', e)
  }
}

function openCustomerModal() {
  loadCustomers()
  showCustomerModal.value = true
}

const selectedCustomer = computed(() => {
  return allCustomers.value.find(c => c.id === selectedCustomerId.value) || null
})

const customerContacts = computed(() => {
  return selectedCustomer.value?.contacts || []
})

function confirmCustomerSelect() {
  if (!selectedCustomer.value) return
  
  const customer = selectedCustomer.value
  const contact = customerContacts.value.find((c: any) => c.id === selectedContactId.value) || null
  
  emit('update-customer', {
    customer: customer.name || '',
    customer_contact: contact?.name || '',
    customer_phone: contact?.phone || '',
  })
  
  showCustomerModal.value = false
  selectedCustomerId.value = ''
  selectedContactId.value = ''
}
</script>

<template>
  <div class="mt-4 border-t border-surface-100 pt-4">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-xs font-semibold text-surface-500 uppercase tracking-wide">客户信息</h3>
      <button v-if="!isArchived" @click="openCustomerModal" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
        <AppIcon name="plus" size="10" stroke="2.5" /> 添加客户
      </button>
    </div>
    
    <div class="grid grid-cols-1 md:grid-cols-3 gap-y-4 gap-x-6">
      <!-- 客户名称 -->
      <div class="flex flex-col gap-0.5">
        <span class="text-xs text-surface-400">名称</span>
        <span class="text-sm" :class="project.customer ? 'text-surface-600' : 'text-surface-400 italic'">{{ project.customer || '暂未指定客户' }}</span>
      </div>
      <!-- 联系人 -->
      <div class="flex flex-col gap-0.5">
        <span class="text-xs text-surface-400">联系人</span>
        <span class="text-sm" :class="(project.customerContact || project.customer_contact) ? 'text-surface-600' : 'text-surface-400 italic'">{{ project.customerContact || project.customer_contact || '暂未指定联系人' }}</span>
      </div>
      <!-- 电话 -->
      <div class="flex flex-col gap-0.5">
        <span class="text-xs text-surface-400">电话</span>
        <span class="text-sm" :class="(project.customerPhone || project.customer_phone) ? 'text-surface-600' : 'text-surface-400 italic'">{{ project.customerPhone || project.customer_phone || '暂未指定电话' }}</span>
      </div>
    </div>

    <!-- 客户选择弹窗 -->
    <Teleport to="body">
      <Transition name="confirm-fade">
        <div v-if="showCustomerModal" class="fixed inset-0 z-[80] flex items-center justify-center">
          <div class="absolute inset-0 bg-black/20" @click="showCustomerModal = false" />
          <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
            <h3 class="text-sm font-semibold text-surface-800 mb-4">选择客户</h3>
            
            <!-- 客户选择 -->
            <div class="mb-4">
              <label class="text-[11px] text-surface-500 mb-1 block">客户</label>
              <SelectWithSearch
                v-model="selectedCustomerId"
                :options="allCustomers.map(c => ({ id: c.id, name: c.name, company: c.company }))"
                placeholder="选择客户..."
              />
            </div>

            <!-- 联系人选择 -->
            <div v-if="selectedCustomer && customerContacts.length > 0" class="mb-4">
              <label class="text-[11px] text-surface-500 mb-1 block">联系人</label>
              <SelectWithSearch
                v-model="selectedContactId"
                :options="customerContacts.map(c => ({ id: c.id, name: c.name, phone: c.phone }))"
                placeholder="选择联系人..."
              />
            </div>

            <!-- 操作按钮 -->
            <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
              <button @click="showCustomerModal = false" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
              <button @click="confirmCustomerSelect" :disabled="!selectedCustomer" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
