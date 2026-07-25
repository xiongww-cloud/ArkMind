<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useProjectStore } from '@/stores/project'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'
import {
  fetchPartners, updatePartner, deletePartner, partnerLinkProject, partnerUnlinkProject,
  partnerUpdateRemark, contactsListAll, contactsListChannels, contactLinkProject, contactUnlinkProject,
  createChannelContact, updatePartnerContact,
} from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'
import SelectWithSearch from '@/components/common/SelectWithSearch.vue'

const props = defineProps<{ project: any; projectId: string }>()
const emit = defineEmits<{ update: [] }>()

const projectStore = useProjectStore()

const channels = ref<any[]>([])
const channelEditNameId = ref('')
const channelEditRemarkId = ref('')
const channelEditName = ref('')
const channelEditRemark = ref('')
const expandedChannelId = ref<string | null>(null)

const showChannelSelectModal = ref(false)
const allChannels = ref<any[]>([])
const selectedChannelId = ref('')
const selectedChannelContactIds = ref<string[]>([])
const channelRemark = ref('')
const channelContactSearch = ref('')

const selectedChannel = computed(() => allChannels.value.find(c => c.id === selectedChannelId.value) || null)
const channelContacts = computed(() => selectedChannel.value?.contacts || [])
const filteredChannelContacts = computed(() => {
  const q = channelContactSearch.value.toLowerCase().trim()
  if (!q) return channelContacts.value
  return channelContacts.value.filter((c: any) =>
    (c.name || '').toLowerCase().includes(q) || (c.phone || '').toLowerCase().includes(q) || (c.position || '').toLowerCase().includes(q)
  )
})

async function confirmChannelSelect() {
  if (!selectedChannel.value) return
  const channel = selectedChannel.value
  const channelName = channel.name
  const contactIds = [...selectedChannelContactIds.value]
  const remark = channelRemark.value || null
  showChannelSelectModal.value = false
  selectedChannelId.value = ''
  selectedChannelContactIds.value = []
  channelRemark.value = ''
  channelContactSearch.value = ''
  try {
    if (contactIds.length > 0) {
      await partnerLinkProject(channel.id, props.projectId, contactIds[0], remark)
      for (const cid of contactIds.slice(1)) {
        await contactLinkProject(cid, props.projectId)
      }
    } else {
      await partnerLinkProject(channel.id, props.projectId, null, remark)
    }
    channels.value = await fetchPartners(props.projectId, 'CHANNEL') || []
    await projectStore.addProjectDynamic(props.projectId, `添加了渠道「${channelName}」`)
    toast({ message: `已添加渠道「${channelName}」`, type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('添加渠道失败', e)
    toast({ message: '添加渠道失败', type: 'error' })
  }
}

function toggleChannel(channelId: string) {
  expandedChannelId.value = expandedChannelId.value === channelId ? null : channelId
}

async function loadChannels() {
  try { channels.value = await fetchPartners(props.projectId, 'CHANNEL') } catch { channels.value = [] }
}

function startEditChannelName(ch: any) {
  channelEditNameId.value = ch.id
  channelEditName.value = ch.name || ''
}
async function cancelEditChannelName() {
  const chId = channelEditNameId.value
  const ch = channels.value.find(c => c.id === chId)
  channelEditNameId.value = ''
  if (ch && !ch.name) {
    try {
      await deletePartner(chId)
      channels.value = channels.value.filter(c => c.id !== chId)
    } catch (e) { console.warn('取消添加渠道', e) }
  }
}
async function saveChannelName(channelId: string) {
  if (channelEditNameId.value !== channelId) return
  const n = channelEditName.value.trim()
  const ch = channels.value.find(c => c.id === channelId)
  if (ch && n === (ch.name || '')) { channelEditNameId.value = ''; return }
  channelEditNameId.value = ''
  if (!n && ch && !ch.name) {
    try {
      await deletePartner(channelId)
      channels.value = channels.value.filter(c => c.id !== channelId)
    } catch (e) { console.warn('取消添加渠道', e) }
    return
  }
  try {
    const oldName = ch?.name || '未命名'
    await updatePartner({ id: channelId, name: n || '' })
    if (ch) ch.name = n
    await projectStore.addProjectDynamic(props.projectId, `渠道「${oldName}」更名为「${n || '未命名'}」`)
    emit('update')
  } catch (e) { console.warn('更新渠道名称失败', e) }
}

function startEditChannelRemark(ch: any) {
  channelEditRemarkId.value = ch.id
  channelEditRemark.value = ch.project_remark || ch.remark || ''
}
function cancelEditChannelRemark() { channelEditRemarkId.value = '' }
async function saveChannelRemark(channelId: string) {
  const r = channelEditRemark.value.trim()
  const ch = channels.value.find(c => c.id === channelId)
  if (ch && r === (ch.project_remark || ch.remark || '')) { channelEditRemarkId.value = ''; return }
  channelEditRemarkId.value = ''
  try {
    await partnerUpdateRemark(channelId, props.projectId, r || null)
    if (ch) ch.project_remark = r
  } catch (e) { console.warn('更新渠道备注失败', e) }
}

async function addChannel() {
  showChannelSelectModal.value = true
  try {
    allChannels.value = await contactsListAll('CHANNEL') || []
  } catch (e) { console.warn('加载渠道列表失败', e) }
}
async function removeChannel(channelId: string) {
  const ok = await dangerConfirm({
    title: '删除渠道',
    message: '确定将此渠道从项目中移除？',
    confirmText: '移除'
  })
  if (!ok) return
  try {
    await partnerUnlinkProject(channelId, props.projectId)
    channels.value = channels.value.filter(c => c.id !== channelId)
    await projectStore.addProjectDynamic(props.projectId, '移除了一个渠道')
    toast({ message: '已移除渠道', type: 'success' })
    emit('update')
  } catch (e) {
    console.warn('移除渠道失败', e)
    toast({ message: '移除失败', type: 'error' })
  }
}

const editingContactNameOf = ref('')
const editingContactPhoneOf = ref('')
const addingContactToChannelId = ref('')
const contactEditName = ref('')
const contactEditPhone = ref('')
const contactEditEmail = ref('')
const contactEditPosition = ref('')

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
  if (!name || !selectedChannelId.value) return
  try {
    const ct = await createChannelContact({ channelId: selectedChannelId.value, name, phone: createContactPhone.value.trim(), email: createContactEmail.value.trim(), position: createContactPosition.value.trim() })
    // 关联到项目
    await contactLinkProject(ct.id, props.projectId)
    // 更新本地联系人列表
    const ch = channels.value.find(c => c.id === selectedChannelId.value)
    if (ch) {
      if (!Array.isArray(ch.contacts)) ch.contacts = []
      ch.contacts.push({ id: ct.id, name: ct.name || name, phone: ct.phone || '', email: ct.email || '', position: ct.position || '' })
    }
    // 自动选中新创建的联系人
    selectedChannelContactIds.value = [ct.id]
    showCreateContactModal.value = false
    await projectStore.addProjectDynamic(props.projectId, `添加了联系人「${name}」`)
    toast({ message: `已创建联系人「${name}」`, type: 'success' })
  } catch (e) {
    console.warn('创建联系人失败', e)
    toast({ message: '创建联系人失败', type: 'error' })
  }
}

const showContactSelectModal = ref(false)
const contactSelectChannelId = ref('')
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

async function startAddContact(channelId: string) {
  contactSelectChannelId.value = channelId
  if (allChannels.value.length === 0) {
    try { allChannels.value = await contactsListChannels() || [] } catch {}
  }
  const ch = channels.value.find(c => c.id === channelId)
  const systemChannel = allChannels.value.find(sc => sc.name === ch?.name)
  availableContacts.value = systemChannel?.contacts || []
  showContactSelectModal.value = true
}

async function confirmContactSelect() {
  if (selectedContactIds.value.length === 0 || !contactSelectChannelId.value) return
  const contactIds = [...selectedContactIds.value]
  const channelId = contactSelectChannelId.value
  showContactSelectModal.value = false
  selectedContactIds.value = []
  contactSelectSearch.value = ''
  try {
    for (const cid of contactIds) {
      await contactLinkProject(cid, props.projectId)
    }
    const ch = channels.value.find(c => c.id === channelId)
    if (ch) {
      if (!Array.isArray(ch.contacts)) ch.contacts = []
      for (const cid of contactIds) {
        const ct = availableContacts.value.find(c => c.id === cid)
        if (ct && !ch.contacts.find(c => c.id === ct.id)) {
          ch.contacts.push({ id: ct.id, name: ct.name, phone: ct.phone, email: ct.email, position: ct.position })
        }
      }
    }
    await projectStore.addProjectDynamic(props.projectId, `添加了联系人`)
    toast({ message: `已添加 ${contactIds.length} 个联系人`, type: 'success' })
  } catch (e) {
    console.warn('添加联系人失败', e)
    toast({ message: '添加联系人失败', type: 'error' })
  }
}

function startEditContactName(ct: any) {
  editingContactNameOf.value = ct.id
  contactEditName.value = ct.name || ''
}
function cancelEditContactName() { editingContactNameOf.value = '' }
async function saveContactName(channelId: string, contactId: string) {
  if (editingContactNameOf.value !== contactId) return
  const n = contactEditName.value.trim()
  editingContactNameOf.value = ''
  if (!n) return
  try {
    await updatePartnerContact({ id: contactId, name: n })
    const ch = channels.value.find(c => c.id === channelId)
    const ct = ch?.contacts?.find((c: any) => c.id === contactId)
    const oldName = ct?.name || ''
    if (ct) ct.name = n
    await projectStore.addProjectDynamic(props.projectId, `联系人姓名从「${oldName}」改为「${n}」`)
  } catch (e) { console.warn('更新联系人姓名失败', e) }
}

function startEditContactPhone(ct: any) {
  editingContactPhoneOf.value = ct.id
  contactEditPhone.value = ct.phone || ''
}
function cancelEditContactPhone() { editingContactPhoneOf.value = '' }
async function saveContactPhone(channelId: string, contactId: string) {
  if (editingContactPhoneOf.value !== contactId) return
  const p = contactEditPhone.value.trim()
  editingContactPhoneOf.value = ''
  try {
    await updatePartnerContact({ id: contactId, phone: p || '' })
    const ch = channels.value.find(c => c.id === channelId)
    const ct = ch?.contacts?.find((c: any) => c.id === contactId)
    const op = ct?.phone || ''
    if (ct) ct.phone = p
    const cname = ct?.name || '未命名'
    await projectStore.addProjectDynamic(props.projectId, `联系人「${cname}」电话从「${op}」改为「${p}」`)
  } catch (e) { console.warn('更新联系人电话失败', e) }
}

async function saveNewContact(channelId: string) {
  if (!addingContactToChannelId.value || addingContactToChannelId.value !== 'new-' + channelId) return
  const n = contactEditName.value.trim()
  const p = contactEditPhone.value.trim()
  const e = contactEditEmail.value.trim()
  const pos = contactEditPosition.value.trim()
  addingContactToChannelId.value = ''
  if (!n && !p && !e && !pos) return
  try {
    const ct = await createChannelContact({ channelId, name: n, phone: p || '', email: e || '', position: pos || '' })
    const ch = channels.value.find(c => c.id === channelId)
    if (ch) {
      if (!Array.isArray(ch.contacts)) ch.contacts = []
      ch.contacts.push({ id: ct.id, name: ct.name || n, phone: ct.phone || p, email: ct.email || e, position: ct.position || pos })
    }
    await contactLinkProject(ct.id, props.projectId)
    await projectStore.addProjectDynamic(props.projectId, `添加了联系人「${n}」`)
  } catch (e) { console.warn('添加联系人失败', e) }
}

async function removeContact(channelId: string, contactId: string) {
  try {
    await contactUnlinkProject(contactId, props.projectId)
    const ch = channels.value.find(c => c.id === channelId)
    if (ch?.contacts) { ch.contacts = ch.contacts.filter((c: any) => c.id !== contactId) }
    await projectStore.addProjectDynamic(props.projectId, '移除了一个联系人')
    toast({ message: '已移除联系人', type: 'success' })
  } catch (e) { console.warn('移除联系人失败', e) }
}

watch(() => props.projectId, () => { loadChannels() }, { immediate: true })
</script>

<template>
  <div class="mt-4 border-t border-surface-100 pt-4">
    <div class="flex items-center justify-between mb-4">
      <h3 class="text-xs font-semibold text-slate-500 uppercase tracking-wide">渠道信息</h3>
      <button v-if="!project.isArchived" @click="addChannel" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors">
        <AppIcon name="plus" size="10" stroke="2.5" /> 添加渠道
      </button>
    </div>
    <div v-if="channels.length === 0" class="text-sm text-slate-400 italic py-2">暂未添加渠道</div>
    <div v-for="ch in channels" :key="ch.id" class="card !p-0 overflow-hidden mb-3">
      <div @click="toggleChannel(ch.id)" class="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-surface-50 transition-colors group/ch-card select-none">
        <div class="h-8 w-8 rounded-lg flex items-center justify-center flex-shrink-0 bg-blue-50 text-blue-500">
          <AppIcon name="building" class="h-4 w-4" />
        </div>
        <div class="flex-1 min-w-0 flex items-center gap-2">
          <p class="text-xs font-medium text-surface-600 whitespace-nowrap flex-shrink-0">{{ ch.name || '未命名' }}</p>
          <template v-if="channelEditRemarkId === ch.id">
            <input v-model="channelEditRemark" @blur="saveChannelRemark(ch.id)" @keyup.enter="saveChannelRemark(ch.id)" @keyup.escape="cancelEditChannelRemark" @click.stop
              class="text-[11px] text-slate-500 bg-transparent border-b border-slate-300 outline-none focus:border-blue-400 min-w-[80px] px-0.5" />
          </template>
          <template v-else>
            <span class="text-[11px] text-slate-400 whitespace-nowrap shrink-0">{{ ch.project_remark || '无备注' }}</span>
            <button v-if="!project.isArchived" @click.stop="startEditChannelRemark(ch)" class="text-slate-300 hover:text-blue-500 shrink-0" title="编辑备注">
              <AppIcon name="edit" size="10" />
            </button>
          </template>
        </div>
        <div class="flex items-center gap-0.5 flex-shrink-0">
          <span class="text-[11px] text-slate-400">{{ (ch.contacts || []).length }} 个联系人</span>
          <button v-if="!project.isArchived" @click.stop="removeChannel(ch.id)" class="btn-ghost !p-1 text-slate-400 hover:text-red-500" title="删除渠道">
            <AppIcon name="trash" class="w-3 h-3" />
          </button>
          <AppIcon name="chevron-right" class="h-3.5 w-3.5 text-surface-300 transition-transform duration-200" :class="expandedChannelId === ch.id ? 'rotate-90' : ''" />
        </div>
      </div>

      <div v-show="expandedChannelId === ch.id" class="border-t border-surface-100 bg-gray-50/50">
        <div class="p-4">
          <div v-if="ch.contacts && ch.contacts.length > 0">
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
                <tr v-for="ct in (ch.contacts || [])" :key="ct.id" class="border-b border-slate-100 group/ct-row hover:bg-white/60 transition-colors">
                  <td class="py-2 pr-2"><span class="text-xs text-slate-600 truncate block">{{ ct.name || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.phone || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.position || '-' }}</span></td>
                  <td class="py-2 pr-2"><span class="text-xs text-slate-500 truncate block">{{ ct.email || '-' }}</span></td>
                  <td class="py-2 text-center"><button v-if="!project.isArchived" @click.stop="removeContact(ch.id, ct.id)" class="text-slate-300 hover:text-red-500 transition-colors opacity-0 group-hover/ct-row:opacity-100" title="删除联系人"><AppIcon name="trash" size="12"/></button></td>
                </tr>
              </tbody>
            </table>
          </div>
          <p v-else class="text-xs text-slate-400 italic">暂无联系人</p>
          <button v-if="!project.isArchived" @click="startAddContact(ch.id)" class="mt-3 btn-secondary">
            <AppIcon name="plus" class="w-2.5 h-2.5" /> 添加联系人
          </button>
        </div>
      </div>
    </div>
  </div>

  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showChannelSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showChannelSelectModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 520px;">
          <!-- 固定头部 -->
          <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <h3 class="text-sm font-semibold text-surface-800 mb-3">选择渠道</h3>
            <label class="text-[11px] text-surface-500 mb-1 block">渠道</label>
            <SelectWithSearch
              v-model="selectedChannelId"
              :options="allChannels.map(c => ({ id: c.id, name: c.name, address: c.address }))"
              placeholder="选择渠道..."
            />
          </div>
          <!-- 联系人区块 -->
          <div v-if="selectedChannel" class="flex-1 flex flex-col min-h-0 px-5 py-3">
            <div class="flex items-center justify-between mb-2">
              <span class="text-[11px] text-surface-500 font-medium">联系人</span>
              <button @click="openCreateContactModal" class="text-[10px] text-blue-600 hover:underline">+ 添加联系人</button>
            </div>
            <div v-if="channelContacts.length > 0" class="flex-1 flex flex-col min-h-0">
              <input v-model="channelContactSearch" type="text" class="w-full px-3 py-1 text-[11px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400 mb-2 shrink-0" placeholder="搜索联系人..." />
              <div class="flex-1 overflow-y-auto border border-surface-200 rounded-lg">
                <label v-for="ct in filteredChannelContacts" :key="ct.id" class="flex items-center gap-2 px-3 py-2 hover:bg-surface-50 cursor-pointer border-b border-surface-50 last:border-b-0">
                  <input type="checkbox" :value="ct.id" v-model="selectedChannelContactIds" class="w-3 h-3 rounded border-slate-200 text-blue-500 focus:ring-blue-400" />
                  <span class="text-xs text-slate-600 flex-1 truncate">{{ ct.name || '-' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.position || '' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.phone || '' }}</span>
                  <span class="text-[10px] text-slate-400">{{ ct.email || '' }}</span>
                </label>
              </div>
            </div>
            <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该渠道暂无联系人</div>
          </div>
          <!-- 备注 + 按钮 -->
          <div class="px-5 py-3 border-t border-surface-100 shrink-0">
            <div class="mb-3">
              <label class="text-[11px] text-surface-500 mb-1 block">备注</label>
              <input v-model="channelRemark" type="text" class="w-full px-3 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-blue-400" placeholder="项目级备注（可选）" />
            </div>
            <div class="flex items-center justify-end gap-2">
              <button @click="showChannelSelectModal = false" class="btn-secondary">取消</button>
              <button @click="confirmChannelSelect" :disabled="!selectedChannel" class="btn-primary">确定{{ selectedChannelContactIds.length > 0 ? ` (${selectedChannelContactIds.length})` : '' }}</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <Teleport to="body">
    <Transition name="confirm-fade">
      <div v-if="showContactSelectModal" class="fixed inset-0 z-[80] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/20" @click="showContactSelectModal = false" />
        <div class="relative bg-white rounded-xl shadow-lg border border-surface-100 w-full max-w-lg mx-4 z-10 flex flex-col" style="height: 420px;">
          <!-- 固定头部 -->
          <div class="px-5 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <h3 class="text-sm font-semibold text-surface-800 mb-3">选择联系人</h3>
            <div v-if="availableContacts.length > 0" class="mb-0">
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
            <div v-else class="flex-1 flex items-center justify-center text-[12px] text-surface-400">该渠道暂无联系人</div>
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
