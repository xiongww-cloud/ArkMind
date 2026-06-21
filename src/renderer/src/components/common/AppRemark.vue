<script setup lang="ts">
defineProps({
  /** 备注文本，为空/falsy 时自动显示"无备注" */
  text: { type: String, default: '' },
})
</script>

<template>
  <span
    class="text-[13px] leading-relaxed line-clamp-2 rounded px-1.5 py-0 border border-transparent"
    :class="text ? 'text-slate-500' : 'text-slate-400 italic'"
  >{{ text || '无备注' }}</span>
</template>
