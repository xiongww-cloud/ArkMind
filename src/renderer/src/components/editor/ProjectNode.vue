<script setup lang="ts">
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { useLinkModals } from '@/composables/useLinkModals'

const props = defineProps(nodeViewProps)
const { openProject } = useLinkModals()

function handleClick(e: MouseEvent) {
  e.preventDefault()
  e.stopPropagation()
  const pid = props.node.attrs.projectId as string
  if (pid) openProject(pid)
}
</script>

<template>
  <node-view-wrapper class="inline-block">
    <span
      class="project-link"
      data-type="project-link"
      :data-project-id="node.attrs.projectId"
      @click.stop="handleClick"
    >
      <svg class="link-svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
      </svg>
      <span class="link-text">{{ node.attrs.name }}</span>
    </span>
  </node-view-wrapper>
</template>
