<script setup lang="ts">
import { ref, computed, watch } from 'vue'

const props = withDefaults(defineProps<{ value: string; max?: number }>(), { max: 3 })

const expanded = ref(false)
watch(() => props.value, () => { expanded.value = false })

// 逗号在 <> 之外才作分隔（保护 "显示名 <a@x.com>" 与带引号名内逗号）；分号先归一为逗号
const addrs = computed(() =>
  (props.value || '')
    .replace(/;/g, ',')
    .split(/,\s*(?![^<]*>)/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
)

const overflow = computed(() => addrs.value.length > props.max)
const preview = computed(() => addrs.value.slice(0, props.max).join(', '))
</script>

<template>
  <span v-if="!overflow" class="break-all">{{ value }}</span>
  <span v-else-if="!expanded" class="break-all">
    {{ preview }}…<button @click="expanded = true" class="text-brand-600 hover:underline whitespace-nowrap ml-1">查看详情({{ addrs.length }})</button>
  </span>
  <span v-else class="inline-flex flex-col gap-0.5 min-w-0">
    <span v-for="(a, i) in addrs" :key="i" class="break-all">{{ a }}<template v-if="i < addrs.length - 1">,</template></span>
    <button @click="expanded = false" class="text-brand-600 hover:underline w-fit">收起</button>
  </span>
</template>
