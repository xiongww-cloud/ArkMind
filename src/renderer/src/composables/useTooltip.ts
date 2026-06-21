import { ref } from 'vue'

const tooltipText = ref('')
const tooltipX = ref(0)
const tooltipY = ref(0)
const tooltipVisible = ref(false)

export function useTooltip() {
  function show(el: HTMLElement, text: string) {
    const r = el.getBoundingClientRect()
    tooltipText.value = text
    tooltipX.value = r.left + r.width / 2
    tooltipY.value = r.top - 6
    tooltipVisible.value = true
  }
  function hide() {
    tooltipVisible.value = false
  }
  return { tooltipText, tooltipX, tooltipY, tooltipVisible, show, hide }
}
