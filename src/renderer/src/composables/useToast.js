import { ref } from 'vue'

const toasts = ref([])
let nextId = 0

const TOAST_DURATION = 2500

function remove(id) {
  const idx = toasts.value.findIndex(t => t.id === id)
  if (idx !== -1) toasts.value.splice(idx, 1)
}

function push(type, message) {
  const id = ++nextId
  toasts.value.push({ id, type, message })
  setTimeout(() => remove(id), TOAST_DURATION)
  return id
}

/** Lightweight success toast. Auto-dismisses after 2.5 s. */
export function toast({ type, message }) {
  push(type, message)
}

toast.success = (message) => push('success', message)
toast.error = (message) => push('error', message)
toast.info = (message) => push('info', message)

export function useToast() {
  return { toasts }
}
