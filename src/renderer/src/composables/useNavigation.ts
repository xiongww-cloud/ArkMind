import { useRouter } from 'vue-router'
import { useSelectionStore } from '@/stores/selection'
import type { SelectionType, JumpExtra } from '@/stores/selection'

const typeRouteMap: Record<SelectionType, string> = {
  product: '/products',
  note: '/notes',
  todo: '/todos',
  project: '/projects',
}

export function useNavigation() {
  const router = useRouter()
  const selectionStore = useSelectionStore()

  function navigateTo(type: SelectionType, id: string, extra?: JumpExtra) {
    selectionStore.jumpTo(type, id, extra)
    if (type === 'project') {
      router.push(`/projects/${id}`)
    } else {
      router.push(typeRouteMap[type])
    }
  }

  return { navigateTo }
}
