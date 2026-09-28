<script setup lang="ts">
import AppIcon from '@/components/common/AppIcon.vue'

function formatTime(ts: string) { if (!ts) return ''; const d = new Date(ts); return d.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) }
function formatSessionTime(ts: string) {
  if (!ts) return ''
  const d = new Date(ts)
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today.getTime() - 86400000)
  const time = d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (d >= today) return `今天 ${time}`
  if (d >= yesterday) return `昨天 ${time}`
  return d.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}
defineProps<{
  sessions: { id: string; name: string; messages: any[]; createdAt: string; updatedAt?: string }[]
  currentSessionId: string | null
  editingSessionId: string | null
  sessionNameEdit: string
}>()
const emit = defineEmits<{
  select: [id: string]
  new: []
  confirmDelete: [id: string]
  startRename: [id: string]
  saveRename: [id: string]
  updateSessionNameEdit: [val: string]
  cancelRename: []
}>()
</script>

<template>
<div class="w-[260px] flex flex-col flex-shrink-0 bg-surface-50/30 border-r border-surface-200 select-none">
  <!-- Header -->
  <div class="h-10 flex items-center justify-between px-3 border-b border-surface-200">
    <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">会话</span>
    <button @click="emit('new')" class="w-6 h-6 flex items-center justify-center rounded-lg text-surface-400 hover:text-brand-600 hover:bg-brand-50 transition-colors" v-tooltip="'新建会话'">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M12 4v16m8-8H4"/></svg>
    </button>
  </div>

  <!-- Session List -->
  <div class="flex-1 overflow-y-auto py-2 px-2 space-y-0.5 scrollbar-thin">
    <div v-if="!sessions.length" class="text-[11px] text-surface-400 text-center py-6">暂无会话，发送消息自动创建</div>
    <div v-for="s in sessions" :key="s.id" @click="emit('select', s.id)"
      class="group flex items-center gap-2.5 px-3 py-2 rounded-lg cursor-pointer text-[12px] transition-all duration-150"
      :class="s.id === currentSessionId ? 'bg-brand-50/70 text-brand-700 font-medium' : 'text-surface-600 hover:bg-surface-100'">
      <div class="w-5 h-5 rounded flex items-center justify-center flex-shrink-0"
        :class="s.id === currentSessionId ? 'text-brand-600' : 'text-surface-400'">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"/></svg>
      </div>
      <template v-if="editingSessionId === s.id">
        <input :value="sessionNameEdit" @input="(e:any) => emit('updateSessionNameEdit', e.target.value)"
          @keyup.enter="emit('saveRename', s.id)" @keyup.escape="emit('cancelRename')"
          @blur="emit('saveRename', s.id)"
          class="flex-1 min-w-0 bg-white border border-brand-300 rounded px-1.5 py-0.5 text-[12px] outline-none" @click.stop />
      </template>
      <template v-else>
        <div class="flex-1 min-w-0">
          <div class="truncate font-medium">{{ s.name }}</div>
          <div v-if="s.messages.length" class="text-[10px] mt-0.5 text-surface-400 truncate">{{ (() => { const m = [...s.messages].reverse().find(m => !m.isThinking); return m?.content?.slice(0,40) || '...' })() }}</div>
          <div v-if="s.updatedAt || s.createdAt" class="text-[9px] mt-0.5 text-surface-400">{{ formatSessionTime(s.updatedAt || s.createdAt) }}</div>
        </div>
        <!-- Rename button — matches NotesPage notebook edit style -->
        <button @click.stop="emit('startRename', s.id)"
          class="w-4 h-4 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 hover:bg-brand-50 opacity-0 group-hover:opacity-100 transition-colors" v-tooltip="'重命名'">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/></svg>
        </button>
        <!-- Delete button -->
        <button @click.stop="emit('confirmDelete', s.id)"
          class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0" v-tooltip="'删除'">
          <AppIcon name="trash" size="12"/>
        </button>
      </template>
    </div>
  </div>
</div>
</template>
