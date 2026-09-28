<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { fetchFileTree, listGlobalFiles } from '@/api/db-client'

const props = defineProps<{ projects: { id: string; name: string }[] }>()
const emit = defineEmits<{
  selectProject: [p: { id: string; name: string }]
  selectFile: [f: any]
  close: []
}>()

const atTab = ref<'project'|'file'>('project')
const atSearch = ref('')
const atFilesLoading = ref(false)
const fileTreeNodes = ref<any[]>([])
const collapsedFolders = ref<Set<string>>(new Set())

// ─── Auto-position below @ button ─────────────────────────────────
const popoverX = ref(-1)
const popoverY = ref(0)
const popoverMaxH = ref('380px')

function updatePosition() {
  const btn = document.querySelector('[data-at-btn]') as HTMLElement|null
  if (!btn) { popoverX.value = 0; popoverY.value = 0; return }
  const r = btn.getBoundingClientRect()
  const pw = 340
  const gap = 4
  let left = r.left + r.width - pw
  let top = r.bottom + gap

  if (left < 8) left = 8
  if (left + pw > window.innerWidth - 8) left = window.innerWidth - pw - 8

  const spaceBelow = window.innerHeight - top - 8
  const spaceAbove = r.top - 8
  if (spaceBelow < 200 && spaceAbove > spaceBelow) {
    top = r.top - Math.min(380, spaceAbove) - gap
    popoverMaxH.value = Math.min(380, spaceAbove) + 'px'
  } else {
    popoverMaxH.value = Math.min(380, spaceBelow) + 'px'
  }

  popoverX.value = left
  popoverY.value = top
}

const filteredProjects = computed(() => {
  const q = atSearch.value.toLowerCase().trim()
  return q ? props.projects.filter(p => p.name.toLowerCase().includes(q)) : props.projects
})

// Build folder+file tree
function buildFileTree(folders: any[], files: any[]): any[] {
  const folderMap = new Map<string, any>()
  const roots: any[] = []

  // Index folders
  for (const f of folders) {
    folderMap.set(f.id, {
      id: f.id,
      name: f.name,
      parent_id: f.parent_id || '',
      sort_order: f.sort_order || 0,
      file_count: f.file_count || 0,
      children: [],
    })
  }

  // Build nested structure
  for (const [id, node] of folderMap) {
    if (node.parent_id && folderMap.has(node.parent_id)) {
      folderMap.get(node.parent_id).children.push(node)
    } else {
      roots.push(node)
    }
  }

  // Sort children by sort_order
  const sortNodes = (nodes: any[]) => {
    nodes.sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name))
    for (const n of nodes) sortNodes(n.children)
  }
  sortNodes(roots)

  // Attach files to their folders (two-pass: first collect, then mutate)
  const filesByFolder = new Map<string, any[]>()
  const orphanFiles: any[] = []
  for (const f of files) {
    const fid = f.lib_folder_id
    if (fid && folderMap.has(fid)) {
      if (!filesByFolder.has(fid)) filesByFolder.set(fid, [])
      filesByFolder.get(fid)!.push(f)
    } else {
      // 无有效 folder_id 的文件记为孤文件，归入未分类根
      orphanFiles.push(f)
    }
  }

  function attachFiles(nodes: any[], parentId: string) {
    const sub = filesByFolder.get(parentId) || []
    for (const sf of sub) {
      nodes.push({ id: sf.id, _type: 'file', _data: sf, name: sf.lib_gn || sf.lib_cn || sf.custom_name || '未命名', ext: sf.file_ext || '' })
    }
  }

  function walkAttach(nodes: any[]) {
    for (const n of nodes) {
      if (n.children) {
        attachFiles(n.children, n.id)
        walkAttach(n.children)
      }
    }
  }

  // 无目录归属的文件归入"未分类"虚拟目录
  if (orphanFiles.length > 0) {
    roots.push({
      id: '__uncategorized__',
      name: '未分类',
      _type: 'folder',
      children: orphanFiles.map(sf => ({
        id: sf.id, _type: 'file', _data: sf,
        name: sf.lib_gn || sf.lib_cn || sf.custom_name || '未命名',
        ext: sf.file_ext || '',
      })),
    })
  }

  walkAttach(roots)

  return roots
}

function toggleFolder(folderId: string) {
  const next = new Set(collapsedFolders.value)
  if (next.has(folderId)) next.delete(folderId)
  else next.add(folderId)
  collapsedFolders.value = next
}

function isFolderCollapsed(folderId: string) { return collapsedFolders.value.has(folderId) }

// Source files for search mode
const allFlatFiles = ref<any[]>([])

// Flatten tree into visible rows with depth for indentation
const flatVisibleNodes = computed(() => {
  const q = atSearch.value.toLowerCase().trim()

  // Search mode: flat filtered list
  if (q) {
    return allFlatFiles.value
      .filter((f: any) => {
        const name = f.lib_gn || f.lib_cn || f.custom_name || ''
        return name.toLowerCase().includes(q)
      })
      .map((f: any) => ({
        id: f.id,
        _type: 'file',
        _data: f,
        _depth: 0,
        name: f.lib_gn || f.lib_cn || f.custom_name || '未命名',
        ext: f.file_ext || '',
      }))
  }

  // Tree mode
  const result: any[] = []
  function walk(nodes: any[], depth: number) {
    for (const node of nodes) {
      if (node._type === 'file') {
        result.push({ ...node, _depth: depth })
      } else {
        result.push({ ...node, _depth: depth })
        if (!collapsedFolders.value.has(node.id)) {
          walk(node.children || [], depth + 1)
        }
      }
    }
  }
  walk(fileTreeNodes.value, 0)
  return result
})

function open() {
  atTab.value = 'project'
  atSearch.value = ''
}

async function fetchAtFiles() {
  atFilesLoading.value = true
  try {
    const [treeRes, filesRes] = await Promise.all([
      fetchFileTree(),
      listGlobalFiles(),
    ])
    const folders = treeRes?.folders || []
    const files = (filesRes as any)?.files || []
    allFlatFiles.value = files
    fileTreeNodes.value = buildFileTree(folders, files)
    // Default: all folders collapsed（包括未分类）
    collapsedFolders.value = new Set(folders.map((f: any) => f.id).concat('__uncategorized__'))
  } catch { fileTreeNodes.value = [] }
  finally { atFilesLoading.value = false }
}

function onSelectProject(p: { id: string; name: string }) { emit('selectProject', p) }
function onSelectFile(f: any) {
  emit('selectFile', f._data || f)
}
function onClose() {
  emit('close')
}

function handleClickOutside(e: MouseEvent) {
  const el = document.querySelector('[data-at-popover]')
  if (el && !el.contains(e.target as Node)) onClose()
}

onMounted(() => {
  fetchAtFiles()
  nextTick(updatePosition)
  document.addEventListener('mousedown', handleClickOutside)
})

onUnmounted(() => {
  document.removeEventListener('mousedown', handleClickOutside)
})

defineExpose({ open, updatePosition })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="popoverX >= 0"
        class="fixed z-[60] w-[340px]"
        :style="{ left: popoverX + 'px', top: popoverY + 'px', height: popoverMaxH }"
        data-at-popover
      >
        <div
          class="bg-white rounded-lg shadow-lg border border-surface-200 flex flex-col overflow-hidden h-full"
          @click.stop
          @mousedown.stop
        >
          <!-- Tabs -->
          <div class="flex border-b border-surface-100 shrink-0">
            <button
              @click="atTab='project'"
              class="flex-1 py-2 text-xs font-medium transition-colors"
              :class="atTab==='project' ? 'text-brand-600 border-b-2 border-brand-600' : 'text-surface-500 hover:text-surface-700'"
            >项目</button>
            <button
              @click="atTab='file'"
              class="flex-1 py-2 text-xs font-medium transition-colors"
              :class="atTab==='file' ? 'text-brand-600 border-b-2 border-brand-600' : 'text-surface-500 hover:text-surface-700'"
            >文件</button>
          </div>
          <!-- Search -->
          <div class="px-3 py-2 border-b border-surface-100 shrink-0">
            <div class="flex items-center gap-1.5 bg-surface-50 rounded-lg px-2.5 py-1.5">
              <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
              <input v-model="atSearch" type="text" class="flex-1 bg-transparent text-xs outline-none" placeholder="搜索..." @keydown.escape="onClose"/>
            </div>
          </div>
          <!-- List -->
          <div class="flex-1 overflow-y-auto py-1 min-h-0">
            <!-- Projects -->
            <template v-if="atTab==='project'">
              <button
                v-for="p in filteredProjects" :key="p.id"
                @click="onSelectProject(p)"
                v-tooltip.auto="p.name"
                class="w-full flex items-center gap-2 px-3 py-2 text-xs text-left transition-colors hover:bg-surface-50 group relative"
              >
                <AppIcon name="folder-open" size="13" class="text-surface-400 flex-shrink-0"/>
                <span class="truncate text-surface-700 flex-1 min-w-0">{{ p.name }}</span>
              </button>
              <div v-if="filteredProjects.length === 0" class="px-3 py-4 text-xs text-surface-400 text-center">
                {{ atSearch ? '无匹配项目' : '暂无项目' }}
              </div>
            </template>
            <!-- Files — folder tree -->
            <template v-if="atTab==='file'">
              <div v-if="atFilesLoading" class="px-3 py-4 text-xs text-surface-400 text-center">加载中...</div>
              <template v-else>
                <template v-for="node in flatVisibleNodes" :key="node.id + (node._depth||0)">
                  <!-- Folder -->
                  <button
                    v-if="node._type !== 'file'"
                    @click="toggleFolder(node.id)"
                    v-tooltip.auto="node.name"
                    class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-surface-600 hover:bg-surface-50 transition-colors"
                    :style="{ paddingLeft: `${12 + (node._depth||0) * 16}px` }"
                  >
                    <svg class="w-3 h-3 flex-shrink-0 transition-transform duration-150" :class="isFolderCollapsed(node.id)?'':'rotate-90'" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M9 5l7 7-7 7"/></svg>
                    <AppIcon name="folder" size="13" class="text-surface-400 flex-shrink-0"/>
                    <span class="truncate flex-1 min-w-0">{{ node.name }}</span>
                  </button>
                  <!-- File -->
                  <button
                    v-else
                    @click="onSelectFile(node)"
                    v-tooltip.auto="node.name"
                    class="w-full flex items-center gap-2 pr-3 py-1.5 text-xs text-left hover:bg-surface-50 transition-colors"
                    :style="{ paddingLeft: `${12 + (node._depth||0) * 16}px` }"
                  >
                    <AppIcon name="file-doc" size="13" class="text-surface-400 flex-shrink-0"/>
                    <span class="truncate text-surface-700 flex-1 min-w-0">{{ node.name }}</span>
                    <span class="text-[10px] text-surface-400 flex-shrink-0">{{ node.ext }}</span>
                  </button>
                </template>
                <div v-if="!atFilesLoading && flatVisibleNodes.length === 0" class="px-3 py-4 text-xs text-surface-400 text-center">
                  暂无文件
                </div>
              </template>
            </template>
          </div>
        </div>
      </div>
  </Teleport>
</template>
