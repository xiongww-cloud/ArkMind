<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { notificationList, notificationMarkRead, notificationMarkAllRead, notificationDelete } from '@/api/notification-client'
import type { Notification } from '@/api/notification-client'

const props = defineProps<{
  visible?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const router = useRouter()
const notifications = ref<Notification[]>([])
const loading = ref(true)
const expandedId = ref<string | null>(null)

async function loadNotifications() {
  loading.value = true
  try {
    const r = await notificationList()
    notifications.value = r.notifications || []
  } catch (e) {
    console.error('[NotificationPanel] load failed:', e)
  } finally {
    loading.value = false
  }
}

onMounted(loadNotifications)
watch(() => props.visible, (val) => { if (val) loadNotifications() })

function toggleExpand(id: string) {
  expandedId.value = expandedId.value === id ? null : id
}

async function markRead(id: string) {
  await notificationMarkRead(id)
  const n = notifications.value.find(n => n.id === id)
  if (n) n.read = 1
}

async function markAllRead() {
  await notificationMarkAllRead()
  notifications.value.forEach(n => n.read = 1)
}

async function removeNotification(id: string) {
  await notificationDelete(id)
  notifications.value = notifications.value.filter(n => n.id !== id)
}

function goToEntity(n: Notification) {
  if (n.entity_type === 'project' && n.entity_id) {
    router.push(`/projects/${n.entity_id}`)
  } else if (n.entity_type === 'todo') {
    router.push('/todos')
  }
  emit('close')
}

function timeAgo(ts: string): string {
  if (!ts) return ''
  try {
    const norm = ts.includes('T') ? ts : ts.replace(' ', 'T')
    const diff = Date.now() - new Date(norm).getTime()
    if (isNaN(diff)) return ts.slice(0, 10)
    const mins = Math.floor(diff / 60000)
    if (mins < 1) return '刚刚'
    if (mins < 60) return `${mins}分钟前`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}小时前`
    const days = Math.floor(hrs / 24)
    if (days < 30) return `${days}天前`
    return ts.slice(0, 10)
  } catch { return ts.slice(0, 10) }
}

const typeIcon: Record<string, string> = {
  todo_due: '⏰',
  todo_today: '🔴',
  project_stale: '📌',
  bid_deadline: '🎯',
  llm_insight: '💡',
}

const priorityDot: Record<string, string> = {
  high: 'bg-red-500',
  medium: 'bg-amber-400',
  low: 'bg-surface-300',
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <div class="flex items-center justify-between px-4 py-3 border-b border-surface-100">
      <div class="flex items-center gap-2">
        <button @click="emit('close')" class="text-surface-500 hover:text-surface-700 transition-colors" title="返回">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15.75 19.5L8.25 12l7.5-7.5"/>
          </svg>
        </button>
        <h3 class="text-[13px] font-semibold text-surface-700">通知中心</h3>
      </div>
      <button @click="markAllRead" class="text-[11px] text-brand-600 hover:underline">全部已读</button>
    </div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading" class="p-6 text-center text-[12px] text-surface-400">加载中...</div>
      <div v-else-if="notifications.length === 0" class="p-6 text-center text-[12px] text-surface-400">暂无通知</div>
      <div v-else>
        <div v-for="n in notifications" :key="n.id"
          class="border-b border-surface-50 hover:bg-surface-50 transition-colors cursor-pointer"
          @click="toggleExpand(n.id)">
          <div class="flex items-start gap-2.5 px-4 py-3">
            <span class="text-[14px] mt-0.5 flex-shrink-0">{{ typeIcon[n.type] || '💡' }}</span>
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <div v-if="n.read === 0" class="w-1.5 h-1.5 rounded-full flex-shrink-0" :class="priorityDot[n.priority] || priorityDot.medium"></div>
                <span class="text-[12px] font-medium text-surface-800 truncate" :class="{ 'font-normal text-surface-600': n.read === 1 }">{{ n.title }}</span>
              </div>
              <p v-if="expandedId === n.id" class="text-[11px] text-surface-500 mt-1 whitespace-pre-wrap">{{ n.content }}</p>
            </div>
            <span class="text-[10px] text-surface-400 flex-shrink-0 mt-0.5">{{ timeAgo(n.created_at) }}</span>
          </div>
          <!-- Expanded actions -->
          <div v-if="expandedId === n.id" class="flex items-center gap-2 px-4 pb-3 pl-9">
            <button v-if="n.read === 0" @click.stop="markRead(n.id)" class="text-[10px] text-brand-600 hover:underline">标记已读</button>
            <button v-if="n.entity_id" @click.stop="goToEntity(n)" class="text-[10px] text-brand-600 hover:underline">查看详情</button>
            <button @click.stop="removeNotification(n.id)" class="text-[10px] text-red-500 hover:underline ml-auto">删除</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
