<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { usePresentationsStore } from '@/stores/presentations'
import type { FolderNode, PresentationItem } from '@/types/presentation'
import PresentationFrame from '@/components/presentation/PresentationFrame.vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { toast } from '@/composables/useToast'
import { dangerConfirm } from '@/composables/useConfirm'

const router = useRouter()
const store = usePresentationsStore()

onMounted(() => store.hydrate())

// ─── 目录选择 ────────────────────────────────────────────────────
type Selection = 'all' | 'none' | string // 'all'=全部 'none'=未分类 其他=目录ID
const selected = ref<Selection>('all')
const search = ref('')

interface FlatNode {
  id: string
  name: string
  depth: number
  parentId: string | null
  hasChildren: boolean
}
const collapsedFolders = ref<Set<string>>(new Set())
function toggleFolder(id: string) {
  const s = new Set(collapsedFolders.value)
  if (s.has(id)) s.delete(id)
  else s.add(id)
  collapsedFolders.value = s
}

const flatFolders = computed<FlatNode[]>(() => {
  const out: FlatNode[] = []
  const walk = (nodes: FolderNode[], depth: number) => {
    for (const n of nodes) {
      out.push({
        id: n.id,
        name: n.name,
        depth,
        parentId: n.parentId,
        hasChildren: n.children?.length > 0
      })
      if (!collapsedFolders.value.has(n.id) && n.children?.length) walk(n.children, depth + 1)
    }
  }
  walk(store.folderTree, 0)
  return out
})

function selectNode(sel: Selection) {
  selected.value = sel
}

// ─── 列表过滤 ────────────────────────────────────────────────────
const list = computed<PresentationItem[]>(() => {
  let items = store.listInFolder(
    selected.value === 'all' ? 'all' : selected.value === 'none' ? null : selected.value
  )
  const q = search.value.trim().toLowerCase()
  if (q) items = items.filter((p) => p.title.toLowerCase().includes(q))
  return items
})

// ─── 时间显示 ────────────────────────────────────────────────────
function fmtTime(ts: number): string {
  const d = new Date(ts)
  const today = new Date()
  const yesterday = new Date(Date.now() - 86400_000)
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  if (d.toDateString() === today.toDateString()) return `今天 ${hm}`
  if (d.toDateString() === yesterday.toDateString()) return `昨天 ${hm}`
  return `${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

// ─── 新建演示 ────────────────────────────────────────────────────
const createOpen = ref(false)
const createTitle = ref('')
const createFolder = ref<string>('none')

const folderOptions = computed(() => [
  { value: 'none', label: '未分类' },
  ...flatFolders.value.map((f) => ({ value: f.id, label: `${'　'.repeat(f.depth)}${f.name}` }))
])

function openCreate() {
  createTitle.value = ''
  createFolder.value =
    selected.value !== 'all' && selected.value !== 'none' ? selected.value : 'none'
  createOpen.value = true
}
function submitCreate() {
  const title = createTitle.value.trim()
  if (!title) return
  const fid = createFolder.value === 'none' ? null : createFolder.value
  const id = store.createPresentation(title, fid)
  createOpen.value = false
  toast({ message: '已创建，开始和 AI 沟通策划吧', type: 'success' })
  router.push(`/presentations/${id}`)
}

function openPresentation(p: PresentationItem) {
  router.push(`/presentations/${p.id}`)
}

// ─── 移动到目录 ──────────────────────────────────────────────────
const moveTarget = ref<PresentationItem | null>(null)
const moveFolder = ref('none')
function openMove(p: PresentationItem) {
  moveTarget.value = p
  moveFolder.value = p.folderId ?? 'none'
}
function submitMove() {
  if (!moveTarget.value) return
  store.movePresentation(moveTarget.value.id, moveFolder.value === 'none' ? null : moveFolder.value)
  moveTarget.value = null
  toast({ message: '已移动', type: 'success' })
}

// ─── 删除 ────────────────────────────────────────────────────────
async function removePresentation(p: PresentationItem) {
  const ok = await dangerConfirm({
    title: '删除演示材料',
    message: `确定删除「${p.title}」？此操作不可撤销。`,
    confirmText: '删除'
  })
  if (!ok) return
  store.deletePresentation(p.id)
  toast({ message: '已删除', type: 'success' })
}

// ─── 目录管理（行内新增 / 重命名 / 删除） ────────────────────────
const editingFolderId = ref<string | null>(null)
const folderEditName = ref('')
const addingNew = ref(false)
const addingParentId = ref<string | null>(null)
const newFolderName = ref('')

/** 行内编辑自动聚焦 */
const vFocus = {
  mounted: (el: HTMLInputElement) => {
    el.focus()
    el.select()
  }
}

function isDuplicateName(name: string, excludeId?: string, parentId?: string | null): boolean {
  if (!name.trim()) return false
  const q = name.trim().toLowerCase()
  return store.folders.some(
    (f) =>
      f.id !== excludeId &&
      f.name.toLowerCase() === q &&
      (f.parentId || null) === (parentId || null)
  )
}
function startAddFolder() {
  addingNew.value = true
  addingParentId.value = null
  newFolderName.value = '新建目录'
}
function startAddSubFolder(parentId: string) {
  addingNew.value = true
  addingParentId.value = parentId
  newFolderName.value = '新建目录'
  const s = new Set(collapsedFolders.value)
  s.delete(parentId)
  collapsedFolders.value = s
}
function confirmAddFolder() {
  const n = newFolderName.value.trim()
  addingNew.value = false
  if (!n) return
  if (isDuplicateName(n, undefined, addingParentId.value)) {
    toast({ message: '同级目录下已存在同名文件夹', type: 'error' })
    return
  }
  const id = store.addFolder(n, addingParentId.value)
  selectNode(id)
  toast({ message: '目录已创建', type: 'success' })
  void id
}
function startRename(id: string, name: string) {
  editingFolderId.value = id
  folderEditName.value = name
}
function confirmRename(id: string) {
  const n = folderEditName.value.trim()
  editingFolderId.value = null
  if (!n) return
  const f = store.folders.find((x) => x.id === id)
  if (f && n !== f.name && isDuplicateName(n, id, f.parentId)) {
    toast({ message: '同级目录下已存在同名文件夹', type: 'error' })
    return
  }
  store.renameFolder(id, n)
}
async function removeFolder(id: string) {
  const f = store.folders.find((x) => x.id === id)
  if (!f) return
  const ok = await dangerConfirm({
    title: '删除目录',
    message: `删除「${f.name}」后，其中的材料将移入「未分类」，子目录将上提一级。`,
    confirmText: '删除'
  })
  if (!ok) return
  store.deleteFolder(id)
  if (selected.value === id) selected.value = 'all'
  toast({ message: '目录已删除', type: 'success' })
}

// ─── 拖拽材料到目录 ──────────────────────────────────────────────
const dragItem = ref<PresentationItem | null>(null)
const isDragging = ref(false)
const dragOverId = ref('')

function onDragStart(_e: DragEvent, p: PresentationItem) {
  dragItem.value = p
  isDragging.value = true
}
function onDropOnFolder(folderId: string) {
  dragOverId.value = ''
  if (!dragItem.value) return
  store.movePresentation(dragItem.value.id, folderId)
  toast({ message: '已移动', type: 'success' })
  dragItem.value = null
  isDragging.value = false
}
</script>

<template>
  <div
    class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm"
  >
    <!-- ═══ 左侧目录树（对齐笔记库/资料库） ═══ -->
    <aside
      class="w-52 flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden"
    >
      <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider"
          >目录</span
        >
        <button
          class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
          title="新建目录"
          @click="startAddFolder()"
        >
          <AppIcon name="plus" size="12" stroke="2.5" />
        </button>
      </div>

      <!-- 搜索 -->
      <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
        <div
          class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1"
        >
          <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0" />
          <input
            v-model="search"
            type="text"
            class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300"
            placeholder="搜索演示材料..."
          />
        </div>
      </div>

      <!-- 树 -->
      <div class="flex-1 px-2 py-1" style="overflow-y: auto; overflow-x: clip">
        <!-- 全部材料 -->
        <button
          class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden"
          :style="{ paddingLeft: `${12}px` }"
          :class="[
            selected === 'all'
              ? 'bg-blue-50/70 text-brand-700 font-medium'
              : 'text-surface-600 hover:bg-surface-100'
          ]"
          @click="selectNode('all')"
        >
          <div
            v-if="selected === 'all'"
            class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"
          ></div>
          <AppIcon name="grid" size="13" class="flex-shrink-0" />
          全部材料
        </button>

        <!-- 目录树节点 -->
        <template v-for="node in flatFolders" :key="node.id">
          <!-- 行内重命名 -->
          <div
            v-if="editingFolderId === node.id"
            class="flex items-center gap-1.5 py-1 min-w-0 pr-2"
            :style="{ paddingLeft: `${12 + node.depth * 12}px` }"
          >
            <span class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" />
            <input
              v-model="folderEditName"
              v-focus
              class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text"
              @keyup.enter="confirmRename(node.id)"
              @blur="confirmRename(node.id)"
            />
          </div>

          <!-- 正常节点 -->
          <div
            v-else
            :data-drop-target="node.id"
            class="group/folder flex items-center gap-1.5 w-full px-2 py-1.5 text-xs text-left transition-colors rounded-lg cursor-pointer relative overflow-hidden select-none"
            :class="[
              selected === node.id
                ? 'bg-blue-50/70 text-brand-700 font-medium'
                : 'text-surface-600 hover:bg-surface-100',
              isDragging && dragOverId === node.id
                ? 'bg-blue-50 ring-2 ring-blue-300 text-brand-700'
                : ''
            ]"
            :style="{ paddingLeft: `${12 + node.depth * 12}px` }"
            @click="selectNode(node.id)"
            @dragover.prevent="dragOverId = node.id"
            @dragleave="dragOverId = ''"
            @drop.prevent="onDropOnFolder(node.id)"
          >
            <div
              v-if="selected === node.id"
              class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"
            ></div>
            <span
              v-if="node.hasChildren"
              class="w-3 h-3 flex items-center justify-center rounded text-surface-400 flex-shrink-0"
              @click.stop="toggleFolder(node.id)"
            >
              <svg
                class="w-2.5 h-2.5 transition-transform duration-150"
                :class="!collapsedFolders.has(node.id) ? 'rotate-90' : ''"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2.5"
              >
                <path d="M9 5l7 7-7 7" />
              </svg>
            </span>
            <span v-else class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" />
            <span class="truncate flex-1">{{ node.name }}</span>
            <span
              class="w-4 h-4 hidden group-hover/folder:flex items-center justify-center rounded text-surface-400 hover:text-brand-600"
              title="新增子目录"
              @click.stop="startAddSubFolder(node.id)"
            >
              <AppIcon name="plus" size="10" />
            </span>
            <span
              class="w-4 h-4 hidden group-hover/folder:flex items-center justify-center rounded text-surface-400 hover:text-brand-600"
              title="重命名"
              @click.stop="startRename(node.id, node.name)"
            >
              <AppIcon name="edit" size="10" />
            </span>
            <span
              class="w-4 h-4 hidden group-hover/folder:flex items-center justify-center rounded text-surface-400 hover:text-red-500"
              title="删除目录"
              @click.stop="removeFolder(node.id)"
            >
              <AppIcon name="trash" size="10" />
            </span>
          </div>

          <!-- 子目录新增输入 -->
          <div
            v-if="addingNew && addingParentId === node.id"
            class="flex items-center gap-1.5 py-1 min-w-0 pr-2"
            :style="{ paddingLeft: `${12 + (node.depth + 1) * 12}px` }"
          >
            <span class="w-3 h-3 flex-shrink-0"></span>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" />
            <input
              v-model="newFolderName"
              v-focus
              class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text"
              @keyup.enter="confirmAddFolder"
              @keyup.escape="addingNew = false"
              @blur="confirmAddFolder"
            />
          </div>
        </template>

        <!-- 根级新增目录输入 -->
        <div
          v-if="addingNew && !addingParentId"
          class="flex items-center gap-1.5 py-1 min-w-0 pr-2"
          style="padding-left: 16px"
        >
          <span class="w-3 h-3 flex-shrink-0"></span>
          <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" />
          <input
            v-model="newFolderName"
            v-focus
            class="min-w-0 flex-1 border border-brand-300 rounded px-2 py-0.5 text-[11px] outline-none select-text"
            @keyup.enter="confirmAddFolder"
            @keyup.escape="addingNew = false"
            @blur="confirmAddFolder"
          />
        </div>

        <!-- 分隔 + 未分类 -->
        <div class="mt-2 pt-1.5 border-t border-surface-100">
          <button
            class="flex items-center gap-1.5 w-full py-1.5 text-xs text-left transition-colors rounded-lg relative overflow-hidden"
            :style="{ paddingLeft: `${12}px` }"
            :class="[
              selected === 'none'
                ? 'bg-blue-50/70 text-brand-700 font-medium'
                : 'text-surface-600 hover:bg-surface-100'
            ]"
            @click="selectNode('none')"
          >
            <div
              v-if="selected === 'none'"
              class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"
            ></div>
            <AppIcon name="folder" size="13" class="flex-shrink-0 text-surface-400" />
            未分类
          </button>
        </div>
      </div>
    </aside>

    <!-- ═══ 右侧卡片墙 ═══ -->
    <main class="flex-1 flex flex-col overflow-hidden min-w-0">
      <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider"
          >演示材料</span
        >
        <button
          class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"
          @click="openCreate"
        >
          <AppIcon name="plus" size="10" stroke="2.5" />
          新建演示材料
        </button>
      </div>

      <div class="flex-1 overflow-y-auto p-2.5 scrollbar-thin">
        <!-- 空态 -->
        <div
          v-if="list.length === 0"
          class="h-full flex flex-col items-center justify-center text-center"
        >
          <AppIcon name="monitor" size="34" class="text-surface-200 mb-3" />
          <p class="text-xs text-surface-400 mb-3">
            暂无演示材料，点击「新建演示材料」与 AI 沟通策划
          </p>
          <button
            class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"
            @click="openCreate"
          >
            <AppIcon name="plus" size="10" stroke="2.5" />
            新建演示材料
          </button>
        </div>

        <!-- 卡片网格 -->
        <div
          v-else
          class="grid gap-2.5"
          style="grid-template-columns: repeat(auto-fill, minmax(216px, 1fr))"
        >
          <article
            v-for="p in list"
            :key="p.id"
            class="group relative bg-white border rounded-lg shadow-sm cursor-pointer transition-all duration-200 overflow-hidden select-none"
            :class="
              isDragging && dragItem?.id === p.id
                ? 'opacity-40'
                : 'border-surface-100 hover:border-surface-300 hover:shadow-md'
            "
            draggable="true"
            @dragstart="onDragStart($event, p)"
            @dragend="isDragging = false"
            @click="openPresentation(p)"
          >
            <!-- 缩略图 -->
            <div
              class="relative aspect-video bg-surface-100 overflow-hidden border-b border-surface-50"
            >
              <template v-if="p.slides.length > 0">
                <PresentationFrame :slide="p.slides[0]" :theme="p.theme" />
              </template>
              <div
                v-else
                class="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-gradient-to-br from-surface-50 to-surface-100"
              >
                <svg
                  class="w-6 h-6 text-surface-300"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  stroke-width="1.5"
                >
                  <path d="M8 10h8m-8 4h5m9-2A9 9 0 113 12a9 9 0 0118 0z" />
                </svg>
                <span class="text-[9px] text-surface-400">沟通策划中</span>
              </div>

              <!-- 状态徽标 -->
              <span
                v-if="p.generating"
                class="absolute top-1.5 left-1.5 px-1.5 py-0.5 rounded text-[9px] font-medium bg-blue-500/90 text-white animate-pulse"
                >生成中…</span
              >
              <span
                v-else-if="p.status === 'generated'"
                class="absolute top-1.5 left-1.5 px-1.5 py-0.5 rounded text-[9px] font-medium bg-emerald-500/90 text-white"
                >{{ p.slides.length }} 页</span
              >
              <span
                v-else
                class="absolute top-1.5 left-1.5 px-1.5 py-0.5 rounded text-[9px] font-medium bg-slate-500/80 text-white"
                >草稿</span
              >

              <!-- hover 操作 -->
              <div
                class="absolute top-1.5 right-1.5 flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity"
                @click.stop
              >
                <button
                  class="w-6 h-6 flex items-center justify-center rounded-md bg-black/35 backdrop-blur-sm text-white hover:bg-brand-500 transition-colors"
                  title="移动到…"
                  @click="openMove(p)"
                >
                  <AppIcon name="folder-open" size="11" />
                </button>
                <button
                  class="w-6 h-6 flex items-center justify-center rounded-md bg-black/35 backdrop-blur-sm text-white hover:bg-red-500 transition-colors"
                  title="删除"
                  @click="removePresentation(p)"
                >
                  <AppIcon name="trash" size="11" />
                </button>
              </div>
            </div>

            <!-- 信息区 -->
            <div class="px-2.5 py-2">
              <p class="text-[12px] font-medium text-surface-800 truncate" :title="p.title">
                {{ p.title }}
              </p>
              <p class="mt-0.5 text-[9px] text-surface-300">
                {{ fmtTime(p.updatedAt) }}更新
                <template v-if="p.status === 'generated'"> · {{ p.slides.length }}页</template>
                <template v-else>
                  · 沟通{{ p.discussion.filter((m) => m.role === 'user').length }}轮</template
                >
              </p>
            </div>
          </article>
        </div>
      </div>
    </main>
  </div>

  <!-- ═══ 新建演示弹窗 ═══ -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div
        v-if="createOpen"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm"
      >
        <div
          class="bg-white rounded-2xl w-[380px] shadow-lg border border-surface-100 overflow-hidden"
        >
          <div class="flex items-center justify-between px-5 py-4 border-b border-surface-100">
            <h3 class="text-[13px] font-semibold text-surface-900">新建演示材料</h3>
            <button
              class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-brand-600 transition-colors"
              @click="createOpen = false"
            >
              ✕
            </button>
          </div>
          <div class="px-5 py-4 space-y-3.5">
            <div>
              <label class="block text-[11px] text-surface-500 mb-1.5">标题</label>
              <input
                v-model="createTitle"
                autofocus
                class="w-full border border-surface-200 rounded-lg px-3 py-2 text-[12.5px] outline-none focus:border-brand-300 focus:ring-2 focus:ring-brand-100 transition-all"
                placeholder="例如：智慧园区方案客户交流"
                @keyup.enter="submitCreate"
              />
            </div>
            <div>
              <label class="block text-[11px] text-surface-500 mb-1.5">保存到</label>
              <select
                v-model="createFolder"
                class="w-full border border-surface-200 rounded-lg px-3 py-2 text-[12.5px] outline-none focus:border-brand-300 bg-white"
              >
                <option v-for="opt in folderOptions" :key="opt.value" :value="opt.value">
                  {{ opt.label }}
                </option>
              </select>
            </div>
            <p class="text-[10px] text-surface-400 leading-relaxed">
              创建后将进入制作室，先和 AI 沟通策划，再一键生成幻灯片。
            </p>
          </div>
          <div class="px-5 py-3.5 bg-surface-50 flex justify-end gap-2">
            <button
              class="px-3.5 py-2 text-[12px] text-surface-600 hover:bg-surface-100 rounded-lg transition-colors"
              @click="createOpen = false"
            >
              取消
            </button>
            <button
              class="px-3.5 py-2 text-[12px] font-medium rounded-lg transition-colors"
              :class="
                createTitle.trim()
                  ? 'bg-brand-600 text-white hover:bg-brand-700'
                  : 'bg-surface-200 text-surface-400 cursor-not-allowed'
              "
              @click="submitCreate"
            >
              创建并进入制作室
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- ═══ 移动到目录弹窗 ═══ -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div
        v-if="moveTarget"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm"
      >
        <div
          class="bg-white rounded-2xl w-[340px] shadow-lg border border-surface-100 overflow-hidden"
        >
          <div class="px-5 py-4 border-b border-surface-100">
            <h3 class="text-[13px] font-semibold text-surface-900">移动到…</h3>
            <p class="mt-0.5 text-[11px] text-surface-400 truncate">{{ moveTarget.title }}</p>
          </div>
          <div class="px-5 py-4 max-h-[280px] overflow-y-auto space-y-1 scrollbar-thin">
            <label
              class="flex items-center gap-2 px-2.5 py-2 rounded-lg cursor-pointer text-[12px] text-surface-700 hover:bg-surface-50 transition-colors"
            >
              <input v-model="moveFolder" type="radio" value="none" class="accent-brand-600" />
              未分类
            </label>
            <label
              v-for="f in flatFolders"
              :key="f.id"
              class="flex items-center gap-2 px-2.5 py-2 rounded-lg cursor-pointer text-[12px] text-surface-700 hover:bg-surface-50 transition-colors"
              :style="{ paddingLeft: `${10 + f.depth * 16}px` }"
            >
              <input v-model="moveFolder" type="radio" :value="f.id" class="accent-brand-600" />
              {{ f.name }}
            </label>
          </div>
          <div class="px-5 py-3.5 bg-surface-50 flex justify-end gap-2">
            <button
              class="px-3.5 py-2 text-[12px] text-surface-600 hover:bg-surface-100 rounded-lg transition-colors"
              @click="moveTarget = null"
            >
              取消
            </button>
            <button
              class="px-3.5 py-2 text-[12px] font-medium text-white bg-brand-600 hover:bg-brand-700 rounded-lg transition-colors"
              @click="submitMove"
            >
              移动
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.scrollbar-thin::-webkit-scrollbar {
  width: 4px;
}
.scrollbar-thin::-webkit-scrollbar-thumb {
  background: #e5e5ea;
  border-radius: 4px;
}
.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.2s ease;
}
.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}
</style>
