<script setup lang="ts">
import { ref, computed } from 'vue'
import { useProjectStore } from '@/stores/project'
import { fetchFiles, previewFile, updateFileMeta, fetchFileTree } from '@/api/db-client'
import AppIcon from '@/components/common/AppIcon.vue'

const props = defineProps<{ projectId: string }>()
const emit = defineEmits<{ insertFile: [file: any] }>()

const projectStore = useProjectStore()

const files = ref<any[]>([])
const folders = ref<any[]>([])
const loading = ref(false)
const expanded = ref<Set<string | null>>(new Set())
const editingFileId = ref<string | null>(null)
const editingFileName = ref('')

const grouped = computed(() => {
  const folderMap = new Map(folders.value.map((f: any) => [f.id, f.name]))
  const byFolder = new Map<string | null, any[]>()
  for (const f of files.value) {
    const fid = (f as any).folder_id || null
    if (!byFolder.has(fid)) byFolder.set(fid, [])
    byFolder.get(fid)!.push(f)
  }
  const groups: { folderId: string | null; folderName: string; files: any[] }[] = []
  for (const [fid, filesArr] of byFolder) {
    groups.push({ folderId: fid, folderName: fid ? folderMap.get(fid) || '未命名' : '未分类', files: filesArr })
  }
  groups.sort((a, b) => {
    if (a.folderId === null && b.folderId === null) return 0
    if (a.folderId === null) return 1
    if (b.folderId === null) return -1
    return a.folderName.localeCompare(b.folderName)
  })
  return groups
})

function toggleFolder(fid: string | null) {
  if (expanded.value.has(fid)) expanded.value.delete(fid)
  else expanded.value.add(fid)
}

async function loadFiles() {
  loading.value = true
  try {
    const backendId = await projectStore.syncProjectToBackend(props.projectId)
    const [filesData, treeData] = await Promise.all([
      fetchFiles(backendId),
      fetchFileTree(backendId)
    ])
    files.value = Array.isArray(filesData) ? filesData : []
    folders.value = treeData?.folders || []
  } catch { files.value = []; folders.value = [] }
  finally { loading.value = false }
}

function fileDisplayName(f: any): string {
  return f.custom_name || f.displayName || f.name || f.lib_cn || f.lib_gn || '未命名'
}

function fileTypeClass(ext: string) {
  const map: Record<string, string> = {
    docx: 'bg-blue-50 text-blue-600', doc: 'bg-blue-50 text-blue-600',
    pdf: 'bg-red-50 text-red-600',
    xlsx: 'bg-emerald-50 text-emerald-600', xls: 'bg-emerald-50 text-emerald-600',
    pptx: 'bg-orange-50 text-orange-600', ppt: 'bg-orange-50 text-orange-600',
    png: 'bg-purple-50 text-purple-600', jpg: 'bg-purple-50 text-purple-600',
  }
  return map[ext.toLowerCase()] || 'bg-slate-50 text-slate-500'
}

async function previewFileById(fileId: string) {
  try { await previewFile(fileId) } catch (e) { console.error('[Project] previewFile failed:', e) }
}

function startRename(f: any) {
  editingFileId.value = f.id
  editingFileName.value = f.custom_name || ''
  import('vue').then(({ nextTick }) => {
    nextTick(() => {
      const el = document.querySelector<HTMLInputElement>('[data-panel-rename-input]')
      el?.focus(); el?.select()
    })
  })
}

async function confirmRename(f: any) {
  const n = editingFileName.value.trim()
  if (n) {
    try { await updateFileMeta(f.id, { customName: n }); await loadFiles() } catch (e) { console.error('[Project] renameFile failed:', e) }
  }
  editingFileId.value = null
}

function cancelRename() { editingFileId.value = null }

defineExpose({ loadFiles })
</script>

<template>
  <div class="flex-shrink-0 w-[300px] border-l border-surface-200 flex flex-col bg-surface-50/20">
    <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
      <span class="text-[10px] font-semibold text-surface-400 uppercase tracking-wider">项目文件</span>
      <span class="text-[9px] text-surface-300">{{ files.length }}</span>
    </div>
    <div class="flex-1 overflow-y-auto p-2">
      <div v-if="loading" class="text-center py-4"><p class="text-xs text-surface-400">加载中...</p></div>
      <template v-else-if="grouped.length > 0">
        <div v-for="group in grouped" :key="group.folderId || '__uncat__'" class="mb-0.5">
          <button @click="toggleFolder(group.folderId)" class="w-full flex items-center gap-1 px-1.5 py-1 rounded text-[10px] font-medium text-surface-500 hover:text-surface-700 hover:bg-surface-100 transition-colors cursor-pointer">
            <svg class="w-2.5 h-2.5 transition-transform shrink-0" :class="expanded.has(group.folderId) ? 'rotate-90' : ''" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
            <svg class="w-3.5 h-3.5 shrink-0" :class="group.folderId ? 'text-amber-500' : 'text-surface-400'" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5"><path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/></svg>
            <span class="truncate ml-1">{{ group.folderName }}</span>
            <span class="ml-auto text-surface-300 tabular-nums">{{ group.files.length }}</span>
          </button>
          <div v-if="expanded.has(group.folderId)" class="ml-3 space-y-0.5 mt-0.5">
            <div v-for="f in group.files" :key="f.id" class="group relative flex items-center py-1 px-1.5 rounded hover:bg-white hover:shadow-sm transition-all">
              <div class="w-6 h-6 flex items-center justify-center rounded font-bold text-[7px] tracking-wider shrink-0 select-none" :class="fileTypeClass(f.file_ext || '')">{{ (f.file_ext || 'file').toUpperCase() }}</div>
              <div class="ml-1.5 flex-1 min-w-0 flex items-center gap-1">
                <template v-if="editingFileId === f.id">
                  <input v-model="editingFileName" data-panel-rename-input class="w-full text-[10px] font-medium text-surface-700 bg-transparent outline-none border-b border-brand-400 pb-0.5" @keyup.enter="confirmRename(f)" @blur="confirmRename(f)" @click.stop @keyup.escape="cancelRename()" />
                </template>
                <template v-else>
                  <p class="text-[10px] font-medium text-surface-700 truncate leading-tight cursor-pointer hover:text-blue-600 flex-1 min-w-0" :title="fileDisplayName(f)" @click="previewFileById(f.id)">{{ fileDisplayName(f) }}</p>
                </template>
                <button @click.stop="emit('insertFile', f)" class="shrink-0 w-4.5 h-4.5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors opacity-0 group-hover:opacity-100 ml-1" title="插入到笔记"><AppIcon name="file-plus" size="12"/></button>
              </div>
            </div>
          </div>
        </div>
      </template>
      <div v-else class="text-center py-8"><p class="text-xs text-surface-400">暂无项目文件</p><p class="text-[10px] text-surface-300 mt-1">前往「文件」tab 上传</p></div>
    </div>
  </div>
</template>
