import { ref } from 'vue'

const projectModalId = ref<string | null>(null)

export function useLinkModals() {
  function openProject(id: string) { projectModalId.value = id }
  function closeProject() { projectModalId.value = null }
  return { projectModalId, openProject, closeProject }
}
