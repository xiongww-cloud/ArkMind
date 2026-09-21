<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { onActivated } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import NotificationPanel from '@/components/notification/NotificationPanel.vue'
import { notificationStats, notificationMarkAllRead } from '@/api/notification-client'
import { toast } from '@/composables/useToast'
import { listen } from '@tauri-apps/api/event'

const unreadCount = ref(0)
const panelRef = ref<InstanceType<typeof NotificationPanel> | null>(null)
let unlistenNotify: (() => void) | null = null

async function loadStats() {
  try {
    const s = await notificationStats()
    if (s) unreadCount.value = s.unread_count || 0
  } catch { /* ignore */ }
}

async function handleMarkAllRead() {
  try {
    await notificationMarkAllRead()
    unreadCount.value = 0
    panelRef.value?.reload()
    toast({ message: '已全部标记为已读', type: 'success' })
  } catch {
    toast({ message: '操作失败', type: 'error' })
  }
}

onMounted(async () => {
  await loadStats()
  unlistenNotify = await listen<string>('notification:new', () => loadStats())
})
onActivated(() => { loadStats(); panelRef.value?.reload() })
</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm flex flex-col overflow-hidden">
    <!-- 工具栏 -->
    <div class="flex items-center gap-2 px-4 py-2 border-b border-surface-100 shrink-0">
      <button v-if="unreadCount > 0 && !panelRef?.multiSelectMode"
        @click="handleMarkAllRead"
        class="btn-secondary">全部已读</button>
      <template v-if="panelRef?.multiSelectMode">
        <span class="text-[10px] text-surface-400">已选{{ panelRef?.selectedCount }}条</span>
        <button @click="panelRef?.toggleSelectAll()" class="btn-secondary">
          {{ panelRef?.allSelected ? '取消全选' : '全选' }}
        </button>
        <button @click="panelRef?.exitMultiSelect()" class="btn-secondary">取消</button>
        <button v-if="panelRef?.selectedCount > 0" @click="panelRef?.batchDelete()"
          class="btn-secondary !text-red-500 !border-red-200 hover:!bg-red-50">批量删除</button>
      </template>
      <template v-else>
        <button @click="panelRef?.enterMultiSelect()" class="btn-secondary">多选</button>
      </template>
      <div class="flex-1" />
      <span v-if="unreadCount > 0" class="text-[11px] text-surface-400">未读{{ unreadCount }}</span>
    </div>
    <!-- 通知列表（全宽复用 NotificationPanel） -->
    <div class="flex-1 min-h-0 overflow-y-auto">
      <NotificationPanel ref="panelRef" :visible="true" @read="loadStats" />
    </div>
  </div>
</template>
