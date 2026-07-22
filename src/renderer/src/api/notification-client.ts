import { invoke } from '@tauri-apps/api/core'

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

export async function notificationStats(): Promise<NotificationStats> {
  return invoke('notification_stats')
}

export async function notificationConfigGet(): Promise<NotificationConfig> {
  return invoke('notification_config_get')
}

export async function notificationConfigUpdate(data: Record<string, unknown>): Promise<void> {
  return invoke('notification_config_update', { data })
}
