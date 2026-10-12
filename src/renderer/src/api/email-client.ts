import { invoke } from '@tauri-apps/api/core'

export interface EmailAccount {
  id: string
  name: string
  email_address: string
  imap_host: string
  imap_port: number
  username: string
  sync_enabled: number
  retention_months: number
  last_sync_at: string
}

export interface Email {
  id: string
  account_id: string
  folder: string
  from_addr: string
  from_name: string
  to_addr: string
  cc_addr: string
  subject: string
  received_at: string
  is_read: number
  has_attachments: number
  ai_summary: string
  ai_category: string
  created_at: string
}

export interface EmailDetail extends Email {
  body_text: string
  body_html: string
  message_id: string
}

export interface EmailAttachment {
  id: string
  filename: string
  mime_type: string
  size: number
  file_asset_id: string | null
  is_inline: number
}

export async function emailAddAccount(data: {
  name: string
  emailAddress: string
  imapHost: string
  imapPort: number
  username: string
  password: string
  retentionMonths: number
}): Promise<{ success: boolean; id?: string; error?: string }> {
  return invoke('email_add_account', { data })
}

export async function emailListAccounts(): Promise<{
  success: boolean
  accounts: EmailAccount[]
}> {
  return invoke('email_list_accounts')
}

export async function emailUpdateAccount(
  id: string,
  data: Record<string, unknown>
): Promise<{ success: boolean }> {
  return invoke('email_update_account', { id, data })
}

export async function emailDeleteAccount(
  id: string
): Promise<{ success: boolean }> {
  return invoke('email_delete_account', { id })
}

export async function emailList(params: {
  accountId?: string
  folder?: string
  category?: string
  limit?: number
  offset?: number
}): Promise<{ success: boolean; emails: Email[]; total: number }> {
  return invoke('email_list', {
    accountId: params.accountId,
    folder: params.folder,
    category: params.category,
    limit: params.limit,
    offset: params.offset,
  })
}

export async function emailGetDetail(
  id: string
): Promise<{ success: boolean; email: EmailDetail; attachments: EmailAttachment[] }> {
  return invoke('email_get_detail', { id })
}

export async function emailRenderHtml(
  id: string
): Promise<{ success: boolean; html: string; lazy?: boolean }> {
  return invoke('email_render_html', { id })
}

export async function emailInlineAsset(
  emailId: string,
  token: string
): Promise<{ success: boolean; dataUrl?: string; error?: string }> {
  return invoke('email_inline_asset', { emailId, token })
}

export async function emailDelete(
  id: string
): Promise<{ success: boolean }> {
  return invoke('email_delete', { id })
}

export async function emailSync(
  accountId?: string
): Promise<{ success: boolean; synced: number }> {
  return invoke('email_sync', { accountId })
}

export async function emailLoopStatus(): Promise<{
  running: boolean
  last_check_at: string
  next_check_at: string
  interval_minutes: number
}> {
  return invoke('email_loop_status')
}

export async function emailConfigGet(): Promise<Record<string, string>> {
  return invoke('email_config_get')
}

export async function emailConfigUpdate(
  data: Record<string, string>
): Promise<{ success: boolean }> {
  return invoke('email_config_update', { data })
}

export async function emailStats(): Promise<{
  success: boolean
  total: number
  unread: number
  knowledge: number
  pending: number
}> {
  return invoke('email_stats')
}
