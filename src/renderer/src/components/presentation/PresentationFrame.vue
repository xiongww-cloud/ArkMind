<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import type { BlockInstance, Slide } from '@/types/presentation'
import { getLayout, getTheme, normalizeScale, normalizeSpan } from '@/presentation/elements'
import PresentationBlock from './PresentationBlock.vue'

const props = defineProps<{
  slide: Slide
  theme?: string
  page?: number
  total?: number
}>()

const emit = defineEmits<{
  (e: 'select-block', index: number): void
}>()

// ─── 1280×720 设计尺寸，按容器宽度等比缩放 ────────────────────────
const DESIGN_W = 1280
const DESIGN_H = 720

const viewportRef = ref<HTMLElement | null>(null)
const scale = ref(1)
let ro: ResizeObserver | null = null

onMounted(() => {
  if (!viewportRef.value) return
  ro = new ResizeObserver((entries) => {
    const w = entries[0]?.contentRect.width ?? 0
    if (w > 0) scale.value = w / DESIGN_W
  })
  ro.observe(viewportRef.value)
})

onUnmounted(() => {
  ro?.disconnect()
  ro = null
})

const canvasStyle = computed(() => ({
  width: `${DESIGN_W}px`,
  height: `${DESIGN_H}px`,
  transform: `scale(${scale.value})`
}))

/** 主题 → CSS token（结构对象驱动，切换主题不重建内容） */
const theme = computed(() => getTheme(props.theme))
const themeStyle = computed(() => ({
  background: theme.value.colors.backgroundImage ?? theme.value.colors.background,
  color: theme.value.colors.text,
  fontFamily: theme.value.fonts.family,
  '--pf-accent': theme.value.colors.accent,
  '--pf-tint': theme.value.colors.tint,
  '--pf-line': theme.value.colors.line,
  '--pf-muted': theme.value.colors.muted,
  '--pf-card': theme.value.colors.card
}))

const layout = computed(() => getLayout(props.slide.layout))
const isHero = computed(() => layout.value.shell === 'hero')
/** hero 版式：所有区块均 ≥2/3 宽时保持居中单列；存在半宽/1/3 时进入栅格（支持左右布局） */
const heroSingle = computed(
  () => isHero.value && props.slide.blocks.every((b) => normalizeSpan(b.span) >= 8)
)

const cellStyle = (b: BlockInstance) => ({
  gridColumn: `span ${normalizeSpan(b.span)}`,
  '--pf-fs': normalizeScale(b.scale)
})

/** header 版式：开头连续的 眉题/标题 归入标题区（置顶），其余区块进正文区（页面垂直居中） */
const leadTitles = computed<BlockInstance[]>(() => {
  const out: BlockInstance[] = []
  for (const b of props.slide.blocks) {
    if (b.type === 'heading' || b.type === 'kicker') out.push(b)
    else break
  }
  return out
})
const bodyBlocks = computed<BlockInstance[]>(() =>
  props.slide.blocks.slice(leadTitles.value.length)
)
</script>

<template>
  <div ref="viewportRef" class="pf-viewport">
    <div
      class="pf-canvas"
      :style="[canvasStyle, themeStyle]"
    >
      <!-- 版式壳：hero（封面/章节/引用/结束；整体垂直居中） -->
      <div v-if="isHero" class="pf-hero">
        <div class="pf-cells" :class="heroSingle ? 'single' : ''">
          <div
            v-for="(b, i) in slide.blocks"
            :key="`${b.type}-${i}`"
            class="pf-cell"
            :style="cellStyle(b)"
          >
            <PresentationBlock :block="b" hero />
          </div>
        </div>
      </div>

      <!-- 版式壳：header（标题置顶 + 正文区页面垂直居中） -->
      <div v-else class="pf-header-body">
        <div v-if="leadTitles.length" class="pf-title-zone">
          <div
            v-for="(b, i) in leadTitles"
            :key="`t-${b.type}-${i}`"
            class="pf-cell pf-cell-title"
          >
            <PresentationBlock :block="b" />
          </div>
        </div>
        <div class="pf-body-zone">
          <div class="pf-cells">
            <div
              v-for="(b, i) in bodyBlocks"
              :key="`b-${b.type}-${i}`"
              class="pf-cell"
              :style="cellStyle(b)"
            >
              <PresentationBlock :block="b" />
            </div>
          </div>
        </div>
      </div>

      <!-- 页脚 -->
      <footer v-if="page && total" class="pf-footer">
        <span>{{ page }} / {{ total }}</span>
      </footer>
    </div>
  </div>
</template>

<style>
/* ═══ 视口与画布（等比缩放） ═══ */
.pf-viewport {
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  overflow: hidden;
}
.pf-canvas {
  position: absolute;
  top: 0;
  left: 0;
  transform-origin: 0 0;
  box-sizing: border-box;
  overflow: hidden;
  font-family: Inter, 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif;
}

/* ═══ 主题变量（结构对象通过内联 style 注入；以下仅为兜底默认值） ═══ */
.pf-canvas {
  --pf-accent: #2563eb;
  --pf-tint: #eff6ff;
  --pf-line: #e5e7eb;
  --pf-muted: #86909c;
  --pf-card: #ffffff;
}

/* ═══ 版式壳 ═══ */
.pf-hero {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 64px 88px;
  box-sizing: border-box;
}
.pf-header-body {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 72px 88px;
  box-sizing: border-box;
  overflow: hidden;
}
/* 标题区：置顶，不参与垂直居中 */
.pf-title-zone {
  flex-shrink: 0;
  margin-bottom: 30px;
}
.pf-title-zone .pf-cell-title {
  width: 100%;
}
/* 正文区：占满剩余高度并垂直居中，避免页面下半部分留白 */
.pf-body-zone {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
}
.pf-body-zone .pf-cells {
  width: 100%;
}
/* header 版式中的标题左侧强调条，保留原有视觉 */
.pf-header-body .pb-heading {
  display: flex;
  align-items: center;
  gap: 16px;
}
.pf-header-body .pb-heading::before {
  content: '';
  width: 10px;
  height: 36px;
  border-radius: 5px;
  background: var(--pf-accent);
  flex-shrink: 0;
}

/* ═══ 12 列栅格：区块自由排布（全宽/半宽/1/3…） ═══ */
.pf-cells {
  width: 100%;
  display: grid;
  grid-template-columns: repeat(12, minmax(0, 1fr));
  grid-auto-flow: row dense;
  gap: 26px 30px;
  align-items: center;
}
.pf-hero .pf-cells {
  max-width: 1160px;
}
.pf-cell {
  min-width: 0;
}
/* hero 单块：保持居中排版 */
.pf-cells.single {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 22px;
}
.pf-cells.single .pf-cell {
  grid-column: auto;
  max-width: 920px;
}

/* ═══ 页脚页码 ═══ */
.pf-footer {
  position: absolute;
  right: 40px;
  bottom: 26px;
  font-size: 17px;
  color: var(--pf-muted);
  letter-spacing: 0.05em;
}
</style>
