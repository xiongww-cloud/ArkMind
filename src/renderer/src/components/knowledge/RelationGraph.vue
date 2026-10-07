<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useRouter } from 'vue-router'
import ForceGraph from 'force-graph'
import { kbGraphData } from '@/api/db-client'
import { knowledgeTypeLabel, knowledgeTypeColor, KNOWLEDGE_TYPE_LABEL } from '@/utils/knowledgeTypes'
import { useNavigation } from '@/composables/useNavigation'
import { toast } from '@/composables/useToast'
import { invoke } from '@tauri-apps/api/core'
import AppIcon from '@/components/common/AppIcon.vue'

interface GNode {
  id: string
  bizId: string
  type: string
  label: string
  degree: number
  x?: number
  y?: number
}
interface GLink {
  source: string | GNode
  target: string | GNode
  relation: string
}

const router = useRouter()
const { navigateTo } = useNavigation()

const mountRef = ref<HTMLElement | null>(null)
const loading = ref(false)
const loadError = ref('')
const empty = ref(false)
const nodeCount = ref(0)
const linkCount = ref(0)
const orphanFiltered = ref(0)
const truncated = ref(false)
const statsOpen = ref(false)

const focusId = ref<string | null>(null)
const selected = ref<GNode | null>(null)
const canvasWrapRef = ref<HTMLElement | null>(null)
// 选中节点的屏幕坐标（每帧绘制同步，供锚定弹窗跟随）
const popupPos = ref<{ x: number; y: number; sr: number } | null>(null)

const POPUP_W = 230
const popupStyle = computed<Record<string, string>>(() => {
  const p = popupPos.value
  if (!p) return {}
  const W = canvasWrapRef.value?.clientWidth || 800
  const half = POPUP_W / 2 + 6
  const x = Math.min(Math.max(half, p.x), Math.max(half, W - half))
  const above = p.y - p.sr - 140 > 0
  return {
    left: `${x}px`,
    top: `${above ? p.y - p.sr - 10 : p.y + p.sr + 10}px`,
    transform: above ? 'translate(-50%, -100%)' : 'translate(-50%, 0)',
    '--kg-arrow-left': `${p.x - x + POPUP_W / 2}px`,
  } as Record<string, string>
})

// 全量数据（节点对象跨刷新复用，保持坐标稳定）
const nodeById = new Map<string, GNode>()
let rawLinks: { s: string; t: string; rel: string }[] = []
let adjacency = new Map<string, Set<string>>()

// 类型可见性 chips
const typeOrder = Object.keys(KNOWLEDGE_TYPE_LABEL)
const visibleTypes = ref<Set<string>>(new Set())
const typeCounts = ref<Map<string, number>>(new Map())
const chips = computed(() =>
  [...typeCounts.value.entries()]
    .sort((a, b) => typeOrder.indexOf(a[0]) - typeOrder.indexOf(b[0]))
    .map(([t, n]) => ({ type: t, count: n, on: visibleTypes.value.has(t) }))
)

let fg: ForceGraph<GNode, GLink> | null = null
let ro: ResizeObserver | null = null
let firstFitDone = false

function toggleType(t: string) {
  const set = new Set(visibleTypes.value)
  if (set.has(t)) set.delete(t)
  else set.add(t)
  visibleTypes.value = set
  applyView()
}

function currentView(): { nodes: GNode[]; links: GLink[] } {
  let ids: Iterable<string> = nodeById.keys()
  if (focusId.value && nodeById.has(focusId.value)) {
    const ego = new Set<string>([focusId.value])
    for (const a of adjacency.get(focusId.value) || []) {
      ego.add(a)
      for (const b of adjacency.get(a) || []) ego.add(b)
    }
    ids = ego
  }
  const nodes: GNode[] = []
  for (const id of ids) {
    const n = nodeById.get(id)
    if (n && visibleTypes.value.has(n.type)) nodes.push(n)
  }
  const kept = new Set(nodes.map((n) => n.id))
  const links: GLink[] = []
  for (const l of rawLinks) {
    if (kept.has(l.s) && kept.has(l.t)) links.push({ source: l.s, target: l.t, relation: l.rel })
  }
  return { nodes, links }
}

function applyView(fit = false) {
  if (!fg) return
  const { nodes, links } = currentView()
  nodeCount.value = nodes.length
  linkCount.value = links.length
  fg.graphData({ nodes, links })
  if (fit || !firstFitDone) {
    firstFitDone = true
    requestAnimationFrame(() => requestAnimationFrame(() => fg?.zoomToFit(400, 50)))
  }
}

function wantsLabel(n: GNode): boolean {
  return n.degree >= 6 || n.id === selected.value?.id || n.id === focusId.value
}

async function load() {
  if (loading.value) return
  loading.value = true
  loadError.value = ''
  try {
    const r = await kbGraphData()
    if (!r?.success) throw new Error('图谱数据加载失败')
    const seen = new Set<string>()
    for (const n of r.nodes || []) {
      seen.add(n.id)
      const prev = nodeById.get(n.id)
      if (prev) Object.assign(prev, n)
      else nodeById.set(n.id, { ...n })
    }
    for (const id of [...nodeById.keys()]) if (!seen.has(id)) nodeById.delete(id)
    rawLinks = (r.links || []).map((l) => ({ s: String(l.source), t: String(l.target), rel: l.relation || '' }))
    // ★ 双向邻接表：聚焦 2 跳邻域的数据基础（缺了它 ego 永远只剩选中节点）
    adjacency = new Map()
    for (const l of rawLinks) {
      if (!adjacency.has(l.s)) adjacency.set(l.s, new Set())
      if (!adjacency.has(l.t)) adjacency.set(l.t, new Set())
      adjacency.get(l.s)!.add(l.t)
      adjacency.get(l.t)!.add(l.s)
    }
    const counts = new Map<string, number>()
    for (const n of nodeById.values()) counts.set(n.type, (counts.get(n.type) || 0) + 1)
    typeCounts.value = counts
    if (!visibleTypes.value.size) visibleTypes.value = new Set(counts.keys())
    else visibleTypes.value = new Set([...visibleTypes.value].filter((t) => counts.has(t)))
    orphanFiltered.value = r.stats?.orphanFiltered || 0
    truncated.value = !!(r.stats?.nodesTruncated || r.stats?.edgesTruncated)
    empty.value = nodeById.size === 0
    if (focusId.value && !nodeById.has(focusId.value)) { focusId.value = null; focusLabel.value = ''; selected.value = null }
    applyView(!firstFitDone)
  } catch (e: any) {
    loadError.value = String(e?.message || e)
  } finally {
    loading.value = false
  }
}

// 进入聚焦前快照缩放态，退出时精确恢复——缩放比例永远由用户滚轮/按钮控制，交互不被程序抢走
let savedZoom: { k: number; cx: number; cy: number } | null = null
const focusLabel = ref('')

function exitFocus() {
  focusId.value = null
  focusLabel.value = ''
  applyView(false)
  if (savedZoom && fg) {
    fg.zoom(savedZoom.k, 0)
    fg.centerAt(savedZoom.cx, savedZoom.cy, 0)
  } else {
    applyView(true)
  }
  savedZoom = null
}

// 弹窗内显式切换 2 跳邻域：单击节点不再自动重排全图，聚焦必须由用户触发
function toggleFocus() {
  if (!selected.value) return
  if (focusId.value === selected.value.id) { exitFocus(); return }
  if (!focusId.value && fg) savedZoom = { k: fg.zoom(), cx: fg.centerAt().x, cy: fg.centerAt().y }
  focusId.value = selected.value.id
  focusLabel.value = selected.value.label
  applyView(true)
  // 进入聚焦即收起弹窗：聚焦态里点节点才再次弹卡，退出聚焦走右上「看全局」
  selected.value = null
}

watch(selected, (n) => { if (!n) popupPos.value = null })

function openNode(n: GNode) {
  const id = n.bizId
  switch (n.type) {
    case 'PROJECT': navigateTo('project', id); break
    case 'PRODUCT': navigateTo('product', id); break
    case 'NOTE': navigateTo('note', id); break
    case 'TODO': navigateTo('todo', id); break
    case 'CONTACT': router.push({ path: '/contacts', query: { contactId: id } }); break
    case 'CHANNEL': router.push({ path: '/contacts', query: { tab: 'channels', id } }); break
    case 'VENDOR': router.push({ path: '/contacts', query: { tab: 'vendors', id } }); break
    case 'CUSTOMER': router.push({ path: '/contacts', query: { tab: 'customers', id } }); break
    case 'EMAIL': router.push({ path: '/email', query: { id } }); break
    case 'FILE': invoke('file_open', { versionId: id }).catch(() => toast({ message: '打开文件失败', type: 'error' })); break
    default: toast({ message: '该类型暂不支持跳转', type: 'info' })
  }
}

function onEsc(e: KeyboardEvent) {
  if (e.key === 'Escape' && selected.value) selected.value = null
}

onMounted(async () => {
  window.addEventListener('keydown', onEsc)
  await nextTick()
  const el = mountRef.value
  if (!el) return
  el.addEventListener('dblclick', onCanvasDblClick)
  fg = new ForceGraph<GNode, GLink>(el)
    .autoPauseRedraw(false)
    .backgroundColor('#ffffff')
    .nodeVal((n) => nodeValOf(n))
    .nodeColor((n) => knowledgeTypeColor(n.type))
    .nodeLabel((n) => `${n.label}｜${knowledgeTypeLabel(n.type)}｜${n.degree} 条关联`)
    .nodeCanvasObjectMode(() => 'after')
    .nodeCanvasObject((node, ctx, scale) => {
      const r = Math.sqrt(Math.max(0, nodeValOf(node))) * 4
      const isSel = node.id === selected.value?.id
      if (isSel) {
        ctx.beginPath()
        ctx.arc(node.x || 0, node.y || 0, r + 2.5 / scale, 0, 2 * Math.PI)
        ctx.strokeStyle = '#1d4ed8'
        ctx.lineWidth = 2 / scale
        ctx.stroke()
        const scr = fg?.graph2ScreenCoords(node.x || 0, node.y || 0)
        if (scr) {
          const sr = Math.max(6, r * scale)
          const prev = popupPos.value
          if (!prev || Math.abs(prev.x - scr.x) > 0.5 || Math.abs(prev.y - scr.y) > 0.5 || Math.abs(prev.sr - sr) > 0.5) popupPos.value = { x: scr.x, y: scr.y, sr }
        }
      }
      if (wantsLabel(node) || scale > 2.4) {
        const fs = 11 / scale
        ctx.font = `${fs}px system-ui, sans-serif`
        ctx.textAlign = 'center'
        ctx.textBaseline = 'bottom'
        const text = node.label.length > 14 ? node.label.slice(0, 13) + '…' : node.label
        ctx.strokeStyle = 'rgba(255,255,255,0.9)'
        ctx.lineWidth = 3 / scale
        ctx.strokeText(text, node.x || 0, (node.y || 0) - r - 1 / scale)
        ctx.fillStyle = isSel ? '#1d4ed8' : '#334155'
        ctx.fillText(text, node.x || 0, (node.y || 0) - r - 1 / scale)
      }
    })
    .linkColor(() => 'rgba(148,163,184,0.35)')
    .linkWidth(0.75)
    .onNodeClick((n) => { selected.value = n })
    .onBackgroundClick(() => {
      // 点空白只收起详情弹窗；聚焦态不退出聚焦（退出用右上「看全局」），缩放视图完全不动
      selected.value = null
    })
    .width(el.clientWidth)
    .height(el.clientHeight)
  ro = new ResizeObserver(() => {
    if (!fg || !mountRef.value) return
    fg.width(mountRef.value.clientWidth).height(mountRef.value.clientHeight)
  })
  ro.observe(el)
  await load()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onEsc)
  mountRef.value?.removeEventListener('dblclick', onCanvasDblClick)
  ro?.disconnect()
  ro = null
  fg?._destructor()
  fg = null
})

// 与库默认 nodeRelSize=4 对齐：radius = sqrt(val) * 4；度数→面积近似线性
function nodeValOf(n: GNode): number { return 1.5 + Math.min(30, n.degree) }

// 此版本 force-graph 无 onNodeDoubleClick API：双击命中检测手动实现直跳
function onCanvasDblClick(e: MouseEvent) {
  if (!fg || !mountRef.value) return
  const rect = mountRef.value.getBoundingClientRect()
  const g = fg.screen2GraphCoords(e.clientX - rect.left, e.clientY - rect.top)
  let best: GNode | null = null
  let bestD = Infinity
  for (const n of fg.graphData().nodes) {
    const r = Math.sqrt(Math.max(0, nodeValOf(n))) * 4
    const d = Math.hypot((n.x || 0) - g.x, (n.y || 0) - g.y)
    if (d <= r + 3 && d < bestD) { best = n; bestD = d }
  }
  if (best) { selected.value = best; openNode(best) }
}

defineExpose({ reload: load })
</script>

<template>
  <div class="flex flex-col h-full min-h-0 relative">
    <!-- 工具条：类型筛选 chips + 状态 -->
    <div class="flex items-center gap-1.5 flex-wrap px-4 pt-3 pb-2 border-b border-surface-100 flex-shrink-0">
      <button
        v-for="c in chips" :key="c.type"
        @click="toggleType(c.type)"
        class="flex items-center gap-1 px-1.5 py-0.5 rounded-md border text-[10px] transition-colors select-none"
        :class="c.on ? 'border-surface-200 bg-white text-surface-600' : 'border-surface-100 bg-surface-50 text-surface-300'"
      >
        <span class="w-2 h-2 rounded-full flex-shrink-0" :style="{ background: c.on ? knowledgeTypeColor(c.type) : '#cbd5e1' }"></span>
        {{ knowledgeTypeLabel(c.type) }}<span class="tabular-nums">{{ c.count }}</span>
      </button>
      <div class="ml-auto flex items-center gap-2 text-[10px] text-surface-400">
        <span v-if="focusId" class="text-brand-600">聚焦「{{ focusLabel }}」2 跳邻域</span>
        <span>{{ nodeCount }} 节点 · {{ linkCount }} 关系</span>
        <span v-if="orphanFiltered" class="text-surface-300">（{{ orphanFiltered }} 个无关联对象未入图）</span>
        <span v-if="truncated" class="text-amber-500">已按连接度截断</span>
        <button @click="statsOpen = !statsOpen" class="w-4 h-4 flex items-center justify-center rounded-full text-surface-400 hover:text-surface-600 hover:bg-surface-100" v-tooltip="'口径说明'">?</button>
      </div>
    </div>

    <!-- 画布 -->
    <div ref="canvasWrapRef" class="flex-1 relative min-h-0">
      <!-- 聚焦态退出主操作：悬浮画布右上，一眼可见 -->
      <button v-if="focusId" @click="exitFocus" class="absolute top-2 right-2 z-20 btn-primary text-[11px] shadow-lg"><AppIcon name="refresh" size="11" />看全局</button>
      <div ref="mountRef" class="absolute inset-0" v-show="!loading && !loadError && !empty"></div>

      <!-- 选中对象锚定弹窗：跟随节点屏幕坐标，贴边自动翻转 -->
      <div v-if="selected && popupPos && !loadError" class="kg-popup absolute z-10 bg-white rounded-xl border border-surface-200 shadow-lg p-3 text-[11px]" :style="[{ width: POPUP_W + 'px' }, popupStyle]">
        <span class="kg-popup-arrow" :class="popupStyle.transform === 'translate(-50%, -100%)' ? 'is-bottom' : 'is-top'"></span>
        <div class="flex items-start justify-between gap-2">
          <div class="min-w-0">
            <p class="font-medium text-surface-800 break-words leading-snug">{{ selected.label }}</p>
            <p class="text-[10px] text-surface-400 mt-0.5">
              <span class="inline-block w-2 h-2 rounded-full mr-1 align-middle" :style="{ background: knowledgeTypeColor(selected.type) }"></span>
              {{ knowledgeTypeLabel(selected.type) }} · {{ selected.degree }} 条关联
            </p>
          </div>
          <button @click="selected = null" class="text-surface-300 hover:text-surface-500 flex-shrink-0"><AppIcon name="x" size="12" /></button>
        </div>
        <div class="mt-2 flex items-center gap-1.5">
          <button @click="openNode(selected)" class="flex-1 btn-primary text-[11px] !py-1"><AppIcon name="arrow-up-right" size="11" />打开</button>
          <button @click="toggleFocus" class="flex-1 btn-secondary text-[11px] !py-1">{{ focusId === selected.id ? '退出聚焦' : '聚焦邻域' }}</button>
        </div>
        <p class="text-[9px] text-surface-300 mt-1.5">双击节点可直接打开</p>
      </div>

      <!-- 状态卡 -->
      <div v-if="loading" class="absolute inset-0 flex items-center justify-center text-[11px] text-surface-400">
        <svg class="animate-spin w-4 h-4 mr-2 text-brand-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" style="margin-right:6px"><path d="M21 12a9 9 0 11-6.219-8.56"/></svg>
        正在构建关系图谱…
      </div>
      <div v-else-if="loadError" class="absolute inset-0 flex flex-col items-center justify-center gap-2">
        <p class="text-[11px] text-red-400">图谱加载失败：{{ loadError }}</p>
        <button @click="load" class="btn-secondary text-xs">重试</button>
      </div>
      <div v-else-if="empty" class="absolute inset-0 flex flex-col items-center justify-center gap-1.5 text-center px-8">
        <p class="text-[12px] text-surface-500">还没有对象关联</p>
        <p class="text-[11px] text-surface-400 leading-relaxed">把文件关联到项目/产品、把联系人归属到合作伙伴后，这里会生长出知识关系网络。</p>
      </div>

      <!-- 口径说明浮层 -->
      <div v-if="statsOpen" class="absolute bottom-2 left-2 right-2 md:right-auto md:w-[340px] bg-gray-800 text-white text-[10px] leading-relaxed rounded-lg shadow-xl p-3" @click="statsOpen = false">
        节点 = 存在业务关联的知识对象（孤立对象不入图）；大小 ∝ 关联数量；颜色 = 对象类型。
        边来自项目↔文件/待办/笔记、联系人↔合作伙伴/项目、附件↔邮件/产品 等业务关系。
        点击节点弹出详情卡（双击直接打开对象），卡内可聚焦其 2 跳邻域；聚焦态点空白仅收起卡片，点其他节点可换焦，右上「看全局」退出聚焦。
      </div>
    </div>
  </div>
</template>

<style scoped>
:deep(canvas) { display: block; }
.kg-popup-arrow {
  position: absolute;
  left: var(--kg-arrow-left, 50%);
  width: 10px;
  height: 10px;
  background: #fff;
  border: 1px solid #e2e8f0;
  transform: translateX(-50%) rotate(45deg);
}
.kg-popup-arrow.is-bottom { bottom: -6px; border-top: none; border-left: none; }
.kg-popup-arrow.is-top { top: -6px; border-bottom: none; border-right: none; }
</style>
