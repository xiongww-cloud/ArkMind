/**
 * PMS 核心数据清洗函数
 *
 * 策略：
 *   1. 自定义名称 = 最后一个 `_` 之后的文本
 *   2. 如果文本中存在 `(`，删除括号及其之后的所有文本
 *   3. 剩余文本按空格拆分，设备型号 = 前两个词（中间加空格）
 *
 * 输入示例：
 *   'CloudEngine S8700-6(China)_核心交换机'
 *     → { cleanModel: 'CloudEngine S8700-6', customName: '核心交换机' }
 *   '1 V600R025_办公核心交换机'
 *     → { cleanModel: '1 V600R025', customName: '办公核心交换机' }
 *   'OceanStor 5310'
 *     → { cleanModel: '', customName: null }
 */
export function parseHardwareName(rawString) {
  if (!rawString || typeof rawString !== 'string') {
    return { cleanModel: '', customName: null }
  }

  let cleaned = rawString

  // 0. 删除 _数字_ 中的 _数字（华为 BOM 特有模式）
  cleaned = cleaned.replace(/_(\d+)_/g, '_')

  // 1. 自定义名称 = 最后一个 `_` 之后的文本
  let customName = null
  let working = cleaned
  const lastUnderscore = cleaned.lastIndexOf('_')
  if (lastUnderscore !== -1) {
    customName = cleaned.substring(lastUnderscore + 1).trim() || null
    working = cleaned.substring(0, lastUnderscore)
  }

  // 2. 删除 `(` 及其之后的所有文本
  const parenIdx = working.indexOf('(')
  if (parenIdx !== -1) {
    working = working.substring(0, parenIdx)
  }
  const chParenIdx = working.indexOf('（')
  if (chParenIdx !== -1) {
    working = working.substring(0, chParenIdx)
  }

  // 3. 按空格拆分，取前两个词作为型号
  const words = working.trim().split(/\s+/).filter(Boolean)
  const cleanModel = words.length >= 2
    ? `${words[0]} ${words[1]}`
    : (words.length === 1 ? words[0] : '')

  return { cleanModel, customName }
}

/**
 * 设备型号提取：删括号 → 删下划线后缀 → 按空格拆分 → 取前两个词拼接
 *
 * 示例：
 *   'CloudEngine S8700-6(China) 核心交换机' → 'CloudEngine S8700-6'
 *   'S12700E(交流)'                          → 'S12700E'
 *
 * @param {string} text - 含设备型号的原始文本
 * @returns {string} 提取的型号；无有效词时返回空串
 */
export function extractModelByWords(text) {
  if (!text || typeof text !== 'string') return ''

  let cleaned = text

  // 0. 删除 _数字_ 中的 _数字
  cleaned = cleaned.replace(/_(\d+)_/g, '_')

  // 1. 若含下划线，取左侧部分
  const underscoreIdx = cleaned.lastIndexOf('_')
  if (underscoreIdx !== -1) {
    cleaned = cleaned.substring(0, underscoreIdx)
  }

  // 2. 删除 `(` 及其之后的所有文本
  const parenIdx = cleaned.indexOf('(')
  if (parenIdx !== -1) {
    cleaned = cleaned.substring(0, parenIdx)
  }
  const chParenIdx = cleaned.indexOf('（')
  if (chParenIdx !== -1) {
    cleaned = cleaned.substring(0, chParenIdx)
  }

  // 3. 按空格拆分，取前两个词
  const words = cleaned.split(/\s+/).filter(Boolean)
  if (words.length === 0) return ''
  return words.length >= 2
    ? `${words[0]} ${words[1]}`
    : words[0]
}

/**
 * Batch parse an array of raw name strings.
 * @param {string[]} rawNames
 * @returns {Array<{ cleanModel: string, customName: string|null }>}
 */
export function parseHardwareNames(rawNames) {
  return rawNames.map(parseHardwareName)
}
