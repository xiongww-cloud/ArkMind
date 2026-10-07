<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, nextTick } from 'vue'
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
    adjacency = new Map()
    const counts = new Map<string, number>()
    for (const n of nodeById.values()) counts.set(n.type, (counts.get(n.type) || 0) + 1)
    typeCounts.value = counts
    if (!visibleTypes.value.size) visibleTypes.value = new Set(counts.keys())
    else visibleTypes.value = new Set([...visibleTypes.value].filter((t) => counts.has(t)))
    orphanFiltered.value = r.stats?.orphanFiltered || 0
    truncated.value = !!(r.stats?.nodesTruncated || r.stats?.edgesTruncated)
    empty.value = nodeById.size === 0
    if (focusId.value && !nodeById.has(focusId.value)) { focusId.value = null; selected.value = null }
    applyView(!firstFitDone)
  } catch (e: any) {
    loadError.value = String(e?.message || e)
  } finally {
    loading.value = false
  }
}

function exitFocus() {
  focusId.value = null
  selected.value = null
  applyView(true)
}

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

onMounted(async () => {
  await nextTick()
  const el = mountRef.value
  if (!el) return
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
    .onNodeClick((n) => { selected.value = n; focusId.value = n.id; applyView(true) })
    .onBackgroundClick(() => { selected.value = null; focusId.value = null; applyView(true) })
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
  ro?.disconnect()
  ro = null
  fg?._destructor()
  fg = null
})

// 与库默认 nodeRelSize=4 对齐：radius = sqrt(val) * 4；度数→面积近似线性
function nodeValOf(n: GNode): number { return 1.5 + Math.min(30, n.degree) }

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
        <span v-if="focusId" class="text-brand-600">聚焦「{{ selected?.label }}」2 跳邻域</span>
        <button v-if="focusId" @click="exitFocus" class="btn-secondary text-[10px] !px-1.5 !py-0.5">看全局</button>
        <span>{{ nodeCount }} 节点 · {{ linkCount }} 关系</span>
        <span v-if="orphanFiltered" class="text-surface-300">（{{ orphanFiltered }} 个无关联对象未入图）</span>
        <span v-if="truncated" class="text-amber-500">已按连接度截断</span>
        <button @click="statsOpen = !statsOpen" class="w-4 h-4 flex items-center justify-center rounded-full text-surface-400 hover:text-surface-600 hover:bg-surface-100" v-tooltip="'口径说明'">?</button>
      </div>
    </div>

    <!-- 画布 -->
    <div class="flex-1 relative min-h-0">
      <div ref="mountRef" class="absolute inset-0" v-show="!loading && !loadError && !empty"></div>

      <!-- 选中对象浮层 -->
      <div v-if="selected && !loadError" class="absolute top-2 right-2 w-[230px] bg-white rounded-xl border border-surface-200 shadow-lg p-3 text-[11px]">
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
        <button @click="openNode(selected)" class="mt-2 w-full btn-primary text-[11px] !py-1"><AppIcon name="arrow-up-right" size="11" />打开该对象</button>
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
        点击节点聚焦其邻域，再次点击空白处返回全局。
      </div>
    </div>
  </div>
</template>

<style scoped>
:deep(canvas) { display: block; }
</style>
