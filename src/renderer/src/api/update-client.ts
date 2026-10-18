import { invoke } from '@tauri-apps/api/core'

export interface UpdateCheckResult {
  has_update: boolean
  latest: string
  url: string
}

/** 检查更新：查公开分发仓库最新已发布版本；网络失败抛错（调用方决定静默或提示） */
export function checkUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>('update_check')
}
