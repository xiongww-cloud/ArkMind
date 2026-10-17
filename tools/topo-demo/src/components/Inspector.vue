<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { Graph } from '@antv/x6'
import type { SelInfo } from '../graph/sel'
import { activeTypeNames, getActivePreset } from '../graph/linkTypes'
import {
  EDGE_COLORS,
  EDGE_SHAPES,
  MARKER_NAMES,
  applyLinkType,
  isDoubleEdge,
  applyEdgeColor,
  applyEdgeLabel,
  applyEdgeShape,
  applyEdgeWidth,
  applyEndMarkers,
  applyLineStyle,
  getRecentColors,
  pushRecentColor,
  readEdgeColor,
  readEdgeLabelState,
  readEdgeShape,
  readEdgeWidth,
  readEndMarkers,
  readLineStyle,
  type EdgeShape,
  type LineStyle,
  type MarkerName
} from '../graph/setup'
import {
  LABEL_COLORS,
  LABEL_FONT_SIZES,
  NODE_STATUSES,
  NODE_STYLE_MODES,
  applyNodeLabelStyle,
  applyNodeStyleMode,
  readNodeLabelStyle,
  readNodeStatus,
  readNodeStyleMode,
  setNodeStatus,
  type NodeStatus,
  type NodeStyleMode
} from '../graph/node'

const props = defineProps<{ graph: Graph | null; sel: SelInfo }>()
const emit = defineEmits<{ (e: 'changed'): void; (e: 'action', a: 'front' | 'back'): void }>()

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

/* ---------------- 节点操作 ---------------- */

function renameNode() {
  const n = props.sel.nodes[0]
  if (!n) return
  n.setAttrByPath('label/text', nameDraft.value)
  n.setData({ ...(n.getData() || {}), label: nameDraft.value })
  emit('changed')
}

function nodeStyle(mode: NodeStyleMode) {
  applyNodeStyleMode(props.sel.nodes[0], mode)
  emit('changed')
}

function nodeStatus(s: NodeStatus) {
  setNodeStatus(props.sel.nodes[0], s)
  emit('changed')
}

function nodeFont(size: number) {
  applyNodeLabelStyle(props.sel.nodes[0], { fontSize: size })
  emit('changed')
}

function nodeColor(c: string) {
  applyNodeLabelStyle(props.sel.nodes[0], { fill: c })
  emit('changed')
}

function nodeBold() {
  applyNodeLabelStyle(props.sel.nodes[0], { fontWeight: props.sel.labelBold ? 'normal' : 'bold' })
  emit('changed')
}

/* ---------------- 连线操作 ---------------- */

function setEdgeLabel() {
  applyEdgeLabel(props.sel.edges, { text: labelDraft.value })
  emit('changed')
}

function setLabelPos(v: number) {
  applyEdgeLabel(props.sel.edges, { position: v / 100 })
  emit('changed')
}

function setPilled(on: boolean) {
  applyEdgeLabel(props.sel.edges, { pilled: on })
  emit('changed')
}

function setEdgeFontSize(size: number) {
  applyEdgeLabel(props.sel.edges, { fontSize: size })
  emit('changed')
}

function setEdgeLabelColor(c: string) {
  applyEdgeLabel(props.sel.edges, { color: c })
  emit('changed')
}

function setShape(s: EdgeShape) {
  applyEdgeShape(props.sel.edges, s)
  emit('changed')
}

function setLineStyle(s: LineStyle) {
  applyLineStyle(props.sel.edges, s)
  emit('changed')
}

function setHead(m: MarkerName) {
  applyEndMarkers(props.sel.edges, m, props.sel.tailMarker)
  emit('changed')
}

function setTail(m: MarkerName) {
  applyEndMarkers(props.sel.edges, props.sel.headMarker, m)
  emit('changed')
}

function setWidth(w: number) {
  applyEdgeWidth(props.sel.edges, w)
  emit('changed')
}

const recents = ref(getRecentColors())

function setColor(c: string) {
  applyEdgeColor(props.sel.edges, c)
  pushRecentColor(c)
  recents.value = getRecentColors()
  emit('changed')
}

function setLinkType(name: string) {
  if (!props.graph || !name) return
  applyLinkType(props.graph, props.sel.edges, name, getActivePreset(name))
  emit('changed')
}

const typeNames = activeTypeNames()
const typeOptions = computed(() => {
  const opts = [...typeNames]
  const cur = props.sel.linkType
  if (cur && !opts.includes(cur)) opts.unshift(cur)
  return opts
})

function deleteSel() {
  if (!props.graph) return
  const cells = props.graph.getSelectedCells()
  if (cells.length) props.graph.removeCells(cells)
  emit('changed')
}

const markerLabel: Record<MarkerName, string> = {
  none: '无',
  block: '方块',
  classic: '尖箭',
  diamond: '菱形',
  cross: '十字'
}

const statusOptions = computed(() => NODE_STATUSES)
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
      <div class="field">
        <label>样式</label>
        <div class="btn-group">
          <button
            v-for="m in NODE_STYLE_MODES"
            :key="m.key"
            :class="{ active: sel.styleMode === m.key }"
            @click="nodeStyle(m.key)"
          >
            {{ m.label }}
          </button>
        </div>
      </div>
      <div class="field">
        <label>状态</label>
        <select v-model="sel.status" @change="nodeStatus(sel.status)">
          <option v-for="s in statusOptions" :key="s.key" :value="s.key">{{ s.label }}</option>
        </select>
      </div>
      <div class="field row-inline">
        <div>
          <label>字号</label>
          <select :value="sel.labelFontSize" @change="nodeFont(Number(($event.target as HTMLSelectElement).value))">
            <option v-for="s in LABEL_FONT_SIZES" :key="s" :value="s">{{ s }}</option>
          </select>
        </div>
        <div>
          <label>字重</label>
          <button :class="{ active: sel.labelBold }" @click="nodeBold">B</button>
        </div>
      </div>
      <div class="field">
        <label>标签颜色</label>
        <div class="swatches">
          <span
            v-for="c in LABEL_COLORS"
            :key="c"
            class="dot"
            :class="{ picked: sel.labelColor.toLowerCase() === c.toLowerCase() }"
            :style="{ background: c }"
            @click="nodeColor(c)"
          />
        </div>
      </div>
      <div class="row row-btns">
        <button title="置于顶层" @click="emit('action', 'front')">↑ 置顶</button>
        <button title="置于底层" @click="emit('action', 'back')">↓ 置底</button>
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
      <div v-if="sel.labelText" class="field">
        <label>标签位置 {{ Math.round(sel.labelPos * 100) }}%</label>
        <input
          type="range"
          min="0"
          max="100"
          :value="Math.round(sel.labelPos * 100)"
          @change="setLabelPos(Number(($event.target as HTMLInputElement).value))"
        />
        <label class="check">
          <input type="checkbox" :checked="sel.labelPilled" @change="setPilled(($event.target as HTMLInputElement).checked)" />
          药丸底
        </label>
      </div>
      <div class="field">
        <label>链路类型</label>
        <select :value="sel.linkType" @change="setLinkType(($event.target as HTMLSelectElement).value)">
          <option v-if="!sel.linkType" value="">—（自定义/未设）</option>
          <option v-for="t in typeOptions" :key="t" :value="t">{{ t }}</option>
        </select>
      </div>
      <div class="field">
        <label>形状</label>
        <div class="btn-group wrap">
          <button
            v-for="s in EDGE_SHAPES"
            :key="s.key"
            :class="{ active: sel.edgeShape === s.key }"
            @click="setShape(s.key)"
          >
            {{ s.label }}
          </button>
        </div>
      </div>
      <div class="field">
        <label>线型</label>
        <div class="btn-group">
          <button :class="{ active: sel.lineStyle === 'solid' }" @click="setLineStyle('solid')">实线</button>
          <button :class="{ active: sel.lineStyle === 'dashed' }" @click="setLineStyle('dashed')">虚线</button>
          <button :class="{ active: sel.lineStyle === 'dotted' }" @click="setLineStyle('dotted')">点线</button>
        </div>
      </div>
      <div class="field row-inline">
        <div>
          <label>线宽</label>
          <div class="btn-group">
            <button
              v-for="w in [1, 2, 3]"
              :key="w"
              :class="{ active: sel.strokeWidth === w }"
              @click="setWidth(w)"
            >
              {{ w }}
            </button>
          </div>
        </div>
      </div>
      <div class="field row-inline">
        <div>
          <label>头部箭头</label>
          <select :value="sel.headMarker" @change="setHead(($event.target as HTMLSelectElement).value as MarkerName)">
            <option v-for="m in MARKER_NAMES" :key="m" :value="m">{{ markerLabel[m] }}</option>
          </select>
        </div>
        <div>
          <label>尾部箭头</label>
          <select :value="sel.tailMarker" @change="setTail(($event.target as HTMLSelectElement).value as MarkerName)">
            <option v-for="m in MARKER_NAMES" :key="m" :value="m">{{ markerLabel[m] }}</option>
          </select>
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
        <div v-if="recents.length" class="swatches recents">
          <span
            v-for="c in recents"
            :key="'r' + c"
            class="dot small"
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
  width: 244px;
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

.field label.check {
  display: flex;
  align-items: center;
  gap: 5px;
  margin-top: 8px;
  cursor: pointer;
}

.field input[type='range'] {
  width: 100%;
}

.field select {
  font: inherit;
  border: 1px solid #d5dae2;
  border-radius: 6px;
  padding: 4px 6px;
  background: #fff;
  outline: none;
  width: 100%;
}

.row-inline {
  display: flex;
  gap: 10px;
}

.row-inline > div {
  flex: 1;
  min-width: 0;
}

.btn-group {
  display: flex;
  gap: 6px;
}

.btn-group.wrap {
  flex-wrap: wrap;
}

.btn-group button {
  flex: 1;
  padding: 4px 0;
  min-width: 40px;
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
  flex-wrap: wrap;
}

.swatches.recents {
  margin-top: 8px;
  padding-top: 0;
}

.dot {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  cursor: pointer;
  border: 2px solid transparent;
}

.dot.small {
  width: 14px;
  height: 14px;
}

.dot.picked {
  border-color: #26282c;
}

.row {
  margin-top: 10px;
}

.row-btns {
  display: flex;
  gap: 6px;
}

.row-btns button {
  flex: 1;
}
</style>
