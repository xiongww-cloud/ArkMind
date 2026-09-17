/**
 * 华为 SCT 报价单硬件/软件型号解析工具
 *
 * 硬件型号解析规则（parseHardwareName）：
 *   1. 去掉中间序列号 `_<数字>_`（如 `_1_` → `_`）
 *   2. 最后一个 `_` 之后为 customName（站点 / 角色 / 备注）
 *   3. 型号主体剥离版本号 V\d+R\d+ 及其后内容
 *   4. 剥离尾部残留序列号 `_数字`
 *   5. 剥离括号说明（中英文括号及其中内容）
 *   —— 保留型号数字（如 2000 / 5310 / 8），它是产品型号的一部分
 *
 * 示例 → 输出：
 *   'OceanStor Dorado 2000 V700R001_站点1'
 *     → { cleanModel: 'OceanStor Dorado 2000', customName: '站点1' }
 *   'CloudEngine S8700-6C-EI_1 V200R019_核心交换机'
 *     → { cleanModel: 'CloudEngine S8700-6C-EI', customName: '核心交换机' }
 *   'OceanStor 5310'
 *     → { cleanModel: 'OceanStor 5310', customName: null }
 *   '灾备服务(站点内报价)_站点1'
 *     → { cleanModel: '灾备服务', customName: '站点1' }
 */

/** 删除第一个中英文括号 '(' '（' 及其后全部内容 */
function stripParentheses(s) {
  const half = s.indexOf('(')
  const full = s.indexOf('（')
  let cut = s.length
  if (half !== -1 && half < cut) cut = half
  if (full !== -1 && full < cut) cut = full
  return cut < s.length ? s.substring(0, cut) : s
}

/**
 * 解析华为硬件型号字符串。
 * @param {string} rawString
 * @returns {{ cleanModel: string, customName: string|null }}
 */
export function parseHardwareName(rawString) {
  if (!rawString || typeof rawString !== 'string') {
    return { cleanModel: '', customName: null }
  }

  let cleaned = rawString.trim()
  cleaned = cleaned.replace(/_(\d+)_/g, '_')

  const lastUnderscore = cleaned.lastIndexOf('_')
  let customName = null
  let working = cleaned
  if (lastUnderscore !== -1) {
    customName = cleaned.substring(lastUnderscore + 1).trim() || null
    working = cleaned.substring(0, lastUnderscore)
  }

  working = working.replace(/\s*V\d+R\d+.*$/i, '')
  working = working.replace(/_\d+$/g, '')
  working = stripParentheses(working)

  return { cleanModel: working.trim(), customName }
}

/**
 * 解析软件许可行：与硬件一致地拆站点 / 剥版本号 / 去括号，
 * 尾部数字属于型号（如 'DCS虚拟化套件 8'）自然保留。
 * @param {string} rawString
 * @returns {{ cleanModel: string, customName: string|null }}
 */
export function parseLicenseName(rawString) {
  if (!rawString || typeof rawString !== 'string') {
    return { cleanModel: '', customName: null }
  }

  let cleaned = rawString.trim()
  cleaned = cleaned.replace(/_(\d+)_/g, '_')

  const lastUnderscore = cleaned.lastIndexOf('_')
  let customName = null
  if (lastUnderscore !== -1) {
    customName = cleaned.substring(lastUnderscore + 1).trim() || null
    cleaned = cleaned.substring(0, lastUnderscore)
  }

  cleaned = cleaned.replace(/\s*V\d+R\d+.*$/i, '')
  cleaned = stripParentheses(cleaned)

  return { cleanModel: cleaned.trim(), customName }
}

export function parseHardwareNames(rawNames) {
  return (rawNames || []).map(parseHardwareName)
}

export function parseLicenseNames(rawNames) {
  return (rawNames || []).map(parseLicenseName)
}

export default parseHardwareName
