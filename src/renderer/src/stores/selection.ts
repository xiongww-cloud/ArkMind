import { defineStore } from 'pinia'
import { reactive } from 'vue'

export type SelectionType = 'product' | 'note' | 'todo' | 'project' | 'contact' | 'partner' | 'email'

export interface JumpExtra {
  tab?: 'overview' | 'notes' | 'todos' | 'files'
  targetType?: SelectionType
  targetId?: string
}

export const useSelectionStore = defineStore('selection', () => {
  const current = reactive<{ type: SelectionType | null; id: string | null; seq: number; extra: JumpExtra | null }>({
    type: null,
    id: null,
    seq: 0,
    extra: null,
  })

  function jumpTo(t: SelectionType, i: string, extra?: JumpExtra) {
    current.type = t
    current.id = i
    current.extra = extra ?? null
    current.seq++
  }

  function clear() {
    current.type = null
    current.id = null
    current.extra = null
  }

  return { current, jumpTo, clear }
})
