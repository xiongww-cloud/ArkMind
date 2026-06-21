<script setup lang="ts">
import { ref, computed } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'

const props = defineProps({
  node: { type: Object, required: true },
  depth: { type: Number, default: 0 },
  products: { type: Array, default: () => [] },
  selectedIds: { type: Array, default: () => [] },
})

const emit = defineEmits(['toggle-product', 'toggle-series'])

// Products directly under this series
const nodeProducts = computed(() => {
  const products = Array.isArray(props.products) ? props.products : []
  return products.filter(p => p.seriesId === props.node.id)
})

// Collect all product IDs under this node and its descendants
function collectAllIds(node) {
  const products = Array.isArray(props.products) ? props.products : []
  const ids = products.filter(p => p.seriesId === node.id).map(p => p.id)
  if (node.children) {
    for (const child of node.children) {
      ids.push(...collectAllIds(child))
    }
  }
  return ids
}

const allIds = computed(() => collectAllIds(props.node))

const checkedState = computed(() => {
  const total = allIds.value.length
  if (total === 0) return 'none'
  const selected = allIds.value.filter(id => props.selectedIds.includes(id)).length
  if (selected === 0) return 'none'
  if (selected === total) return 'all'
  return 'partial'
})

const hasChildren = computed(() =>
  (nodeProducts.value.length > 0) || (props.node.children?.length > 0)
)

const isExpanded = ref(false)

function toggleExpand() {
  isExpanded.value = !isExpanded.value
}
</script>

<template>
  <div>
    <!-- Series node -->
    <div
      class="flex items-center gap-1 cursor-pointer hover:bg-slate-50 transition-colors select-none"
      :style="{ paddingLeft: `${depth * 16 + 8}px`, paddingRight: '8px', paddingTop: '5px', paddingBottom: '5px' }"
    >
      <!-- Chevron toggle -->
      <span
        v-if="hasChildren"
        @click.stop="toggleExpand"
        class="shrink-0 w-3.5 h-3.5 flex items-center justify-center text-slate-400 hover:text-slate-600 transition-transform"
        :class="isExpanded ? 'rotate-90' : ''"
      >
        <AppIcon name="chevron-right" class="w-3 h-3" />
      </span>
      <span v-else class="w-3.5 shrink-0" />

      <input
        type="checkbox"
        :checked="checkedState === 'all'"
        :indeterminate.prop="checkedState === 'partial'"
        @change.stop="emit('toggle-series', node)"
        class="w-3.5 h-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
      />
      <AppIcon name="folder" class="w-3.5 h-3.5 text-slate-400 shrink-0" />
      <span class="text-xs text-slate-700 font-medium truncate">{{ node.name }}</span>
      <span class="text-[10px] text-slate-400">({{ allIds.length }})</span>
    </div>

    <!-- Products under this series (only when expanded) -->
    <template v-if="isExpanded">
      <label
        v-for="p in nodeProducts"
        :key="p.id"
        class="flex items-center gap-1.5 cursor-pointer hover:bg-slate-50 transition-colors select-none"
        :style="{ paddingLeft: `${(depth + 1) * 16 + 8}px`, paddingRight: '8px', paddingTop: '4px', paddingBottom: '4px' }"
      >
        <span class="w-3.5 shrink-0" />
        <input
          type="checkbox"
          :checked="selectedIds.includes(p.id)"
          @change="emit('toggle-product', p.id)"
          class="w-3.5 h-3.5 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
        />
        <AppIcon name="server" class="w-3 h-3 text-slate-300 shrink-0" />
        <span class="text-xs text-slate-600 truncate">{{ p.name }}</span>
      </label>

      <!-- Children series -->
      <MaterialProductTreeNode
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :depth="depth + 1"
        :products="products"
        :selectedIds="selectedIds"
        @toggle-product="emit('toggle-product', $event)"
        @toggle-series="emit('toggle-series', $event)"
      />
    </template>
  </div>
</template>
