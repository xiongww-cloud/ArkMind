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
// 数据流视图：待处理（默认）/已处理（已办结+已忽略，可恢复）——铃铛弹窗保持单流，区分只在全页
const viewFilter = ref<'pending' | 'handled'>('pending')
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
      <!-- 流切换：待处理 / 已处理 -->
      <div class="flex items-center gap-1">
        <button @click="viewFilter = 'pending'"
          class="px-2.5 py-1 text-[11px] rounded-md transition-colors"
          :class="viewFilter === 'pending' ? 'bg-blue-50 text-blue-600 font-medium' : 'text-slate-500 hover:bg-slate-50'">待处理</button>
        <button @click="viewFilter = 'handled'"
          class="px-2.5 py-1 text-[11px] rounded-md transition-colors"
          :class="viewFilter === 'handled' ? 'bg-blue-50 text-blue-600 font-medium' : 'text-slate-500 hover:bg-slate-50'">已处理</button>
      </div>
      <div class="w-px h-4 bg-surface-200 mx-1" />
      <button v-if="viewFilter === 'pending' && unreadCount > 0 && !panelRef?.multiSelectMode"
        @click="handleMarkAllRead"
        class="btn-secondary">全部已读</button>
      <template v-if="panelRef?.multiSelectMode">
        <span class="text-[10px] text-surface-400">已选{{ panelRef?.selectedCount }}条</span>
        <button @click="panelRef?.toggleSelectAll()" class="btn-secondary">
          {{ panelRef?.allSelected ? '取消全选' : '全选' }}
        </button>
        <button @click="panelRef?.exitMultiSelect()" class="btn-secondary">取消</button>
        <button v-if="viewFilter === 'pending' && panelRef?.selectedCount > 0" @click="panelRef?.batchComplete()"
          class="btn-secondary !text-emerald-600 !border-emerald-200 hover:!bg-emerald-50">批量办结</button>
        <button v-if="panelRef?.selectedCount > 0" @click="panelRef?.batchDelete()"
          class="btn-secondary !text-red-500 !border-red-200 hover:!bg-red-50">批量删除</button>
      </template>
      <template v-else>
        <button @click="panelRef?.enterMultiSelect()" class="btn-secondary">多选</button>
      </template>
      <div class="flex-1" />
    </div>
    <!-- 通知列表（全宽复用 NotificationPanel，stateFilter 变化时面板自行重载） -->
    <div class="flex-1 min-h-0 overflow-y-auto">
      <NotificationPanel ref="panelRef" :visible="true" :state-filter="viewFilter" @read="loadStats" />
    </div>
  </div>
</template>
