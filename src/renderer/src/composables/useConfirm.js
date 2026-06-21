import { ref } from 'vue'

const state = ref({
  show: false,
  title: '',
  message: '',
  type: 'confirm', // 'confirm' | 'alert'
  confirmText: '确认',
  cancelText: '取消',
  resolve: null,
})

let id = 0

/**
 * Show a blocking confirmation dialog. Returns a Promise that resolves to
 * `true` if the user clicked confirm, `false` otherwise.
 *
 * Usage: const ok = await confirm({ title: '删除', message: '确定要删除吗？' })
 */
export function confirm({ title, message, confirmText, cancelText } = {}) {
  return new Promise((resolve) => {
    state.value = {
      show: true,
      title: title || '确认操作',
      message: message || '',
      type: 'confirm',
      confirmText: confirmText || '确认',
      cancelText: cancelText || '取消',
      resolve,
    }
  })
}

/**
 * Show a blocking danger confirmation dialog (red confirm button).
 */
export function dangerConfirm({ title, message, confirmText, cancelText } = {}) {
  return new Promise((resolve) => {
    state.value = {
      show: true,
      title: title || '确认操作',
      message: message || '',
      type: 'confirm',
      danger: true,
      confirmText: confirmText || '确认删除',
      cancelText: cancelText || '取消',
      resolve,
    }
  })
}

/**
 * Show a blocking alert dialog. Resolves when dismissed.
 */
export function alert({ title, message, confirmText } = {}) {
  return new Promise((resolve) => {
    state.value = {
      show: true,
      title: title || '提示',
      message: message || '',
      type: 'alert',
      confirmText: confirmText || '知道了',
      cancelText: '',
      resolve,
    }
  })
}

export function useConfirmDialog() {
  function handleConfirm() {
    state.value.resolve?.(true)
    state.value.show = false
  }

  function handleCancel() {
    state.value.resolve?.(false)
    state.value.show = false
  }

  return { state, handleConfirm, handleCancel }
}
