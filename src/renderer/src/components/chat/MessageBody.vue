<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { splitInteractive } from '@/utils/uiBlocks'
import { renderMd, useAgentChat } from '@/composables/useAgentChat'
import { toast } from '@/composables/useToast'
import UiBlock from '@/components/chat/UiBlock.vue'
import MarkdownTable from '@/components/chat/MarkdownTable.vue'

const props = defineProps<{ raw: string; msgId: string; annotate?: boolean }>()
const emit = defineEmits<{ send: [text: string] }>()
const agent = useAgentChat()

const segs = computed(() => splitInteractive(props.raw))
const htmls = reactive<Record<number, string>>({})

const tableRefs = ref<Record<number, { getRevision: () => { diff: string; headerLabel: string }; hasChanges: () => boolean } | null>>({})
function setTableRef(i: number, el: unknown) { tableRefs.value[i] = el as never }
function getRevisions(): { diff: string; headerLabel: string }[] {
  return Object.entries(tableRefs.value)
    .filter(([, r]) => r?.hasChanges())
    .sort((a, b) => Number(a[0]) - Number(b[0]))
    .map(([, r]) => r!.getRevision())
}
defineExpose({ getRevisions, hasTableChanges: () => Object.values(tableRefs.value).some(r => r?.hasChanges()) })

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
</script>

<template>
  <div class="space-y-1">
    <template v-for="(seg, i) in segs" :key="i">
      <UiBlock v-if="seg.kind === 'block'" :raw="seg.text" :msg-id="msgId" @send="sendFromBlock" />
      <MarkdownTable v-else-if="seg.kind === 'table'" :md="seg.text" :forced="!!annotate" :ref="(el) => setTableRef(i, el)" />
      <!-- md 段：htmls 未就绪时留空（异步渲染瞬时）；与旧整段渲染行为一致 -->
      <div v-else class="markdown-body" v-html="htmls[i] || ''" />
    </template>
  </div>
</template>
