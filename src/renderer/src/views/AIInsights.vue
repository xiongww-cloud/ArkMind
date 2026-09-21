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
    <div class="flex items-center justify-between px-4 py-2.5 border-b border-surface-100 shrink-0">
      <div class="flex items-center gap-2 min-w-0">
        <AppIcon name="bell" class="h-4 w-4 text-surface-500" />
        <h1 class="text-sm font-semibold text-surface-800">AI 洞察</h1>
        <span v-if="unreadCount > 0"
          class="px-1.5 py-0.5 rounded-full bg-red-500 text-white text-[10px] font-medium leading-none">
          {{ unreadCount > 99 ? '99+' : unreadCount }}
        </span>
        <!-- 多选模式控件 -->
        <template v-if="panelRef?.multiSelectMode">
          <span class="text-[10px] text-surface-400 ml-1">已选{{ panelRef?.selectedCount }}条</span>
          <button @click="panelRef?.toggleSelectAll()"
            class="text-[10px] text-blue-600 hover:underline">
            {{ panelRef?.allSelected ? '取消全选' : '全选' }}
          </button>
          <button @click="panelRef?.exitMultiSelect()"
            class="text-[10px] text-surface-400 hover:text-surface-600">取消</button>
          <button v-if="panelRef?.selectedCount > 0" @click="panelRef?.batchDelete()"
            class="text-[10px] text-red-500 hover:underline">批量删除</button>
        </template>
        <template v-else>
          <button @click="panelRef?.enterMultiSelect()"
            class="text-[10px] text-blue-600 hover:underline ml-1">多选</button>
        </template>
      </div>
      <div class="flex items-center gap-2">
        <button v-if="unreadCount > 0 && !panelRef?.multiSelectMode"
          @click="handleMarkAllRead"
          class="text-[10px] text-blue-600 hover:underline">全部已读</button>
      </div>
    </div>
    <!-- 通知列表（全宽复用 NotificationPanel） -->
    <div class="flex-1 min-h-0 overflow-y-auto">
      <NotificationPanel ref="panelRef" :visible="true" @read="loadStats" />
    </div>
  </div>
</template>
