<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'

const props = defineProps<{
  files?: any[]
  noteId?: string
}>()

const emit = defineEmits<{
  upload: [file: { path: string; name?: string }]
  preview: [file: any]
  insert: [file: any]
  delete: [file: any]
  rename: [file: any, name: string]
}>()

const isCollapsed = ref(true)
const isDragging = ref(false)
const fileInput = ref<HTMLInputElement>()
const editingFileId = ref<string | null>(null)
const editingName = ref('')

async function triggerFileInput() {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const result = await open({ filters: [], multiple: true })
  if (!result) return
  const paths = Array.isArray(result) ? result : [result]
  for (const p of paths) {
    emit('upload', { path: p } as any)
  }
}

function handleFileSelect(_e: Event) {
  // Not used; Tauri dialog handles file picking
}

function formatSize(bytes: number | undefined): string {
  if (!bytes || bytes === 0) return '--'
  if (bytes >= 1073741824) return (bytes/1073741824).toFixed(1)+' GB'
  if (bytes >= 1048576) return (bytes/1048576).toFixed(1)+' MB'
  if (bytes >= 1024) return (bytes/1024).toFixed(0)+' KB'
  return bytes+' B'
}

function getFileTypeClass(type: string) {
  const map: Record<string, string> = {
    docx: 'bg-blue-50 text-blue-600', doc: 'bg-blue-50 text-blue-600',
    pdf: 'bg-red-50 text-red-600',
    xlsx: 'bg-emerald-50 text-emerald-600', xls: 'bg-emerald-50 text-emerald-600',
    pptx: 'bg-orange-50 text-orange-600', ppt: 'bg-orange-50 text-orange-600',
    png: 'bg-purple-50 text-purple-600', jpg: 'bg-purple-50 text-purple-600', jpeg: 'bg-purple-50 text-purple-600',
  }
  return map[type] || 'bg-slate-50 text-slate-500'
}

function handleDragOver(e: DragEvent) { e.preventDefault(); isDragging.value = true }
function handleDragLeave() { isDragging.value = false }
function handleDrop(e: DragEvent) {
  e.preventDefault()
  isDragging.value = false
  const fs = e.dataTransfer?.files
  if (fs) {
    for (let i = 0; i < fs.length; i++) {
      const f = fs[i] as any
      emit('upload', { path: f.path, name: f.name })
    }
  }
}

function startRename(file: any) {
  editingFileId.value = file.id
  editingName.value = file.lib_gn || file.lib_cn || file.custom_name || file.global_name || file.mapping_name || ''
  import('vue').then(({ nextTick }) => nextTick(() => {
    const el = document.querySelector<HTMLInputElement>('[data-note-rename-input]')
    el?.focus(); el?.select()
  }))
}
function confirmRename(file: any) {
  const n = editingName.value.trim()
  if (n) emit('rename', file, n)
  editingFileId.value = null
}
function cancelRename() { editingFileId.value = null }

const panelRef = ref<HTMLElement>()
function handleClickOutside(e: MouseEvent) {
  if (isCollapsed.value) return
  const target = e.target as HTMLElement
  // 点击面板内、折叠按钮、或标签弹窗时不关闭
  if (target.closest('[data-att-panel]') || target.closest('[data-att-toggle]') || document.querySelector('[class*="fixed inset-0 z-\[80\]"]')?.contains(target)) return
  isCollapsed.value = true
}
onMounted(() => document.addEventListener('mousedown', handleClickOutside))
onUnmounted(() => document.removeEventListener('mousedown', handleClickOutside))
function fileNameFor(file: any) {
  return file.lib_gn || file.lib_cn || file.custom_name || file.global_name || file.mapping_name || '未命名'
}
</script>

<template>
  <div class="absolute right-0 top-0 bottom-0 select-none pointer-events-none">
    <!-- panel wrapper (floating, width collapses) -->
    <div
      ref="panelRef"
      data-att-panel
      class="relative flex flex-col h-full bg-white shadow-lg transition-all duration-300 ease-in-out pointer-events-auto"
      :class="isCollapsed ? 'w-0 border-l-0' : 'w-80 border-l border-gray-100 overflow-visible'"
    >
      <!-- toggle -->
      <button
        data-att-toggle
        @click="isCollapsed = !isCollapsed"
        class="absolute -left-4 top-1/2 -translate-y-1/2 z-50 w-4 h-14 flex items-center justify-center bg-white border border-y-gray-200/80 border-l-gray-200/80 border-r-0 rounded-l-xl shadow-[-4px_0_8px_rgba(0,0,0,0.03)] text-gray-400 hover:text-blue-500 transition-all cursor-pointer select-none pointer-events-auto"
      >
        <svg
          class="h-3 w-3 transition-transform duration-300"
          :class="{ 'rotate-180': isCollapsed }"
          fill="none" viewBox="0 0 24 24" stroke="currentColor"
          stroke-width="2.5"
        >
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7" />
        </svg>
      </button>

      <!-- inner content (dragover target) -->
      <div
        class="relative flex flex-col h-full overflow-hidden"
        @dragover.prevent="handleDragOver"
        @dragleave.prevent="handleDragLeave"
        @drop.prevent="handleDrop"
      >
        <!-- drag overlay -->
        <div v-if="isDragging" class="absolute inset-0 bg-blue-50/60 border-2 border-dashed border-blue-400 z-30 flex flex-col items-center justify-center pointer-events-none transition-all">
          <svg class="w-8 h-8 text-blue-400 mb-2" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M12 4v16m8-8H4"/></svg>
          <span class="text-blue-600 text-xs font-medium">释放文件以上传到当前笔记</span>
        </div>

        <!-- header -->
        <div class="flex items-center justify-between px-3 h-9 border-b border-gray-100 shrink-0">
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] font-semibold text-gray-700">附件</span>
            <span class="text-[9px] px-1.5 py-0.5 rounded-full bg-gray-100 text-gray-500">{{ files?.length || 0 }}</span>
          </div>
          <button @click="triggerFileInput" class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors cursor-pointer" title="上传附件">
            <AppIcon name="plus" size="10" stroke="2.5"/>
            <span>上传附件</span>
          </button>
        </div>

        <!-- file list -->
        <div class="flex-1 overflow-y-auto p-3 space-y-2 custom-scrollbar">
          <div v-if="!files || files.length === 0" class="h-64 flex flex-col items-center justify-center text-center px-6">
            <div class="w-14 h-14 rounded-2xl bg-gray-50 flex items-center justify-center text-gray-300 mb-4">
              <svg class="h-7 w-7" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.2"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
            </div>
            <p class="text-[13px] font-medium text-gray-400">暂无附件</p>
            <p class="text-[11px] text-gray-300 mt-1">拖拽文件或点击上方 + 上传</p>
          </div>

          <div
            v-for="file in files" :key="file.id"
            class="group relative flex items-center p-2 rounded-xl border border-gray-100 bg-white hover:border-gray-200 hover:shadow-sm transition-all duration-200"
            :class="editingFileId === file.id ? 'cursor-default' : ''"
          >
            <div class="w-9 h-9 flex items-center justify-center rounded-lg font-bold text-[8px] tracking-wider shrink-0 select-none" :class="getFileTypeClass(file.file_ext || '')">{{ (file.file_ext || 'file').toUpperCase() }}</div>
            <div class="ml-2.5 flex-1 min-w-0 pr-16">
              <template v-if="editingFileId === file.id">
                <input
                  v-model="editingName"
                  data-note-rename-input
                  class="w-full text-[11px] font-medium text-gray-700 bg-transparent outline-none border-b border-brand-400 pb-0.5"
                  @keyup.enter="confirmRename(file)"
                  @blur="confirmRename(file)"
                  @click.stop
                  @keyup.escape="cancelRename()"
                />
              </template>
              <template v-else>
                <p class="text-[11px] font-medium text-gray-700 truncate leading-tight cursor-pointer hover:text-blue-600 transition-colors" :title="fileNameFor(file)" @click="emit('preview', file)">{{ fileNameFor(file) }}</p>
              </template>
              <p class="text-[9px] text-gray-400 mt-0.5">{{ formatSize(file.file_size) }} · {{ (file.mapped_at || file.updated_at || '').slice(0,16) }}</p>
            </div>
            <div class="absolute right-1.5 top-1/2 -translate-y-1/2 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity duration-150 bg-gradient-to-l from-white via-white pl-4">
              <button @click.stop="startRename(file)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="重命名"><AppIcon name="edit" size="13"/></button>
              <button @click.stop="emit('insert', file)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="插入到光标"><AppIcon name="file-plus" size="13"/></button>
              <button @click.stop="emit('delete', file)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除附件"><AppIcon name="trash" size="13"/></button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.custom-scrollbar::-webkit-scrollbar { width: 4px; }
.custom-scrollbar::-webkit-scrollbar-track { background: transparent; }
.custom-scrollbar::-webkit-scrollbar-thumb { background: #e5e7eb; border-radius: 2px; }
.custom-scrollbar::-webkit-scrollbar-thumb:hover { background: #d1d5db; }
</style>
