import { invoke } from '@tauri-apps/api/core'

/**
 * 粘贴文件分片上传 helper
 *
 * 替代「整文件 arrayBuffer → invoke({bytes})」的一次性 IPC——Tauri 会把 Uint8Array
 * JSON 序列化成数字数组（每字节一个 JSON 数字），主线程三次 O(文件大小) 操作会冻结界面。
 * 分片（1MB）+ 片间 await 让主线程自然让出，同时每片完成即回调进度。
 *
 * 后端：chat_save_temp_chunk（file_commands.rs），路径由 upload_id 派生，末片返回 {path, name}。
 * 大小上限与后端 PASTE_MAX_BYTES（20MB）保持一致。
 */

export const MAX_PASTE_BYTES = 20 * 1024 * 1024
const CHUNK_BYTES = 1024 * 1024

export interface PasteUploadResult {
  path: string
  name: string
}

/** 把 ArrayBuffer 转 base64（分块避免大数组 spread 栈溢出） */
function toBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf)
  let binary = ''
  const step = 0x8000
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + step) as unknown as number[])
  }
  return btoa(binary)
}

/**
 * 分片上传一个粘贴的文件到临时目录
 * @param file   浏览器 File（粘贴来源）
 * @param ext    扩展名（不含点）
 * @param onProgress 0-100 进度回调
 * @param isCancelled 返回 true 时中断上传并清理临时文件
 */
export async function uploadPasteFile(
  file: File,
  ext: string,
  onProgress: (pct: number) => void,
  isCancelled?: () => boolean
): Promise<PasteUploadResult> {
  if (file.size === 0) throw new Error('文件数据为空')
  if (file.size > MAX_PASTE_BYTES) {
    throw new Error(`文件超过 ${Math.floor(MAX_PASTE_BYTES / 1024 / 1024)}MB 上限`)
  }

  const uploadId = crypto.randomUUID()
  const total = Math.max(1, Math.ceil(file.size / CHUNK_BYTES))
  const origName = file.name || null
  let lastPath = ''

  for (let i = 0; i < total; i++) {
    if (isCancelled?.()) {
      await invoke('chat_save_temp_abort', { uploadId, ext }).catch(() => {})
      throw new Error('已取消')
    }
    const start = i * CHUNK_BYTES
    const chunk = file.slice(start, Math.min(start + CHUNK_BYTES, file.size))
    const dataB64 = toBase64(await chunk.arrayBuffer())
    const r: any = await invoke('chat_save_temp_chunk', {
      uploadId,
      dataB64,
      ext,
      origName,
      index: i,
      total,
    })
    if (r?.path) lastPath = r.path
    onProgress(Math.round(((i + 1) / total) * 100))
  }

  if (!lastPath) throw new Error('上传失败：未返回文件路径')
  const name = file.name || lastPath.replace(/^.*[\\\/]/, '')
  return { path: lastPath, name }
}
