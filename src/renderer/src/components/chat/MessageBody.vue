<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { splitInteractive, parseUiBlock } from '@/utils/uiBlocks'
import { renderMd, useAgentChat } from '@/composables/useAgentChat'
import { toast } from '@/composables/useToast'
import UiBlock from '@/components/chat/UiBlock.vue'
import MarkdownTable from '@/components/chat/MarkdownTable.vue'

const props = defineProps<{ raw: string; msgId: string; annotate?: boolean }>()
const emit = defineEmits<{ send: [text: string]; 'revision-change': [revisions: { diff: string; headerLabel: string }[]] }>()
const agent = useAgentChat()

const segs = computed(() => splitInteractive(props.raw).filter(sg => !(sg.kind === 'block' && planOf(sg.text))))
const htmls = reactive<Record<number, string>>({})

type TableInst = { getRevision: () => { diff: string; headerLabel: string }; hasChanges: () => boolean; reset: () => void } | null
const tableRefs = ref<Record<number, TableInst>>({})
function setTableRef(i: number, el: unknown) { tableRefs.value[i] = el as never }
function changedTables() {
  return Object.entries(tableRefs.value)
    .filter(([, r]) => r?.hasChanges())
    .sort((a, b) => Number(a[0]) - Number(b[0]))
}
function getRevisions(): { diff: string; headerLabel: string }[] {
  return changedTables().map(([, r]) => r!.getRevision())
}
// 撤销第 i 个已改表格的全部编辑（i 与 getRevisions 顺序一致，批注面板卡片寻址用）
function resetTableRevision(i: number) { changedTables()[i]?.[1]?.reset() }
defineExpose({ getRevisions, hasTableChanges: () => Object.values(tableRefs.value).some(r => r?.hasChanges()), resetTableRevision })

watch(segs, async (list) => {
  for (let i = 0; i < list.length; i++) {
    if (list[i].kind !== 'md') continue
    const text = list[i].text
    if (!text.trim()) { htmls[i] = ''; continue }
    htmls[i] = await renderMd(text)
  }
}, { immediate: true, deep: false })

function sendFromBlock(text: string) {
  if (agent.isChatting.value) { toast({ message: '助手正在回复中，请稍候再提交', type: 'info' }); return }
  emit('send', text)
}

// plan 围栏检测（历史消息里整段隐藏：计划进度已归后端状态机 + 底部 Dock，消息流不复述）
const planCache = new Map<string, ReturnType<typeof parseUiBlock>>()
function planOf(text: string) {
  if (!planCache.has(text)) planCache.set(text, parseUiBlock(text))
  const b = planCache.get(text)
  return b && b.type === 'plan' ? b : null
}
</script>

<template>
  <div class="space-y-1">
    <template v-for="(seg, i) in segs" :key="i">
      <UiBlock v-if="seg.kind === 'block'" :raw="seg.text" :msg-id="msgId" @send="sendFromBlock" />
      <MarkdownTable v-else-if="seg.kind === 'table'" :md="seg.text" :forced="!!annotate" :ref="(el) => setTableRef(i, el)" @revised="emit('revision-change', getRevisions())" />
      <!-- md 段：htmls 未就绪时留空（异步渲染瞬时）；与旧整段渲染行为一致 -->
      <div v-else class="markdown-body" v-html="htmls[i] || ''" />
    </template>
  </div>
</template>
