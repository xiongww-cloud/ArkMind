<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import AppIcon from './AppIcon.vue'

interface Option {
  id: string
  name: string
  address?: string
  phone?: string
  remark?: string
  contacts?: any[]
  [key: string]: any
}

const props = withDefaults(defineProps<{
  show: boolean
  type: 'channel' | 'vendor' | 'customer'
  options: Option[]
  title?: string
}>(), {
  title: '选择'
})

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'select', data: { entity: Option; contacts: any[] }): void
}>()

const searchQuery = ref('')
const selectedEntity = ref<Option | null>(null)
const selectedContactIds = ref<string[]>([])

const filteredOptions = computed(() => {
  if (!searchQuery.value.trim()) return props.options
  const q = searchQuery.value.toLowerCase()
  return props.options.filter(opt =>
    opt.name.toLowerCase().includes(q) ||
    (opt.address && opt.address.toLowerCase().includes(q))
  )
})

const entityContacts = computed(() => {
  if (!selectedEntity.value) return []
  return selectedEntity.value.contacts || []
})

const filteredContacts = computed(() => {
  if (!searchQuery.value.trim()) return entityContacts.value
  const q = searchQuery.value.toLowerCase()
  return entityContacts.value.filter(ct =>
    ct.name?.toLowerCase().includes(q) ||
    ct.phone?.toLowerCase().includes(q)
  )
})

function selectEntity(opt: Option) {
  selectedEntity.value = opt
  selectedContactIds.value = []
}

function toggleContact(ct: any) {
  const idx = selectedContactIds.value.indexOf(ct.id)
  if (idx >= 0) {
    selectedContactIds.value.splice(idx, 1)
  } else {
    selectedContactIds.value.push(ct.id)
  }
}

function confirm() {
  if (!selectedEntity.value) return
  const selectedContacts = entityContacts.value.filter(ct => 
    selectedContactIds.value.includes(ct.id)
  )
  emit('select', { entity: selectedEntity.value, contacts: selectedContacts })
  close()
}

function close() {
  selectedEntity.value = null
  selectedContactIds.value = []
  searchQuery.value = ''
  emit('close')
}

function handleClickOutside(e: MouseEvent) {
  const modal = document.getElementById('partner-select-modal')
  if (modal && !modal.contains(e.target as Node)) {
    close()
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="show" id="partner-select-modal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="close" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 p-5 z-10">
          <h3 class="text-sm font-semibold text-surface-800 mb-4">{{ title }}</h3>
          
          <!-- Search -->
          <div class="mb-4">
            <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
              <AppIcon name="search" size="12" class="text-surface-400"/>
              <input v-model="searchQuery" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300 border border-surface-200 rounded px-2 focus:border-brand-400 transition-colors" placeholder="搜索公司名称..."/>
            </div>
          </div>

          <!-- Entity list -->
          <div v-if="!selectedEntity" class="max-h-[200px] overflow-y-auto border border-surface-200 rounded-lg">
            <div v-for="opt in filteredOptions" :key="opt.id" @click="selectEntity(opt)"
              class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-100 last:border-b-0">
              <div class="h-6 w-6 rounded flex items-center justify-center bg-blue-50 text-blue-500 text-[10px] font-bold">
                {{ opt.name?.[0] || '?' }}
              </div>
              <div class="flex-1 min-w-0">
                <p class="text-[11px] text-surface-700 truncate">{{ opt.name }}</p>
                <p v-if="opt.address" class="text-[10px] text-surface-400 truncate">{{ opt.address }}</p>
              </div>
              <span class="text-[9px] text-surface-400">{{ opt.contacts?.length || 0 }} 个联系人</span>
            </div>
            <div v-if="filteredOptions.length === 0" class="px-3 py-4 text-center text-[11px] text-surface-400">
              无匹配结果
            </div>
          </div>

          <!-- Contact selection -->
          <div v-else>
            <div class="flex items-center gap-2 mb-3">
              <span class="text-[11px] text-surface-500">已选：</span>
              <span class="text-[11px] font-medium text-surface-700">{{ selectedEntity.name }}</span>
              <button @click="selectedEntity = null" class="text-[10px] text-surface-400 hover:text-surface-600">更换</button>
            </div>

            <div class="mb-3">
              <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
                <AppIcon name="search" size="12" class="text-surface-400"/>
                <input v-model="searchQuery" type="text" class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300 border border-surface-200 rounded px-2 focus:border-brand-400 transition-colors" placeholder="搜索联系人..."/>
              </div>
            </div>

            <div class="max-h-[150px] overflow-y-auto border border-surface-200 rounded-lg">
              <div v-for="ct in filteredContacts" :key="ct.id" @click="toggleContact(ct)"
                class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-100 last:border-b-0">
                <div class="w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0"
                  :class="selectedContactIds.includes(ct.id) ? 'bg-brand-500 border-brand-500' : 'border-surface-300'">
                  <svg v-if="selectedContactIds.includes(ct.id)" class="w-2.5 h-2.5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                </div>
                <div class="flex-1 min-w-0">
                  <p class="text-[11px] text-surface-700 truncate">{{ ct.name || '未命名' }}</p>
                  <p v-if="ct.phone" class="text-[10px] text-surface-400 truncate">{{ ct.phone }}</p>
                </div>
              </div>
              <div v-if="filteredContacts.length === 0" class="px-3 py-4 text-center text-[11px] text-surface-400">
                无联系人
              </div>
            </div>
          </div>

          <!-- Actions -->
          <div class="flex items-center justify-end gap-2 mt-4 pt-3 border-t border-surface-100">
            <button @click="close" class="py-1 px-2.5 text-[12px] text-surface-400 hover:text-surface-600 hover:bg-surface-50 rounded transition-colors">取消</button>
            <button @click="confirm" :disabled="!selectedEntity" class="py-1 px-3 text-[12px] text-white bg-blue-600 hover:bg-blue-700 rounded transition-colors disabled:opacity-50">确定</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
