import { ref, computed, type Ref } from 'vue'

export function useMultiSelect<T extends { id: string }>(items: Ref<T[]>) {
  const selectedIds = ref<Set<string>>(new Set())
  const anchorId = ref<string | null>(null)

  function clear() { selectedIds.value = new Set(); anchorId.value = null }

  function selectSingle(id: string) {
    const s = new Set<string>(); s.add(id)
    selectedIds.value = s; anchorId.value = id
  }

  function selectRange(id: string) {
    if (!anchorId.value) { selectSingle(id); return }
    const ids = items.value.map(i => i.id)
    const a = ids.indexOf(anchorId.value), b = ids.indexOf(id)
    if (a < 0 || b < 0) { selectSingle(id); return }
    const [lo, hi] = a <= b ? [a, b] : [b, a]
    selectedIds.value = new Set(ids.slice(lo, hi + 1))
  }

  function isSelected(id: string) { return selectedIds.value.has(id) }
  const selectedCount = computed(() => selectedIds.value.size)

  return { selectedIds, anchorId, clear, selectSingle, selectRange, isSelected, selectedCount }
}
