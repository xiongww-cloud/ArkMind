import type { Ref } from 'vue'

export interface ToastOptions {
  type?: 'success' | 'error' | 'warning' | 'info'
  message: string
}

export function toast(opts: ToastOptions): number
export namespace toast {
  function success(message: string): number
  function error(message: string): number
  function info(message: string): number
}

export function useToast(): {
  toasts: Ref<Array<{ id: number; type?: string; message: string }>>
}
