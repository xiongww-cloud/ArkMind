<script setup lang="ts">
import { ref, onUnmounted } from 'vue'

/**
 * 批注选区层：active 时监听容器内文本选区 → 浮出「添加批注」气泡 → 输入修订要求 → emit add。
 * 表格容器（.md-table-root）与输入框内的选区一律忽略——表格走单元格编辑通道，两通道不交叉。
 */
const props = defineProps<{ active: boolean }>()
const emit = defineEmits<{ add: [annotation: { text: string; note: string }] }>()

const root = ref<HTMLElement | null>(null)
const bubble = ref<{ x: number; y: number; text: string } | null>(null)
const composer = ref<{ x: number; y: number; text: string; note: string } | null>(null)

function onDocMouseUp() {
  if (!props.active) { bubble.value = null; return }
  requestAnimationFrame(() => {
    const sel = window.getSelection?.()
    if (!props.active || !sel || sel.isCollapsed) { if (!composer.value) bubble.value = null; return }
    const text = sel.toString().trim()
    if (!text) { if (!composer.value) bubble.value = null; return }
    const range = sel.getRangeAt(0)
    if (!root.value?.contains(range.commonAncestorContainer)) { return }
    const parent = (range.commonAncestorContainer.nodeType === Node.TEXT_NODE ? range.commonAncestorContainer.parentElement : range.commonAncestorContainer) as HTMLElement | null
    if (!parent || parent.closest('.md-table-root') || parent.closest('input,textarea') || parent.closest('.annotate-skip')) { bubble.value = null; return }
    const r = range.getBoundingClientRect()
    if (!r.width && !r.height) { bubble.value = null; return }
    bubble.value = { x: Math.min(Math.max(r.left + r.width / 2, 90), window.innerWidth - 90), y: r.bottom + 6, text: text.slice(0, 120) }
  })
}
document.addEventListener('mouseup', onDocMouseUp)
onUnmounted(() => { document.removeEventListener('mouseup', onDocMouseUp) })

function openComposer() {
  if (!bubble.value) return
  composer.value = { ...bubble.value, note: '' }
  bubble.value = null
}
const clampComposerX = (x: number) => Math.min(Math.max(x, 140), window.innerWidth - 140)
function confirmAdd() {
  if (!composer.value || !composer.value.note.trim()) return
  emit('add', { text: composer.value.text, note: composer.value.note.trim() })
  composer.value = null
  window.getSelection()?.removeAllRanges()
}
function cancelComposer() { composer.value = null }

defineExpose({ close: () => { bubble.value = null; composer.value = null } })
</script>

<template>
  <div ref="root" :class="active ? 'selection-[color:#1e40af] selection:bg-blue-100' : ''"><slot /></div>
  <Teleport to="body">
    <button v-if="bubble && active" type="button"
      class="fixed z-[95] annotate-skip px-2.5 py-1 rounded-full bg-slate-800/95 text-white text-[10px] shadow-lg hover:bg-slate-700 transition-colors"
      :style="{ left: bubble.x + 'px', top: bubble.y + 'px', transform: 'translateX(-50%)' }"
      @mousedown.prevent="openComposer">✎ 批注</button>
    <div v-if="composer && active" class="fixed z-[96] annotate-skip w-[264px] rounded-xl border border-brand-300 bg-white shadow-xl p-2.5"
      :style="{ left: clampComposerX(composer.x) + 'px', top: composer.y + 'px', transform: 'translateX(-50%)' }">
      <p class="text-[10px] text-surface-400 leading-snug max-h-[42px] overflow-hidden mb-1.5">选中：{{ composer.text }}</p>
      <textarea v-model="composer.note" rows="2" placeholder="写下你的修订要求…" autofocus
        class="w-full text-[11px] border border-surface-200 rounded-lg px-2 py-1.5 outline-none focus:border-brand-400 resize-none" @keydown.esc.prevent="cancelComposer" />
      <div class="flex justify-end gap-1.5 mt-1.5">
        <button type="button" @click="cancelComposer" class="px-2 py-0.5 text-[10px] text-surface-400 hover:text-surface-600">取消</button>
        <button type="button" :disabled="!composer.note.trim()" @click="confirmAdd"
          class="px-2.5 py-0.5 text-[10px] font-medium rounded-md bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-40 disabled:cursor-not-allowed">添加批注</button>
      </div>
    </div>
  </Teleport>
</template>
