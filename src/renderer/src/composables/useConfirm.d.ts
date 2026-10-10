export interface DangerConfirmOptions {
  title?: string
  message: string
  confirmText?: string
  cancelText?: string
}

export declare function dangerConfirm(options: string | DangerConfirmOptions): Promise<boolean>
export declare function confirm(options: string | DangerConfirmOptions): Promise<boolean>
export declare function alert(options: string | DangerConfirmOptions): Promise<void>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export declare function useConfirmDialog(): any
