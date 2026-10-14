<script setup lang="ts">
import { ref, watch } from 'vue'
import type { Graph } from '@antv/x6'
import type { SelInfo } from '../graph/sel'
import {
  EDGE_COLORS,
  applyEdgeColor,
  applyLineStyle,
  applyMarker,
  type EdgeLineStyle,
  type EdgeMarker
} from '../graph/setup'

const props = defineProps<{ graph: Graph | null; sel: SelInfo }>()
const emit = defineEmits<{ (e: 'changed'): void }>()

const nameDraft = ref('')
const labelDraft = ref('')

watch(
  () => [props.sel.kind, props.sel.name, props.sel.labelText],
  () => {
    nameDraft.value = props.sel.name
    labelDraft.value = props.sel.labelText
  },
  { immediate: true }
)

function renameNode() {
  const n = props.sel.nodes[0]
  if (!n) return
  n.setAttrByPath('label/text', nameDraft.value)
  n.setData({ ...(n.getData() || {}), label: nameDraft.value })
  emit('changed')
}

function setEdgeLabel() {
  props.sel.edges.forEach((edge) => {
    const text = labelDraft.value
    if (!text) {
      edge.setLabels([])
    } else {
      edge.setLabels([
        {
          position: 0.5,
          attrs: { label: { text, fontSize: 12, fill: '#4b5563' } }
        }
      ])
    }
  })
  emit('changed')
}

function setLineStyle(s: EdgeLineStyle) {
  applyLineStyle(props.sel.edges, s)
  emit('changed')
}

function setMarker(m: EdgeMarker) {
  applyMarker(props.sel.edges, m)
  emit('changed')
}

function setColor(c: string) {
  applyEdgeColor(props.sel.edges, c)
  emit('changed')
}

function deleteSel() {
  if (!props.graph) return
  const cells = props.graph.getSelectedCells()
  if (cells.length) props.graph.removeCells(cells)
  emit('changed')
}
</script>

<template>
  <aside class="inspector">
    <h3>属性</h3>

    <div v-if="sel.kind === 'none'" class="tip">
      选中节点或连线后编辑属性。<br />
      双击节点/连线可快速修改名称。
    </div>

    <template v-else-if="sel.kind === 'node'">
      <div class="field">
        <label>名称</label>
        <input v-model="nameDraft" type="text" @keyup.enter="renameNode" @change="renameNode" />
      </div>
      <div class="row">
        <button @click="deleteSel">删除节点</button>
      </div>
    </template>

    <template v-else-if="sel.kind === 'edge'">
      <div class="field">
        <label>标签</label>
        <input v-model="labelDraft" type="text" @keyup.enter="setEdgeLabel" @change="setEdgeLabel" />
      </div>
      <div class="field">
        <label>线型</label>
        <div class="btn-group">
          <button :class="{ active: sel.lineStyle === 'solid' }" @click="setLineStyle('solid')">实线</button>
          <button :class="{ active: sel.lineStyle === 'dashed' }" @click="setLineStyle('dashed')">虚线</button>
          <button :class="{ active: sel.lineStyle === 'dotted' }" @click="setLineStyle('dotted')">点线</button>
        </div>
      </div>
      <div class="field">
        <label>箭头</label>
        <div class="btn-group">
          <button :class="{ active: sel.marker === 'none' }" @click="setMarker('none')">无</button>
          <button :class="{ active: sel.marker === 'target' }" @click="setMarker('target')">单向</button>
          <button :class="{ active: sel.marker === 'both' }" @click="setMarker('both')">双向</button>
        </div>
      </div>
      <div class="field">
        <label>颜色</label>
        <div class="swatches">
          <span
            v-for="c in EDGE_COLORS"
            :key="c"
            class="dot"
            :class="{ picked: sel.color.toLowerCase() === c.toLowerCase() }"
            :style="{ background: c }"
            @click="setColor(c)"
          />
        </div>
      </div>
      <div class="row">
        <button @click="deleteSel">删除连线</button>
      </div>
    </template>

    <template v-else>
      <div class="tip">已选中 {{ sel.nodes.length + sel.edges.length }} 个元素</div>
      <div class="row">
        <button @click="deleteSel">删除所选</button>
      </div>
    </template>
  </aside>
</template>

<style scoped>
.inspector {
  width: 232px;
  flex-shrink: 0;
  background: #fff;
  border-left: 1px solid #e5e8ee;
  padding: 14px;
  overflow-y: auto;
}

h3 {
  font-size: 13px;
  margin-bottom: 12px;
  color: #26282c;
}

.tip {
  color: #8a94a6;
  line-height: 1.7;
}

.field {
  margin-bottom: 12px;
}

.field label {
  display: block;
  font-size: 12px;
  color: #5a6472;
  margin-bottom: 4px;
}

.btn-group {
  display: flex;
  gap: 6px;
}

.btn-group button {
  flex: 1;
  padding: 4px 0;
}

button.active {
  border-color: #1677ff;
  color: #1677ff;
  background: #f0f7ff;
}

.swatches {
  display: flex;
  gap: 8px;
  padding-top: 2px;
}

.dot {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  cursor: pointer;
  border: 2px solid transparent;
}

.dot.picked {
  border-color: #26282c;
}

.row {
  margin-top: 10px;
}
</style>
