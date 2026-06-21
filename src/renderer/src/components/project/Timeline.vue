<script setup lang="ts">
import { ref, computed } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { formatDateTime } from '@/utils/time'
import { useProjectStore } from '@/stores/project'
import { useAuthStore } from '@/stores/auth'
import { dangerConfirm } from '@/composables/useConfirm'

const props = defineProps({
  events: { type: Array, default: () => [] },
  projectId: { type: String, default: '' },
  logFilter: { type: String, default: 'all' }
})

const emit = defineEmits(['refresh'])

const projectStore = useProjectStore()
const authStore = useAuthStore()

const filteredEvents = computed(() => {
  const list = [...props.events].sort((a, b) => String(b.date || '').localeCompare(String(a.date || '')))
  if (props.logFilter === 'all') return list
  return list.filter(e => e.type === props.logFilter)
})

const editingId = ref(null)
const editingVal = ref('')

function startEdit(event) {
  if (event.type !== 'note') return
  editingId.value = event._id
  editingVal.value = event.content || ''
}

function cancelEdit() {
  editingId.value = null
}

async function saveEdit(event) {
  const val = editingVal.value.trim()
  if (!val || val === event.content) { editingId.value = null; return }
  try {
    await projectStore.editDynamic(event._id, props.projectId, val)
    editingId.value = null
    emit('refresh')
  } catch (e) {
    console.warn('编辑失败', e)
  }
}

async function handleDelete(event) {
  if (event.type !== 'note') return
  const ok = await dangerConfirm({ title: '删除日志', message: '确定删除这条日志？', confirmText: '删除' })
  if (!ok) return
  try {
    await projectStore.deleteDynamic(event._id, props.projectId)
    emit('refresh')
  } catch (e) {
    console.warn('删除失败', e)
  }
}
</script>

<template>
  <div class="h-full flex flex-col min-h-0">
    <div v-if="filteredEvents.length === 0" class="flex-1 flex flex-col items-center justify-center text-surface-300">
      <AppIcon name="clock" class="mx-auto h-10 w-10" />
      <p class="mt-2 text-sm">暂无日志记录</p>
    </div>

    <div v-else class="flex-1 overflow-y-auto space-y-2 pr-1 [&::-webkit-scrollbar]:w-1.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-gray-200 [&::-webkit-scrollbar-thumb]:rounded-full">
      <div v-for="(event, idx) in filteredEvents" :key="event._id || idx"
        class="flex items-center gap-2 bg-white rounded-lg border px-3 py-2 group/log"
        :class="event.type === 'system' ? 'border-surface-100' : 'border-surface-100'">

        <!-- Icon -->
        <div class="w-5 h-5 rounded-full flex items-center justify-center flex-shrink-0"
          :class="event.type === 'system' ? 'bg-blue-50' : 'bg-amber-50'">
          <AppIcon name="clock" class="w-2.5 h-2.5" :class="event.type === 'system' ? 'text-blue-400' : 'text-amber-500'" />
        </div>

        <!-- Content area -->
        <div class="flex-1 min-w-0">
          <!-- Header: time -->
          <div class="flex items-center gap-2 mb-0.5">
            <span class="text-[10px] text-surface-400">{{ formatDateTime(event.date) }}</span>
            <span v-if="event.operator && event.operator !== authStore?.currentUser?.name" class="text-[10px] text-surface-400">{{ event.operator }}</span>
          </div>

          <!-- Content (editable for user logs) -->
          <template v-if="editingId === event._id">
            <input v-model="editingVal" @keyup.enter="saveEdit(event)" @keyup.escape="cancelEdit" @blur="saveEdit(event)" class="w-full text-xs bg-white border border-blue-400 rounded px-1.5 py-0.5 outline-none focus:ring-1 focus:ring-blue-200 pr-6" placeholder="输入日志内容..." />
          </template>
          <p v-else class="text-xs text-surface-600 leading-relaxed">{{ event.content }}</p>
        </div>

        <!-- Actions (user log only, hover) -->
        <div v-if="event.type === 'note'" class="flex items-center gap-0.5 flex-shrink-0 opacity-0 group-hover/log:opacity-100 transition-opacity -mr-0.5">
          <button @click.stop="startEdit(event)" class="btn-ghost !p-0.5 text-slate-300 hover:text-blue-600" title="编辑">
            <AppIcon name="edit" class="w-3 h-3" />
          </button>
          <button @click.stop="handleDelete(event)" class="btn-ghost !p-0.5 text-slate-300 hover:text-red-500" title="删除">
            <AppIcon name="trash" class="w-3 h-3" />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
