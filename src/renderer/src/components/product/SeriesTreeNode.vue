<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { UNCATEGORIZED_NAME } from '@/stores/product'

const props = defineProps({
  node: { type: Object, required: true },
  depth: { type: Number, default: 0 },
  selectedId: { type: String, default: null },
  expandedIds: { type: Set, required: true },
})

const emit = defineEmits(['toggle', 'select', 'create-child', 'edit', 'delete', 'drop-product', 'move-series'])

const isExpanded = computed(() => props.expandedIds.has(props.node.id))
const hasChildren = computed(() => props.node.children && props.node.children.length > 0)
const isUncategorized = computed(() => props.node.name === UNCATEGORIZED_NAME && !props.node.parentId)
const canHaveChildren = computed(() => props.depth < 4)

// ─── Directory drag ─────────────────────────────────────────────────────────
function onDirDragStart(e) {
  if (isUncategorized.value) {
    e.preventDefault()
    return
  }
  e.dataTransfer.setData('text/plain', 'series:' + props.node.id)
  e.dataTransfer.effectAllowed = 'move'
  e.currentTarget.classList.add('dragging')
}

function onDirDragEnd(e) {
  e.currentTarget.classList.remove('dragging')
}

// ─── Drop target (for both product drops and directory drops) ──────────────
function onDragOver(e) {
  e.preventDefault()
  e.dataTransfer.dropEffect = 'move'
  e.currentTarget.classList.add('drag-over')
}

function onDragLeave(e) {
  e.currentTarget.classList.remove('drag-over')
}

function onDrop(e) {
  e.currentTarget.classList.remove('drag-over')
  const data = e.dataTransfer.getData('text/plain')
  if (!data) return

  if (data.startsWith('series:')) {
    // Directory drop → reparent
    const draggedId = data.slice(7)
    if (draggedId === props.node.id) return // no self-drop
    // Max depth 5 (allow reasonable nesting)
    if (props.depth >= 4) return
    // Don't allow dropping onto own descendant
    if (isDescendantOf(draggedId, props.node.id)) return
    emit('move-series', draggedId, props.node.id)
  } else {
    // Product drop
    emit('drop-product', props.node.id, data)
  }
}

/** Check if `descendantId` is a descendant of `ancestorId` (prevent circular moves) */
function isDescendantOf(targetId, currentId) {
  if (!hasChildren.value) return false
  for (const child of props.node.children) {
    if (child.id === targetId) return true
    // Recursively check deeper children
    if (child.children && child.children.length > 0) {
      if (checkDeepDescendant(child, targetId)) return true
    }
  }
  return false
}

function checkDeepDescendant(node, targetId) {
  if (!node.children) return false
  for (const child of node.children) {
    if (child.id === targetId) return true
    if (child.children && checkDeepDescendant(child, targetId)) return true
  }
  return false
}
</script>

<template>
  <div>
    <div
      :draggable="!isUncategorized"
      @click="emit('select', node.id)"
      @dragstart="onDirDragStart"
      @dragend="onDirDragEnd"
      @dragover="onDragOver"
      @dragleave="onDragLeave"
      @drop="onDrop"
      class="relative flex items-center gap-1 h-10 px-3 cursor-pointer text-sm transition-colors rounded-r-md mr-1 group"
      :class="[
        selectedId === node.id ? 'bg-blue-50/50 text-blue-600 font-medium' : 'text-slate-600 hover:bg-slate-50 hover:text-slate-900',
      ]"
      :style="{ paddingLeft: `${depth * 16 + 12}px` }"
    >
      <div v-if="selectedId === node.id" class="absolute left-0 top-1/2 -translate-y-1/2 w-0.5 h-5 bg-blue-500 rounded-full" />
      <button
        v-if="hasChildren"
        @click.stop="emit('toggle', node.id)"
        class="shrink-0 w-4 h-4 flex items-center justify-center text-slate-400 hover:text-slate-600"
      >
        <AppIcon
          name="chevron-right"
          class="w-3 h-3 transition-transform"
          :class="isExpanded ? 'rotate-90' : ''"
        />
      </button>
      <span v-else class="w-4 shrink-0"></span>
      <AppIcon name="folder" class="w-3.5 h-3.5 shrink-0" :class="selectedId === node.id ? 'text-blue-500' : 'text-slate-400'" />
      <span class="truncate ml-1.5 flex-1">{{ node.name }}</span>

      <div class="opacity-0 group-hover:opacity-100 transition-opacity shrink-0 flex items-center gap-0.5">
        <button v-if="canHaveChildren" @click.stop="emit('create-child', node.id)" class="btn-ghost !p-0.5 text-slate-400 hover:text-blue-500" title="添加子目录">
          <AppIcon name="plus" class="w-3 h-3" />
        </button>
        <button @click.stop="emit('edit', node.id)" class="btn-ghost !p-0.5 text-slate-400 hover:text-blue-500" title="编辑">
          <AppIcon name="edit" class="w-3 h-3" />
        </button>
        <button v-if="!isUncategorized" @click.stop="emit('delete', node.id)" class="btn-ghost !p-0.5 text-slate-400 hover:text-red-500" title="删除">
          <AppIcon name="trash" class="w-3 h-3" />
        </button>
      </div>
    </div>
    <template v-if="isExpanded && hasChildren">
      <SeriesTreeNode
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :depth="depth + 1"
        :selectedId="selectedId"
        :expandedIds="expandedIds"
        @toggle="emit('toggle', $event)"
        @select="emit('select', $event)"
        @create-child="emit('create-child', $event)"
        @edit="emit('edit', $event)"
        @delete="emit('delete', $event)"
        @drop-product="(dirId, prodId) => emit('drop-product', dirId, prodId)"
        @move-series="(srcId, tgtId) => emit('move-series', srcId, tgtId)"
      />
    </template>
  </div>
</template>

<style scoped>
.drag-over {
  background-color: #eff6ff;
  box-shadow: 0 0 0 1px #60a5fa;
  border-radius: 0.375rem;
}
.dragging {
  opacity: 0.5;
}
</style>
