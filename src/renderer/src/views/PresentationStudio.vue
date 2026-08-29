<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { usePresentationsStore } from '@/stores/presentations'
import { usePresentationHistoryStore } from '@/stores/presentationHistory'
import PresentationFrame from '@/components/presentation/PresentationFrame.vue'
import PresentationPlayer from '@/components/presentation/PresentationPlayer.vue'
import SlideViewer from '@/components/presentation/SlideViewer.vue'
import PresentationChatPanel from '@/components/presentation/PresentationChatPanel.vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { toast } from '@/composables/useToast'
import { isTauri } from '@/utils/presentationAi'

const route = useRoute()
const router = useRouter()
const store = usePresentationsStore()
const history = usePresentationHistoryStore()

// ─── 当前演示 ────────────────────────────────────────────────────
const pid = computed(() => String(route.params.id || ''))
const p = computed(() => store.getById(pid.value))
const isGenerated = computed(() => p.value?.status === 'generated')

// ─── 右侧面板：仅对话 ───────────────────────────────────────────
const panelTab = ref<'chat'>('chat')

// ─── SlideViewer 状态 ────────────────────────────────────────────
import { parseSlides } from '@/utils/htmlSlides'
import type { ElementInfo } from '@/components/presentation/SlideViewer.vue'

const currentSlideSrcdoc = computed(() => {
  if (!p.value?.htmlContent) return ''
  // 完整 HTML（所有页），由 iframe 内脚本控制显示哪一页
  return p.value.htmlContent
})

const slideCount = computed(() => {
  if (!p.value) return 0
  if (p.value.htmlContent) return parseSlides(p.value.htmlContent).length
  return p.value.slides.length
})

function onElementClicked(info: ElementInfo) {
  console.log('[Studio] element clicked:', info.tagName, info.text?.slice(0, 50))
}
function onSlideChanged(index: number, _total: number) {
  currentSlide.value = index
}

// ─── 演示材料 Agent 事件处理 ──────────────────────────────────────
function onPageGenerated(page: number, html: string) {
  if (!p.value) return
  store.applyPageHtml(p.value, page, html)
}
function onPageUpdated(page: number, html: string) {
  if (!p.value) return
  store.applyPageHtml(p.value, page, html)
}

// ─── AI 可用性提示 ───────────────────────────────────────────────
const aiReady = ref(true)
async function checkAiReady() {
  if (!isTauri()) {
    aiReady.value = false
    return
  }
  try {
    const { fetchAiConfig } = await import('@/api/db-client')
    const cfg = (await fetchAiConfig()) as unknown as {
      providers?: Array<{ id: string }>
      activeProviderId?: string
    }
    const aid = cfg?.activeProviderId
    aiReady.value = !!(aid && (cfg?.providers ?? []).some((p) => p.id === aid))
  } catch {
    aiReady.value = false
  }
}

onMounted(() => {
  store.hydrate()
  window.addEventListener('keydown', onGlobalKey)
  checkAiReady()
  if (!p.value) {
    toast({ message: '演示材料不存在', type: 'error' })
    router.replace('/presentations')
  } else {
    // 若已有 HTML 内容但无缩略图，则生成缩略图
    if (p.value.htmlContent && !p.value.thumbnails?.length) {
      store.refreshThumbnails(p.value.id)
    }
  }
})
onUnmounted(() => {
  window.removeEventListener('keydown', onGlobalKey)
})

// ─── 标题行内编辑 ────────────────────────────────────────────────
const editingTitle = ref(false)
const titleDraft = ref('')
function startEditTitle() {
  if (!p.value) return
  titleDraft.value = p.value.title
  editingTitle.value = true
}
function saveTitle() {
  if (p.value && titleDraft.value.trim()) store.renamePresentation(p.value.id, titleDraft.value)
  editingTitle.value = false
}

// ─── 撤销 / 重做 ─────────────────────────────────────────────────
const canUndo = computed(() => history.undoStack.length > 0)
const canRedo = computed(() => history.redoStack.length > 0)
function undo() {
  history.undo()
}
function redo() {
  history.redo()
}

// ─── 幻灯片级操作（增删 / 复制） ─────────────────────────────────
function addSlide() {
  if (!p.value) return
  const id = store.addSlide(p.value.id)
  if (id) currentSlide.value = p.value.slides.findIndex((s) => s.id === id)
}
function duplicateSlide() {
  if (!p.value) return
  const id = store.duplicateSlide(p.value.id, currentSlide.value)
  if (id) currentSlide.value = p.value.slides.findIndex((s) => s.id === id)
}
function deleteSlide() {
  if (!p.value) return
  store.deleteSlide(p.value.id, currentSlide.value)
  currentSlide.value = Math.min(currentSlide.value, p.value.slides.length - 1)
}
function moveSlide(dir: -1 | 1) {
  if (!p.value) return
  const from = currentSlide.value
  const to = from + dir
  if (to < 0 || to >= p.value.slides.length) return
  store.moveSlide(p.value.id, from, to)
  currentSlide.value = to
}

/** Ctrl/Cmd+Z 撤销、Ctrl+Shift+Z / Ctrl+Y 重做（输入框内不拦截） */
function onGlobalKey(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null
  const tag = t?.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || t?.classList?.contains('ProseMirror')) return
  if (!(e.ctrlKey || e.metaKey)) return
  const k = e.key.toLowerCase()
  if (k === 'z') {
    e.preventDefault()
    if (e.shiftKey) redo()
    else undo()
  } else if (k === 'y') {
    e.preventDefault()
    redo()
  }
}

// ─── 幻灯片浏览与播放 ────────────────────────────────────────────
const currentSlide = ref(0)
const playing = ref(false)

watch(currentSlide, (v) => {
  const total = p.value?.slides.length ?? 0
  if (v >= total) currentSlide.value = Math.max(0, total - 1)
})
watch(
  () => p.value?.status,
  (s, old) => {
    if (s === 'generated' && old === 'discussing') {
      currentSlide.value = 0
    }
  }
)

// ─── 演示材料操作 ────────────────────────────────────────────────
async function startGenerate() {
  if (!p.value || p.value.generating) return
  busy.value = true
  await store.startGeneration(p.value.id)
  busy.value = false
  currentSlide.value = 0
}

const busy = ref(false)
const refiningIdx = ref<number | null>(null)

function toggleRefine() {
  if (!p.value || !p.value.slides.length) return
  refiningIdx.value = refiningIdx.value === currentSlide.value ? null : currentSlide.value
}

/** 缩略图上的精修快捷入口 */
function refineFromThumb(i: number) {
  currentSlide.value = i
  refiningIdx.value = i
}

/** 行内编辑标题自动聚焦 */
const vFocus = {
  mounted: (el: HTMLInputElement) => {
    el.focus()
    el.select()
  }
}
</script>

<template>
  <div
    v-if="p"
    class="flex flex-col w-full h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-hidden"
  >
    <!-- ═══ 顶部工具栏 ═══ -->
    <header class="h-10 flex-shrink-0 flex items-center gap-2 px-3 border-b border-surface-100">
      <button
        class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
        title="返回清单"
        @click="router.push('/presentations')"
      >
        <AppIcon name="arrow-left" size="14" />
      </button>

      <input
        v-if="editingTitle"
        v-model="titleDraft"
        v-focus
        class="flex-1 max-w-md border border-brand-300 rounded px-2 py-0.5 text-[12.5px] font-medium outline-none focus:ring-1 focus:ring-brand-400 select-text"
        @keyup.enter="saveTitle"
        @blur="saveTitle"
      />
      <h1
        v-else
        class="flex-1 min-w-0 truncate text-[13px] font-semibold text-surface-800 cursor-text hover:text-brand-700 transition-colors"
        title="点击重命名"
        @click="startEditTitle"
      >
        {{ p.title }}
      </h1>

      <!-- 状态徽标 -->
      <span
        v-if="p.generating"
        class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-blue-50 text-blue-600"
      >
        <span class="w-1 h-1 rounded-full bg-blue-500 animate-pulse"></span>
        生成中…
      </span>
      <span
        v-else-if="isGenerated"
        class="px-2 py-0.5 rounded-full text-[10px] font-medium bg-emerald-50 text-emerald-600"
        >{{ p.slides.length }} 页 · 已生成</span
      >
      <span
        v-else
        class="px-2 py-0.5 rounded-full text-[10px] font-medium bg-surface-100 text-surface-500"
        >沟通策划中</span
      >

      <!-- 撤销 / 重做 -->
      <div class="flex items-center gap-1">
        <button
          class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          title="撤销 (Ctrl+Z)"
          :disabled="!canUndo"
          @click="undo"
        >
          <svg
            class="w-3.5 h-3.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M9 14L4 9l5-5M4 9h10a6 6 0 016 6v0a6 6 0 01-6 6H7" />
          </svg>
        </button>
        <button
          class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          title="重做 (Ctrl+Shift+Z)"
          :disabled="!canRedo"
          @click="redo"
        >
          <svg
            class="w-3.5 h-3.5"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M15 14l5-5-5-5M20 9H10a6 6 0 00-6 6v0a6 6 0 006 6h7" />
          </svg>
        </button>
      </div>

      <!-- 播放 -->
      <button
        v-if="isGenerated && p.slides.length > 0"
        class="flex items-center gap-1 px-2.5 py-1 text-[10px] rounded-md bg-brand-600 text-white hover:bg-brand-700 transition-colors"
        @click="playing = true"
      >
        <AppIcon name="play" size="10" />
        播放
      </button>
    </header>

    <div class="flex-1 flex min-h-0">
      <!-- ═══ 左侧缩略图栏（已生成） ═══ -->
      <aside
        v-if="isGenerated"
        class="w-44 flex-shrink-0 border-r border-surface-200 bg-surface-50/30 overflow-y-auto scrollbar-thin p-2 space-y-2"
      >
        <button
          class="w-full h-7 flex items-center justify-center gap-1 rounded-lg border border-dashed border-surface-200 text-[10px] text-surface-400 hover:text-brand-600 hover:border-brand-300 transition-colors"
          @click="addSlide"
        >
          <svg
            class="w-3 h-3"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
          新增页
        </button>
        <button
          v-for="(s, i) in p.slides"
          :key="s.id"
          class="group/thumb relative w-full rounded-lg overflow-hidden border transition-all"
          :class="
            i === currentSlide
              ? 'border-brand-500 ring-2 ring-brand-200'
              : 'border-surface-200 hover:border-brand-300'
          "
          @click="currentSlide = i"
        >
          <img
            v-if="p.thumbnails?.[i]"
            :src="p.thumbnails[i]"
            class="w-full h-auto block"
            :alt="`第${i + 1}页`"
          />
          <PresentationFrame v-else :slide="s" :theme="p.theme" />
          <span
            class="absolute left-1 top-1 px-1 py-0.5 rounded bg-black/40 text-white text-[9px] leading-none"
            >{{ i + 1 }}</span
          >
          <span
            class="absolute right-1 top-1 w-5 h-5 hidden group-hover/thumb:flex items-center justify-center rounded bg-black/45 text-white backdrop-blur-sm"
            title="精修此页"
            @click.stop="refineFromThumb(i)"
          >
            <svg
              class="w-2.5 h-2.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              stroke-width="2"
            >
              <path
                d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
              />
            </svg>
          </span>
          <span
            class="absolute left-1 bottom-1 hidden group-hover/thumb:flex items-center gap-0.5"
            @click.stop
          >
            <button
              class="w-5 h-5 flex items-center justify-center rounded bg-black/45 text-white backdrop-blur-sm hover:bg-black/65"
              title="复制此页"
              @click="
                i === currentSlide ? duplicateSlide() : ((currentSlide = i), duplicateSlide())
              "
            >
              <svg
                class="w-2.5 h-2.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M9 9h11v11H9zM5 15H4a1 1 0 01-1-1V4a1 1 0 011-1h10a1 1 0 011 1v1" />
              </svg>
            </button>
            <button
              class="w-5 h-5 flex items-center justify-center rounded bg-black/45 text-white backdrop-blur-sm hover:bg-red-600"
              title="删除此页"
              @click="i === currentSlide ? deleteSlide() : ((currentSlide = i), deleteSlide())"
            >
              <svg
                class="w-2.5 h-2.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M6 6l12 12M6 18L18 6" />
              </svg>
            </button>
          </span>
        </button>
        <!-- 生成中骨架 -->
        <div
          v-if="p.generating"
          class="rounded-lg border border-dashed border-brand-300 bg-brand-50/50 aspect-video flex items-center justify-center"
        >
          <span
            class="w-4 h-4 border-2 border-brand-300 border-t-brand-600 rounded-full animate-spin"
          ></span>
        </div>
      </aside>

      <!-- ═══ 中央区域 ═══ -->
      <section class="flex-1 min-w-0 flex flex-col bg-surface-50/30">
        <!-- 未生成：引导区 -->
        <div
          v-if="!isGenerated && p.slides.length === 0"
          class="flex-1 flex items-center justify-center p-8"
        >
          <div class="max-w-md text-center">
            <div
              class="w-20 h-20 mx-auto mb-5 rounded-3xl bg-gradient-to-br from-[#2563eb]/10 to-[#93c5fd]/10 ring-1 ring-[#2563eb]/10 flex items-center justify-center"
            >
              <AppIcon name="chat" class="w-9 h-9 text-[#2563eb]" />
            </div>
            <template v-if="(p.discussion.filter((m) => m.role === 'user').length ?? 0) === 0">
              <h2 class="text-[15px] font-semibold text-surface-800 mb-2">先聊聊你的想法</h2>
              <p class="text-[12px] text-surface-400 leading-relaxed mb-5">
                在右侧和 AI 策划助手沟通演示的用途、听众与重点内容。<br />
                AI 会检索知识库，与你反复打磨结构，确认后再逐页制作。
              </p>
            </template>
            <template v-else>
              <h2 class="text-[15px] font-semibold text-surface-800 mb-2">方向已经清晰了吗？</h2>
              <p class="text-[12px] text-surface-400 leading-relaxed mb-5">
                已经沟通
                {{ p.discussion.filter((m) => m.role === 'user').length }}
                轮。随时可以继续补充想法，<br />
                或者现在就开始生成完整幻灯片。
              </p>
            </template>
            <button
              class="inline-flex items-center gap-1.5 px-4 py-2 rounded-lg bg-brand-600 hover:bg-brand-700 text-white text-[12px] font-medium shadow-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              :disabled="p.generating"
              @click="startGenerate"
            >
              <span
                v-if="p.generating"
                class="w-3 h-3 border-2 border-white/40 border-t-white rounded-full animate-spin"
              ></span>
              <AppIcon v-else name="play" size="12" />
              {{ p.generating ? '正在生成幻灯片…' : '开始生成演示材料' }}
            </button>
            <p class="mt-3.5 text-[10px] text-surface-300">也可以直接在右侧对 AI 说「开始生成」</p>
          </div>
        </div>

        <!-- 已生成：iframe 预览（新 HTML 架构） -->
        <template v-if="p.htmlContent">
          <div class="flex-1 flex items-center justify-center p-5 min-h-0">
            <div
              class="w-full max-w-[880px] rounded-xl overflow-hidden shadow-lg ring-1 ring-black/5"
            >
              <SlideViewer
                :srcdoc="currentSlideSrcdoc"
                :page-index="currentSlide"
                @element-clicked="onElementClicked"
                @slide-changed="onSlideChanged"
              />
            </div>
          </div>
          <!-- 分页控制条 -->
          <div class="flex-shrink-0 flex items-center justify-center gap-3 pb-4 pt-1">
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              :disabled="currentSlide <= 0"
              @click="currentSlide--"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path d="M15 19l-7-7 7-7" />
              </svg>
            </button>
            <span class="text-[12px] text-surface-500 tabular-nums min-w-[64px] text-center"
              >{{ currentSlide + 1 }} / {{ slideCount }}</span
            >
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              :disabled="currentSlide >= slideCount - 1"
              @click="currentSlide++"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path d="M9 6l6 6-6 6" />
              </svg>
            </button>
          </div>
        </template>
        <!-- 已生成：旧 block 预览（兼容旧数据） -->
        <template v-else-if="p.slides.length > 0">
          <div class="flex-1 flex items-center justify-center p-5 min-h-0">
            <div
              class="w-full max-w-[880px] rounded-xl overflow-hidden shadow-lg ring-1 ring-black/5"
              @click="selectedBlock = null"
            >
              <PresentationFrame
                :slide="p.slides[currentSlide]"
                :theme="p.theme"
                interactive
                :selected-index="selectedBlock ?? undefined"
                :editing-index="inlineEdit?.block ?? null"
                :editing-field="inlineEdit?.field ?? null"
                @select-block="onSelectBlock"
                @begin-edit="onBeginEdit"
                @update-field="onUpdateField"
                @end-edit="onEndEdit"
              />
            </div>
          </div>
          <!-- 分页控制条 -->
          <div class="flex-shrink-0 flex items-center justify-center gap-3 pb-4 pt-1">
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              :disabled="currentSlide <= 0"
              @click="currentSlide--"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path d="M15 19l-7-7 7-7" />
              </svg>
            </button>
            <span class="text-[12px] text-surface-500 tabular-nums min-w-[64px] text-center"
              >{{ currentSlide + 1 }} / {{ p.slides.length }}</span
            >
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              :disabled="currentSlide >= p.slides.length - 1"
              @click="currentSlide++"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path d="M9 6l6 6-6 6" />
              </svg>
            </button>
            <div class="w-px h-4 bg-surface-200 mx-1"></div>
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              title="上移本页"
              :disabled="currentSlide <= 0"
              @click="moveSlide(-1)"
            >
              <svg
                class="w-4 h-4"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M5 15l7-7 7 7" />
              </svg>
            </button>
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              title="下移本页"
              :disabled="currentSlide >= p.slides.length - 1"
              @click="moveSlide(1)"
            >
              <svg
                class="w-4 h-4"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M19 9l-7 7-7-7" />
              </svg>
            </button>
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-brand-600 hover:border-brand-300 transition-colors"
              title="复制本页"
              @click="duplicateSlide"
            >
              <svg
                class="w-4 h-4"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M9 9h11v11H9zM5 15H4a1 1 0 01-1-1V4a1 1 0 011-1h10a1 1 0 011 1v1" />
              </svg>
            </button>
            <button
              class="w-8 h-8 flex items-center justify-center rounded-lg border border-surface-200 bg-white text-surface-500 hover:text-red-500 hover:border-red-300 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
              title="删除本页"
              :disabled="p.slides.length <= 1"
              @click="deleteSlide"
            >
              <svg
                class="w-4 h-4"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path d="M6 6l12 12M6 18L18 6" />
              </svg>
            </button>
            <div class="w-px h-4 bg-surface-200 mx-1"></div>
            <button
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg text-[12px] font-medium transition-colors border"
              :class="
                refiningIdx === currentSlide
                  ? 'bg-brand-50 text-brand-700 border-brand-300'
                  : 'bg-white text-surface-600 border-surface-200 hover:border-brand-300 hover:text-brand-600'
              "
              @click="toggleRefine"
            >
              <svg
                class="w-3.5 h-3.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                stroke-width="2"
              >
                <path
                  d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                />
              </svg>
              {{ refiningIdx === currentSlide ? '退出精修' : '精修本页' }}
            </button>
          </div>
        </template>
      </section>

      <!-- ═══ 右侧 AI 策划面板 ═══ -->
      <aside class="w-[360px] flex-shrink-0 border-l border-surface-200 flex flex-col bg-white">
        <div
          class="h-10 flex-shrink-0 flex items-center justify-between px-3 border-b border-surface-100"
        >
          <div class="flex items-center gap-1 bg-surface-100 rounded-lg p-0.5">
            <button
              class="px-2.5 py-1 rounded-md text-[11px] font-medium transition-colors"
              :class="
                panelTab === 'chat'
                  ? 'bg-white text-surface-800 shadow-sm'
                  : 'text-surface-500 hover:text-surface-700'
              "
              @click="panelTab = 'chat'"
            >
              对话
            </button>
          </div>
          <span class="text-[10px] text-surface-300">沟通 → 生成 → 精修</span>
        </div>

        <!-- ═══ 对话 Tab ═══ -->
        <template v-if="panelTab === 'chat'">
          <PresentationChatPanel
            :pid="pid"
            :deck-id="pid"
            class="flex-1 min-h-0"
            @page-generated="onPageGenerated"
            @page-updated="onPageUpdated"
          />
        </template>
      </aside>
    </div>
  </div>

  <!-- 全屏播放器 -->
  <PresentationPlayer
    v-if="playing && p"
    :slides="p.slides"
    :theme="p.theme"
    :start-index="currentSlide"
    :html-content="p.htmlContent"
    @close="playing = false"
  />
</template>

<style scoped>
.markdown-msg :deep(p) {
  margin: 0 0 6px;
}
.markdown-msg :deep(p:last-child) {
  margin-bottom: 0;
}
.markdown-msg :deep(ol),
.markdown-msg :deep(ul) {
  margin: 4px 0;
  padding-left: 18px;
}
.markdown-msg :deep(li) {
  margin: 2px 0;
}
.markdown-msg :deep(strong) {
  color: #1d4ed8;
  font-weight: 600;
}
.dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #94a3b8;
  animation: dotBounce 1s infinite ease-in-out;
}
@keyframes dotBounce {
  0%,
  80%,
  100% {
    transform: scale(0.7);
    opacity: 0.4;
  }
  40% {
    transform: scale(1);
    opacity: 1;
  }
}
.scrollbar-thin::-webkit-scrollbar {
  width: 4px;
}
.scrollbar-thin::-webkit-scrollbar-thumb {
  background: #e5e5ea;
  border-radius: 4px;
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
