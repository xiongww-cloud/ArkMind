<script setup lang="ts">
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { previewFile } from '@/api/db-client'
import { toast } from '@/composables/useToast'

const props = defineProps(nodeViewProps)

function handleClick(e: MouseEvent) {
  e.preventDefault()
  e.stopPropagation()
  const fid = props.node.attrs.fileId as string
  if (fid) previewFile(fid).catch(() => toast({ message: '无法预览文件', type: 'error' }))
}
</script>

<template>
  <node-view-wrapper class="inline-block">
    <span
      class="file-link"
      data-type="file-link"
      :data-file-id="node.attrs.fileId"
      @click.stop="handleClick"
    >
      <svg class="link-svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" />
      </svg>
      <span class="link-text">{{ node.attrs.name }}</span>
    </span>
  </node-view-wrapper>
</template>
