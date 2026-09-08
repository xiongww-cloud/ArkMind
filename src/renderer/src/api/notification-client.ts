import { invoke } from '@tauri-apps/api/core'

export interface NotificationAction {
  type: string
  label: string
  params?: Record<string, unknown>
}

export interface Notification {
  id: string
  type: string
  title: string
  content: string
  entity_type: string
  entity_id: string
  priority: string
  read: number
  source: string
  follow_up_of: string
  created_at: string
  meta?: string | null
  actions?: NotificationAction[]
}

export interface SuggestionMeta {
  id: string
  kind: 'todo' | 'project' | 'partner'
  status: 'pending' | 'done' | 'ignored'
  result?: { type: string; id: string; name: string }
  note?: string
  [key: string]: unknown
}

export interface NotificationStats {
  success: boolean
  unread_count: number
  today_calls: number
  today_tokens: number
}

export interface NotificationConfig {
  success: boolean
  config: Record<string, string>
}

export async function notificationList(unreadOnly?: boolean): Promise<{ success: boolean; notifications: Notification[] }> {
  return invoke('notification_list', { unreadOnly: unreadOnly ?? null })
}

export async function notificationCreate(data: Record<string, unknown>): Promise<{ success: boolean; id: string }> {
  return invoke('notification_create', { data })
}

export async function notificationMarkRead(id: string): Promise<void> {
  return invoke('notification_mark_read', { id })
}

export async function notificationMarkAllRead(): Promise<void> {
  return invoke('notification_mark_all_read')
}

export async function notificationDelete(id: string): Promise<void> {
  return invoke('notification_delete', { id })
}

export async function notificationBatchDelete(ids: string[]): Promise<void> {
  return invoke('notification_batch_delete', { ids })
}

export async function notificationStats(): Promise<NotificationStats> {
  return invoke('notification_stats')
}

export async function notificationConfigGet(): Promise<NotificationConfig> {
  return invoke('notification_config_get')
}

export async function notificationConfigUpdate(data: Record<string, unknown>): Promise<void> {
  return invoke('notification_config_update', { data })
}

export interface LoopStatus {
  success: boolean
  running: boolean
  last_check_at: string
  next_check_at: string
}

export async function notificationLoopStatus(): Promise<LoopStatus> {
  return invoke('notification_loop_status')
}

// ── 邮件 AI 建议逐条确认 ──

export async function emailActionConfirm(
  notificationId: string,
  suggestionId: string
): Promise<{ success: boolean; items?: SuggestionMeta[]; object?: { type: string; id: string; name: string }; note?: string; linked_files?: number }> {
  return invoke('email_action_confirm', { notificationId, suggestionId })
}

export async function emailActionIgnore(
  notificationId: string,
  suggestionId: string
): Promise<{ success: boolean; items?: SuggestionMeta[]; note?: string }> {
  return invoke('email_action_ignore', { notificationId, suggestionId })
}
