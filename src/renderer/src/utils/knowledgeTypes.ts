/**
 * 知识对象类型的统一标签与配色（知识观测台/图谱共用）
 * 口径 = source_data.source_type 实际值域（MEMORY/POLICY 已迁出，不再列出）
 */
export const KNOWLEDGE_TYPE_LABEL: Record<string, string> = {
  FILE: '文件',
  EMAIL: '邮件',
  NOTE: '笔记',
  PRODUCT: '产品',
  PROJECT: '项目',
  TODO: '待办',
  CONTACT: '联系人',
  CHANNEL: '渠道商',
  VENDOR: '供应商',
  CUSTOMER: '客户',
  CHAT: '对话',
}

export const KNOWLEDGE_TYPE_COLOR: Record<string, string> = {
  FILE: '#60a5fa',
  EMAIL: '#f59e0b',
  NOTE: '#fbbf24',
  PRODUCT: '#a78bfa',
  PROJECT: '#34d399',
  TODO: '#94a3b8',
  CONTACT: '#14b8a6',
  CHANNEL: '#06b6d4',
  VENDOR: '#8b5cf6',
  CUSTOMER: '#f43f5e',
  CHAT: '#ec4899',
}

export function knowledgeTypeLabel(t: string): string {
  return KNOWLEDGE_TYPE_LABEL[t] || t || '未知'
}

export function knowledgeTypeColor(t: string): string {
  return KNOWLEDGE_TYPE_COLOR[t] || '#94a3b8'
}
