<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { splitUiBlocks } from '@/utils/uiBlocks'
import { renderMd, useAgentChat } from '@/composables/useAgentChat'
import { toast } from '@/composables/useToast'
import UiBlock from '@/components/chat/UiBlock.vue'

const props = defineProps<{ raw: string; msgId: string }>()
const emit = defineEmits<{ send: [text: string] }>()
const agent = useAgentChat()

const segs = computed(() => splitUiBlocks(props.raw))
const htmls = reactive<Record<number, string>>({})

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
      <!-- md 段：htmls 未就绪时留空（异步渲染瞬时）；与旧整段渲染行为一致 -->
      <div v-else class="markdown-body" v-html="htmls[i] || ''" />
    </template>
  </div>
</template>
