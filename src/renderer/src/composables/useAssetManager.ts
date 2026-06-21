/**
 * useAssetManager — 素材管线组合式函数
 *
 * 统一管理演示文稿模块中的所有图片资源操作：
 *   - 本地文件 → Base64 转换（MVP 阶段）
 *   - 文件校验（类型 / 大小）
 *   - 上传状态追踪
 *
 * ── 未来迁移路径 ──────────────────────────────────────────────
 *  TODO: MVP 结束后，将 uploadImage 替换为 POST /api/assets
 *        后端存入本地文件系统或 OSS，返回永久 URL。
 *        当前 Base64 方案仅适用于单机原型验证阶段。
 *        - 接口: POST /api/assets  { file: FormData }
 *        - 返回: { id, url, fileName, mimeType, size }
 *        - 存储: SQLite AssetImage 表 (base64 字段 → filePath 字段)
 * ──────────────────────────────────────────────────────────────
 */

import { ref } from 'vue'

// ═══════════════════════════════════════════════════════════════════════════
//  CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/** 支持的图片 MIME 类型 */
const ALLOWED_TYPES = ['image/png', 'image/jpeg', 'image/svg+xml', 'image/webp', 'image/gif']

/** 支持的图片扩展名 */
const ALLOWED_EXTENSIONS = ['.png', '.jpg', '.jpeg', '.svg', '.webp', '.gif']

/** 单文件最大体积 (20 MB) */
const MAX_FILE_SIZE = 20 * 1024 * 1024

// ═══════════════════════════════════════════════════════════════════════════
//  INTERFACES
// ═══════════════════════════════════════════════════════════════════════════

export interface UploadResult {
  /** Base64 data URL (MVP) 或远程 URL (未来) */
  url: string
  /** 原始文件名 */
  fileName: string
  /** MIME 类型 */
  mimeType: string
  /** 文件大小 (bytes) */
  size: number
}

export interface UploadError {
  message: string
  code: 'INVALID_TYPE' | 'FILE_TOO_LARGE' | 'READ_ERROR' | 'UPLOAD_FAILED'
}

// ═══════════════════════════════════════════════════════════════════════════
//  COMPOSABLE
// ═══════════════════════════════════════════════════════════════════════════

export function useAssetManager() {
  const isUploading = ref(false)
  const uploadError = ref<UploadError | null>(null)
  const lastResult = ref<UploadResult | null>(null)

  /**
   * 校验文件合法性
   */
  function validateFile(file: File): UploadError | null {
    // 类型校验 — 同时检查 MIME 和扩展名（SVG 的 MIME 可能不标准）
    const ext = '.' + (file.name.split('.').pop() || '').toLowerCase()
    const mimeOk = ALLOWED_TYPES.includes(file.type)
    const extOk = ALLOWED_EXTENSIONS.includes(ext)
    if (!mimeOk && !extOk) {
      return {
        message: `不支持的文件类型: ${file.type || ext}。支持 PNG, JPG, SVG, WebP, GIF`,
        code: 'INVALID_TYPE',
      }
    }

    // 体积校验
    if (file.size > MAX_FILE_SIZE) {
      const mb = (file.size / (1024 * 1024)).toFixed(1)
      return {
        message: `文件过大 (${mb} MB)，最大支持 20 MB`,
        code: 'FILE_TOO_LARGE',
      }
    }

    return null
  }

  /**
   * 上传图片 (MVP: FileReader → Base64)
   *
   * @param file - 用户选择的图片文件
   * @returns Promise<UploadResult> — 包含 Base64 URL 的上传结果
   *
   * TODO (production): 替换为后端 API 调用
   *   const formData = new FormData()
   *   formData.append('file', file)
   *   const res = await fetch('/api/assets', { method: 'POST', body: formData })
   *   return res.json()
   */
  function uploadImage(file: File): Promise<UploadResult> {
    return new Promise((resolve, reject) => {
      // 1. 校验
      const validationError = validateFile(file)
      if (validationError) {
        uploadError.value = validationError
        reject(validationError)
        return
      }

      // 2. 重置状态
      isUploading.value = true
      uploadError.value = null

      // 3. FileReader → Base64 (MVP)
      const reader = new FileReader()

      reader.onload = () => {
        const url = reader.result as string
        const result: UploadResult = {
          url,
          fileName: file.name,
          mimeType: file.type || 'image/png',
          size: file.size,
        }
        lastResult.value = result
        isUploading.value = false
        resolve(result)
      }

      reader.onerror = () => {
        const err: UploadError = {
          message: '文件读取失败，请重试',
          code: 'READ_ERROR',
        }
        uploadError.value = err
        isUploading.value = false
        reject(err)
      }

      reader.readAsDataURL(file)
    })
  }

  /**
   * 批量上传
   *
   * TODO (production): 替换为后端批量接口 POST /api/assets/batch
   */
  async function uploadImages(files: File[]): Promise<UploadResult[]> {
    const results: UploadResult[] = []
    for (const file of files) {
      try {
        const result = await uploadImage(file)
        results.push(result)
      } catch (_) {
        // 单文件失败不中断批量流程
        continue
      }
    }
    return results
  }

  /**
   * 清除错误状态
   */
  function clearError() {
    uploadError.value = null
  }

  return {
    // State
    isUploading,
    uploadError,
    lastResult,
    // Methods
    uploadImage,
    uploadImages,
    validateFile,
    clearError,
    // Constants
    ALLOWED_TYPES,
    ALLOWED_EXTENSIONS,
    MAX_FILE_SIZE,
  }
}
