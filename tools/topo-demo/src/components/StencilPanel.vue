<script setup lang="ts">
import { ref, watch } from 'vue'
import type { Graph } from '@antv/x6'
import { Dnd } from '@antv/x6-plugin-dnd'
import { CATEGORIES, type IconItem } from '../graph/icons'
import { createTopoNodeMeta } from '../graph/node'

const props = defineProps<{ graph: Graph | null }>()

const keyword = ref('')
const collapsed = ref(new Set<string>())
let dnd: Dnd | null = null

watch(
  () => props.graph,
  (g) => {
    if (g) dnd = new Dnd({ target: g })
  },
  { immediate: true }
)

function toggle(name: string) {
  const s = new Set(collapsed.value)
  if (s.has(name)) s.delete(name)
  else s.add(name)
  collapsed.value = s
}

function filtered(items: IconItem[]): IconItem[] {
  const kw = keyword.value.trim().toLowerCase()
  if (!kw) return items
  return items.filter((i) => i.label.toLowerCase().includes(kw) || i.src.toLowerCase().includes(kw))
}

function startDrag(item: IconItem, e: MouseEvent) {
  if (!dnd || !props.graph) return
  e.preventDefault()
  const node = props.graph.createNode(createTopoNodeMeta(item.src, item.label))
  dnd.start(node, e)
}
</script>

<template>
  <aside class="stencil">
    <div class="stencil-head">
      <input v-model="keyword" type="text" placeholder="搜索图标…" />
    </div>
    <div class="stencil-body">
      <div v-for="cat in CATEGORIES" :key="cat.name" class="cat">
        <h4 class="cat-title" @click="toggle(cat.name)">
          <span class="arrow" :class="{ open: !collapsed.has(cat.name) }">▸</span>
          {{ cat.title }}
          <em>{{ filtered(cat.items).length }}</em>
        </h4>
        <div v-if="!collapsed.has(cat.name)" class="grid">
          <div
            v-for="item in filtered(cat.items)"
            :key="item.src"
            class="tile"
            :title="item.label"
            @mousedown="startDrag(item, $event)"
          >
            <img :src="item.src" :alt="item.label" draggable="false" />
            <span>{{ item.label }}</span>
          </div>
        </div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.stencil {
  width: 252px;
  flex-shrink: 0;
  background: #fff;
  border-right: 1px solid #e5e8ee;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.stencil-head {
  padding: 10px;
  border-bottom: 1px solid #eef0f4;
}

.stencil-body {
  flex: 1;
  overflow-y: auto;
  padding: 4px 10px 16px;
}

.cat {
  margin-top: 8px;
}

.cat-title {
  font-size: 12px;
  font-weight: 600;
  color: #5a6472;
  padding: 6px 4px;
  cursor: pointer;
  user-select: none;
  display: flex;
  align-items: center;
  gap: 4px;
}

.cat-title em {
  margin-left: auto;
  font-style: normal;
  font-size: 11px;
  color: #a8b0bd;
}

.arrow {
  display: inline-block;
  transition: transform 0.12s;
}

.arrow.open {
  transform: rotate(90deg);
}

.grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 6px;
}

.tile {
  border: 1px solid transparent;
  border-radius: 6px;
  padding: 6px 4px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  cursor: grab;
  user-select: none;
}

.tile:hover {
  border-color: #1677ff;
  background: #f0f7ff;
}

.tile:active {
  cursor: grabbing;
}

.tile img {
  width: 46px;
  height: 34px;
  object-fit: contain;
  pointer-events: none;
}

.tile span {
  font-size: 11px;
  color: #5a6472;
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
