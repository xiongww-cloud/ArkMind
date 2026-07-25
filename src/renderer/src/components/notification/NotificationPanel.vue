<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { notificationList, notificationMarkRead, notificationDelete } from '@/api/notification-client'
import type { Notification } from '@/api/notification-client'
import { sanitizeHtml } from '@/utils/sanitize'
import { useTooltip } from '@/composables/useTooltip'
import { dangerConfirm } from '@/composables/useConfirm'

const props = defineProps<{
  visible?: boolean
}>()

const emit = defineEmits<{
  close: []
  read: []
}>()

const router = useRouter()
const notifications = ref<Notification[]>([])
const loading = ref(true)
const expandedId = ref<string | null>(null)

async function renderMd(text: string): Promise<string> {
  if (!text) return ''
  try {
    const mod = await import('marked')
    const Marked = mod.Marked || (mod as any).default?.Marked || (mod as any).default
    if (Marked && typeof Marked === 'function') {
      const marked = new Marked({ gfm: true, breaks: true })
      return sanitizeHtml(marked.parse(text) || '')
    }
    const { marked } = mod as any
    return sanitizeHtml((marked?.parse || marked)(text) || '')
  } catch { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;') }
}

const renderedContent = ref<Record<string, string>>({})

async function renderContent(n: Notification) {
  if (renderedContent.value[n.id]) return
  renderedContent.value[n.id] = await renderMd(n.content)
}

async function loadNotifications() {
  loading.value = true
  try {
    const r = await notificationList()
    notifications.value = r.notifications || []
    // Pre-render all content
    for (const n of notifications.value) {
      await renderContent(n)
    }
  } catch (e) {
    console.error('[NotificationPanel] load failed:', e)
  } finally {
    loading.value = false
  }
}

defineExpose({ reload: loadNotifications })

onMounted(loadNotifications)
watch(() => props.visible, (val) => { if (val) loadNotifications() })

function toggleExpand(id: string) {
  expandedId.value = expandedId.value === id ? null : id
}

async function markRead(id: string) {
  await notificationMarkRead(id)
  const n = notifications.value.find(n => n.id === id)
  if (n) n.read = 1
  emit('read')
}

async function removeNotification(id: string) {
  const ok = await dangerConfirm({ title: '删除通知', message: '确认删除这条通知？' })
  if (!ok) return
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

const typeLabel: Record<string, string> = {
  todo_due: '待办到期',
  todo_today: '今日待办',
  project_stale: '项目停滞',
  bid_deadline: '投标截止',
  llm_insight: 'AI 洞察',
}

const typeLabelColor: Record<string, string> = {
  todo_due: 'bg-amber-50 text-amber-600',
  todo_today: 'bg-red-50 text-red-600',
  project_stale: 'bg-slate-100 text-slate-500',
  bid_deadline: 'bg-blue-50 text-blue-600',
  llm_insight: 'bg-violet-50 text-violet-600',
}

const priorityDot: Record<string, string> = {
  high: 'bg-red-500',
  medium: 'bg-amber-400',
  low: 'bg-surface-300',
}

// ── Tooltip ──
const { tooltipText, tooltipX, tooltipY, tooltipVisible, show: showTip, hide: hideTip } = useTooltip()

function onTitleEnter(e: MouseEvent, title: string) {
  const el = e.currentTarget as HTMLElement
  if (el.scrollWidth > el.clientWidth || el.scrollHeight > el.clientHeight) {
    showTip(el, title)
  }
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- List -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="loading" class="p-6 text-center text-sm text-surface-400">加载中...</div>
      <div v-else-if="notifications.length === 0" class="p-6 text-center text-sm text-surface-400">暂无通知</div>
      <div v-else>
        <div v-for="n in notifications" :key="n.id"
          class="border-b border-surface-50">
          <button @click="toggleExpand(n.id)"
            class="w-full flex items-center gap-3 px-4 py-[5px] text-left transition-colors cursor-pointer"
            :class="expandedId === n.id ? 'bg-blue-50 hover:bg-blue-100 active:bg-blue-200' : 'hover:bg-slate-50 active:bg-slate-100'">
            <span v-if="typeLabel[n.type]"
              class="text-[10px] px-1.5 py-0.5 rounded font-medium flex-shrink-0 leading-none"
              :class="typeLabelColor[n.type] || 'bg-slate-100 text-slate-500'">
              {{ typeLabel[n.type] }}
            </span>
            <div class="flex-1 min-w-0 flex items-center gap-2">
              <div v-if="n.read === 0" class="w-2 h-2 rounded-full flex-shrink-0" :class="priorityDot[n.priority] || priorityDot.medium"></div>
              <span @mouseenter="(e) => onTitleEnter(e, n.title)" @mouseleave="hideTip"
                class="text-[13px] font-medium text-slate-800 truncate cursor-default"
                :class="{ 'font-normal text-slate-500': n.read === 1 }">{{ n.title }}</span>
            </div>
            <span class="text-[11px] text-slate-400 flex-shrink-0">{{ timeAgo(n.created_at) }}</span>
            <svg class="w-3 h-3 flex-shrink-0 text-slate-400 transition-transform duration-200"
                 :class="{ 'rotate-90': expandedId === n.id }"
                 fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7"/>
            </svg>
          </button>
          <!-- Expanded content -->
          <div v-if="expandedId === n.id">
            <div v-if="renderedContent[n.id]" class="notif-md px-4 pb-2 pl-14 text-[13px] text-slate-600 leading-relaxed" v-html="renderedContent[n.id]" />
            <div class="flex items-center gap-3 px-4 pb-2 pl-14">
              <button v-if="n.read === 0" @click.stop="markRead(n.id)" class="text-[11px] text-blue-600 hover:underline">标记已读</button>
              <button v-if="n.entity_id" @click.stop="goToEntity(n)" class="text-[11px] text-blue-600 hover:underline">查看详情</button>
              <button @click.stop="removeNotification(n.id)" class="text-[11px] text-red-500 hover:underline ml-auto">删除</button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Tooltip (Teleport) -->
    <Teleport to="body">
      <div v-if="tooltipVisible"
           class="fixed pointer-events-none z-[9999] px-2 py-1 text-[11px] text-white bg-gray-800 rounded shadow-lg max-w-[320px]"
           :class="tooltipText.length > 40 ? 'break-words' : 'whitespace-nowrap'"
           :style="{ left: tooltipX + 'px', top: tooltipY + 'px', transform: 'translate(-50%, -100%)' }">
        {{ tooltipText }}
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.notif-md :deep(p) { margin: 0.3em 0; }
.notif-md :deep(strong) { font-weight: 600; color: #1e293b; }
.notif-md :deep(ul), .notif-md :deep(ol) { padding-left: 1.2em; margin: 0.2em 0; }
.notif-md :deep(li) { margin: 0.15em 0; }
.notif-md :deep(code) { background: #f1f5f9; color: #e11d48; padding: 1px 4px; border-radius: 3px; font-size: 0.9em; }
.notif-md :deep(pre) { background: #f8fafc; padding: 8px 10px; border-radius: 6px; overflow-x: auto; margin: 0.4em 0; border: 1px solid #e2e8f0; font-size: 12px; }
.notif-md :deep(pre code) { background: none; color: inherit; padding: 0; }
.notif-md :deep(blockquote) { border-left: 3px solid #3b82f6; padding: 4px 10px; margin: 0.4em 0; color: #475569; background: #f0f7ff; border-radius: 0 4px 4px 0; }
.notif-md :deep(a) { color: #2563eb; text-decoration: none; }
.notif-md :deep(a:hover) { text-decoration: underline; }
.notif-md :deep(table) { width: 100%; border-collapse: collapse; margin: 0.4em 0; font-size: 12px; }
.notif-md :deep(th), .notif-md :deep(td) { padding: 4px 8px; border: 1px solid #e2e8f0; text-align: left; }
.notif-md :deep(th) { background: #f8fafc; font-weight: 600; }
</style>
