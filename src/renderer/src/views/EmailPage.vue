<script setup lang="ts">
import { ref, computed, onMounted, watch, onActivated, nextTick } from 'vue'
import { useRoute } from 'vue-router'
import AppIcon from '@/components/common/AppIcon.vue'
import { lazyLoadEmailImages } from '@/utils/emailLazyImages'
import { renderedCacheGet, renderedCacheSet } from '@/utils/emailImageCache'
import {
  emailList,
  emailListAccounts,
  emailSync,
  emailGetDetail,
  emailRenderHtml,
  emailStats,
  type Email,
  type EmailAccount,
  type EmailDetail,
  type EmailAttachment
} from '@/api/email-client'
import { previewFile, copyFileToClipboard, exportFile } from '@/api/db-client'
import { toast } from '@/composables/useToast'
import DOMPurify from 'dompurify'

// ═══════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════

interface AccountState {
  account: EmailAccount
  expanded: boolean
  inboxCount: number
  sentCount: number
}

const accounts = ref<EmailAccount[]>([])
const accountStates = ref<Map<string, AccountState>>(new Map())
const emails = ref<Email[]>([])
const total = ref(0)
const loading = ref(false)
const loadingMore = ref(false)
const hasMore = ref(false)
const syncing = ref(false)
const detailLoading = ref(false)

const activeAccountId = ref<string>('')
const activeFolder = ref<'INBOX' | 'Sent'>('INBOX')
const searchQuery = ref('')
const selectedEmailId = ref<string | null>(null)
const selectedEmail = ref<EmailDetail | null>(null)
const attachments = ref<EmailAttachment[]>([])

// Column widths
const sidebarWidth = ref(180)
const listWidth = ref(280)
let resizing: { panel: string; startX: number; startW: number } | null = null

// ═══════════════════════════════════════════════════════════════
// Computed
// ═══════════════════════════════════════════════════════════════

const filteredEmails = computed(() => {
  let list = emails.value
  if (searchQuery.value) {
    const q = searchQuery.value.toLowerCase()
    list = list.filter(
      (e) =>
        e.subject.toLowerCase().includes(q) ||
        e.from_name.toLowerCase().includes(q) ||
        e.from_addr.toLowerCase().includes(q)
    )
  }
  return list
})

const renderedHtml = ref('')
const emailBodyEl = ref<HTMLElement | null>(null)
let cancelLazy: (() => void) | null = null
let lazyEmailId: string | null = null

function sanitizeEmailHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    ADD_TAGS: ['img', 'a'],
    ADD_ATTR: ['src', 'alt', 'width', 'height', 'href', 'target', 'id', 'style', 'border'],
    ADD_DATA_URI_TAGS: ['img'],
  })
}

// 正文 DOM 真正挂载后再启动懒加载（避免 detailLoading 期间容器不存在）
function startLazyIfPending() {
  if (!lazyEmailId) return
  cancelLazy?.()
  cancelLazy = lazyLoadEmailImages(emailBodyEl.value, lazyEmailId)
}

async function loadRenderedHtml(id: string) {
  cancelLazy?.()
  cancelLazy = null
  lazyEmailId = null
  renderedHtml.value = ''
  try {
    let html = ''
    let lazy = false
    const cached = renderedCacheGet(id)
    if (cached) {
      html = cached.html
      lazy = !!cached.lazy
    } else {
      const r = await emailRenderHtml(id)
      html = r?.html || ''
      lazy = !!r?.lazy
      if (html) renderedCacheSet(id, html, lazy)
    }
    renderedHtml.value = html ? sanitizeEmailHtml(html) : ''
    if (lazy) lazyEmailId = id
  } catch (e) {
    console.error('渲染邮件正文失败:', e)
    renderedHtml.value = ''
  }
}

const accountList = computed(() => {
  return accounts.value.map(acc => ({
    ...acc,
    expanded: accountStates.value.get(acc.id)?.expanded ?? true,
    inboxCount: accountStates.value.get(acc.id)?.inboxCount ?? 0,
    sentCount: accountStates.value.get(acc.id)?.sentCount ?? 0,
  }))
})

const totalStats = computed(() => {
  let totalEmails = 0
  accountStates.value.forEach(state => {
    totalEmails += state.inboxCount + state.sentCount
  })
  return { total: totalEmails }
})

// ═══════════════════════════════════════════════════════════════
// Resize handlers
// ═══════════════════════════════════════════════════════════════

function onResizeStart(panel: string, e: MouseEvent) {
  e.preventDefault()
  resizing = {
    panel,
    startX: e.clientX,
    startW: panel === 'sidebar' ? sidebarWidth.value : listWidth.value,
  }
  document.addEventListener('mousemove', onResizeMove)
  document.addEventListener('mouseup', onResizeEnd)
}

function onResizeMove(e: MouseEvent) {
  if (!resizing) return
  const dx = e.clientX - resizing.startX
  if (resizing.panel === 'sidebar') {
    sidebarWidth.value = Math.max(160, Math.min(280, resizing.startW + dx))
  } else {
    listWidth.value = Math.max(240, Math.min(480, resizing.startW + dx))
  }
}

function onResizeEnd() {
  resizing = null
  document.removeEventListener('mousemove', onResizeMove)
  document.removeEventListener('mouseup', onResizeEnd)
}

// ═══════════════════════════════════════════════════════════════
// Data loading
// ═══════════════════════════════════════════════════════════════

async function loadAccounts() {
  try {
    const res = await emailListAccounts()
    if (res.success) {
      accounts.value = res.accounts || []
      // 初始化账户状态（所有账户默认展开）
      const newStates = new Map<string, AccountState>()
      for (const acc of accounts.value) {
        const existing = accountStates.value.get(acc.id)
        newStates.set(acc.id, {
          account: acc,
          expanded: existing?.expanded ?? true,
          inboxCount: 0,
          sentCount: 0,
        })
      }
      accountStates.value = newStates
      
      // 如果没有选中的账户，默认选中第一个
      if (!activeAccountId.value && accounts.value.length > 0) {
        activeAccountId.value = accounts.value[0].id
      }
      
      // 加载每个账户的统计信息
      await loadAccountStats()
    }
  } catch {}
}

async function loadAccountStats() {
  for (const acc of accounts.value) {
    try {
      // 加载收件箱统计
      const inboxRes = await emailList({
        accountId: acc.id,
        folder: 'INBOX',
        limit: 1,
      })
      
      // 加载已发送统计
      const sentRes = await emailList({
        accountId: acc.id,
        folder: 'Sent',
        limit: 1,
      })
      
      const state = accountStates.value.get(acc.id)
      if (state) {
        state.inboxCount = inboxRes.total || 0
        state.sentCount = sentRes.total || 0
      }
    } catch {}
  }
}

let emailsLoadSeq = 0
async function loadEmails(append = false) {
  const mySeq = ++emailsLoadSeq
  if (append) {
    loadingMore.value = true
  } else {
    loading.value = true
  }
  try {
    const offset = append ? emails.value.length : 0
    const res = await emailList({
      accountId: activeAccountId.value,
      folder: activeFolder.value,
      limit: 20,
      offset,
    })
    if (mySeq !== emailsLoadSeq) return // 账户/文件夹已切换，丢弃过期响应
    const newEmails = res.emails || []
    if (append) {
      emails.value = [...emails.value, ...newEmails]
    } else {
      emails.value = newEmails
    }
    total.value = res.total || 0
    hasMore.value = emails.value.length < total.value
  } catch (e) {
    console.error('加载邮件失败:', e)
  } finally {
    if (mySeq === emailsLoadSeq) {
      loading.value = false
      loadingMore.value = false
    }
  }
}

function loadMore() {
  if (loadingMore.value || !hasMore.value) return
  loadEmails(true)
}

function onListScroll(e: Event) {
  const el = e.target as HTMLElement
  if (!el) return
  // 距底部不足 80px 时触发加载更多
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 80) {
    loadMore()
  }
}

async function loadDetail(id: string) {
  detailLoading.value = true
  try {
    const res = await emailGetDetail(id)
    if (res.success) {
      selectedEmail.value = res.email
      // 过滤掉图片附件（内嵌图片和图片附件都会在正文中渲染）
      attachments.value = (res.attachments || []).filter((att: EmailAttachment) => {
        // 如果标记为内嵌，过滤掉
        if (att.is_inline) return false
        // 如果是图片类型，也过滤掉（会在正文中渲染）
        if (att.mime_type && att.mime_type.startsWith('image/')) return false
        return true
      })
      await loadRenderedHtml(id)
    }
  } catch (e) {
    console.error('加载邮件详情失败:', e)
  } finally {
    detailLoading.value = false
    await nextTick()
    startLazyIfPending()
  }
}

async function handleSync() {
  syncing.value = true
  try {
    const r: any = await emailSync()
    await loadAccounts()
    await loadEmails()
    toast({ message: `同步完成，新增 ${r?.synced ?? 0} 封`, type: 'success' })
  } catch (e: any) {
    console.error('同步失败:', e)
    toast({ message: `邮箱同步失败：${e?.message || e}`, type: 'error' })
  } finally {
    syncing.value = false
  }
}


// ═══════════════════════════════════════════════════════════════
// UI handlers
// ═══════════════════════════════════════════════════════════════

function toggleAccount(accountId: string) {
  const state = accountStates.value.get(accountId)
  if (state) {
    state.expanded = !state.expanded
  }
}

function selectFolder(accountId: string, folder: 'INBOX' | 'Sent') {
  activeAccountId.value = accountId
  activeFolder.value = folder
  selectedEmailId.value = null
  selectedEmail.value = null
  renderedHtml.value = ''
  cancelLazy?.()
  cancelLazy = null
  lazyEmailId = null
}

function selectEmail(id: string) {
  selectedEmailId.value = id
  loadDetail(id)
}

// ═══════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════

function formatSize(bytes: number): string {
  if (bytes < 1024) return bytes + 'B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + 'KB'
  return (bytes / (1024 * 1024)).toFixed(1) + 'MB'
}

function formatDate(dateStr: string): string {
  if (!dateStr) return ''
  try {
    const d = new Date(dateStr)
    const now = new Date()
    const isToday = d.toDateString() === now.toDateString()
    if (isToday) {
      return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
    }
    return d.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' })
  } catch {
    return dateStr.slice(0, 10)
  }
}

function formatFullDate(dateStr: string): string {
  if (!dateStr) return ''
  try {
    return new Date(dateStr).toLocaleString('zh-CN')
  } catch {
    return dateStr
  }
}

async function openAttachment(att: EmailAttachment) {
  if (!att.file_asset_id) {
    console.warn('附件没有关联的文件资产:', att.filename)
    return
  }
  try {
    await previewFile(att.file_asset_id)
  } catch (e) {
    console.error('预览附件失败:', e)
  }
}

async function downloadAttachment(att: EmailAttachment) {
  if (!att.file_asset_id) return
  try {
    await exportFile(att.file_asset_id)
  } catch (e) {
    console.error('下载附件失败:', e)
    toast({ message: '下载失败', type: 'error' })
  }
}

async function copyAttachment(att: EmailAttachment) {
  if (!att.file_asset_id) return
  try {
    const r = await copyFileToClipboard(att.file_asset_id)
    if (r?.success) toast({ message: `已复制「${r.fileName || att.filename}」`, type: 'success' })
  } catch (e) {
    console.error('复制附件失败:', e)
    toast({ message: '复制失败', type: 'error' })
  }
}

// ═══════════════════════════════════════════════════════════════
// Watchers & Lifecycle
// ═══════════════════════════════════════════════════════════════

watch([activeAccountId, activeFolder], () => {
  loadEmails()
})

const route = useRoute()

async function handleRouteQuery() {
  const id = route.query.id as string
  const focus = route.query.focus as string
  const targetId = id || focus
  if (targetId) {
    // First try to load detail to get account info
    const detail = await emailGetDetail(targetId).catch(() => null)
    if (detail?.success && detail.email) {
      // Switch to the correct account if needed
      const emailAccountId = detail.email.account_id
      if (emailAccountId && emailAccountId !== activeAccountId.value) {
        activeAccountId.value = emailAccountId
      }
      // Switch to correct folder if needed
      const emailFolder = detail.email.folder
      if (emailFolder && emailFolder !== activeFolder.value) {
        activeFolder.value = emailFolder
      }
      // Load emails list for the correct account/folder
      await loadEmails()
      selectedEmailId.value = targetId
      selectedEmail.value = detail.email
      attachments.value = (detail.attachments || []).filter((att: EmailAttachment) => {
        if (att.is_inline) return false
        if (att.mime_type && att.mime_type.startsWith('image/')) return false
        return true
      })
      await loadRenderedHtml(targetId)
      await nextTick()
      startLazyIfPending()
    } else {
      // Fallback: try with current account
      if (!activeAccountId.value && accounts.value.length > 0) {
        activeAccountId.value = accounts.value[0].id
      }
      if (activeAccountId.value) {
        await loadEmails()
      }
      selectedEmailId.value = targetId
      await loadDetail(targetId)
    }
  }
}

onMounted(async () => {
  await loadAccounts()
  await handleRouteQuery()
})
watch(() => route.query, async () => {
  await handleRouteQuery()
})
</script>

<template>
  <div class="flex w-full h-full bg-white overflow-hidden rounded-xl border border-surface-200 shadow-sm">
    <!-- Sidebar: Accounts & Folders -->
    <aside class="flex-shrink-0 border-r border-surface-200 bg-surface-50/30 flex flex-col overflow-hidden relative" :style="{width:sidebarWidth+'px'}">
      <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('sidebar', $event)"></div>

      <!-- Header -->
      <div class="flex items-center justify-between shrink-0 px-3 h-10 border-b border-surface-100">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">邮箱</span>
        <button
          class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600 transition-colors"
          :disabled="syncing"
          @click="handleSync"
          :title="syncing ? '同步中...' : '同步邮件'"
        >
          <AppIcon name="refresh" size="12" stroke="2.5" :class="syncing ? 'animate-spin' : ''"/>
        </button>
      </div>

      <!-- Account tree -->
      <div class="flex-1 overflow-y-auto overflow-x-clip py-1">
        <div v-for="acc in accountList" :key="acc.id">
          <!-- Account header -->
          <div
            class="relative flex items-center gap-1.5 px-3 py-1.5 cursor-pointer text-xs transition-colors rounded-lg group"
            :class="'text-surface-600 hover:bg-surface-100'"
            @click="toggleAccount(acc.id)"
          >
            <button
              @click.stop="toggleAccount(acc.id)"
              class="shrink-0 w-3 h-3 flex items-center justify-center text-surface-400 hover:text-surface-600"
            >
              <AppIcon
                name="chevron-right"
                class="w-2.5 h-2.5 transition-transform"
                :class="acc.expanded ? 'rotate-90' : ''"
              />
            </button>
            <AppIcon name="envelope" size="13" class="shrink-0 text-surface-400" />
            <span class="truncate flex-1">{{ acc.name || acc.email_address }}</span>
          </div>
          
          <!-- Folder list (expanded) -->
          <template v-if="acc.expanded">
            <!-- 收件箱 -->
            <div
              class="relative flex items-center gap-1.5 px-2 py-1.5 cursor-pointer text-xs transition-colors rounded-lg group overflow-hidden"
              :class="activeAccountId === acc.id && activeFolder === 'INBOX' 
                ? 'bg-blue-50/70 text-brand-700 font-medium' 
                : 'text-surface-600 hover:bg-surface-100'"
              style="padding-left: 36px"
              @click="selectFolder(acc.id, 'INBOX')"
            >
              <div v-if="activeAccountId === acc.id && activeFolder === 'INBOX'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r" />
              <AppIcon name="inbox" size="13" class="shrink-0" :class="activeAccountId === acc.id && activeFolder === 'INBOX' ? 'text-brand-500' : 'text-surface-400'" />
              <span class="truncate flex-1">收件箱</span>
              <span v-if="acc.inboxCount > 0" class="text-[10px] text-surface-400">
                {{ acc.inboxCount }}
              </span>
            </div>
            
            <!-- 已发送 -->
            <div
              class="relative flex items-center gap-1.5 px-2 py-1.5 cursor-pointer text-xs transition-colors rounded-lg group overflow-hidden"
              :class="activeAccountId === acc.id && activeFolder === 'Sent' 
                ? 'bg-blue-50/70 text-brand-700 font-medium' 
                : 'text-surface-600 hover:bg-surface-100'"
              style="padding-left: 36px"
              @click="selectFolder(acc.id, 'Sent')"
            >
              <div v-if="activeAccountId === acc.id && activeFolder === 'Sent'" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r" />
              <AppIcon name="send" size="13" class="shrink-0" :class="activeAccountId === acc.id && activeFolder === 'Sent' ? 'text-brand-500' : 'text-surface-400'" />
              <span class="truncate flex-1">已发送</span>
              <span v-if="acc.sentCount > 0" class="text-[10px] text-surface-400">
                {{ acc.sentCount }}
              </span>
            </div>
          </template>
        </div>
        
        <!-- 空状态 -->
        <div v-if="accountList.length === 0" class="px-3 py-4 text-center">
          <p class="text-[11px] text-surface-400">暂无邮箱账户</p>
        </div>
      </div>

    </aside>

    <!-- Email List -->
    <div class="flex-shrink-0 flex flex-col overflow-hidden border-r border-surface-200 relative" :style="{width:listWidth+'px'}">
      <div class="absolute right-[-3px] top-0 bottom-0 w-[6px] cursor-col-resize z-10 select-none" @mousedown="onResizeStart('list', $event)"></div>

      <!-- List header -->
      <div class="flex items-center justify-between px-3 h-10 border-b border-surface-100 shrink-0">
        <span class="text-[11px] font-semibold text-surface-500 uppercase tracking-wider">
          {{ activeFolder === 'INBOX' ? '收件箱' : '已发送' }}
          <span class="text-surface-400 font-normal ml-1">{{ total }}</span>
        </span>
      </div>

      <!-- Search -->
      <div class="shrink-0 px-3 py-1.5 border-b border-surface-100">
        <div class="flex items-center gap-1.5 bg-white border border-surface-200 rounded-lg px-2.5 py-1">
          <AppIcon name="search" size="12" class="text-surface-400 flex-shrink-0"/>
          <input
            v-model="searchQuery"
            type="text"
            class="flex-1 bg-transparent text-[11px] outline-none placeholder-surface-300"
            placeholder="搜索邮件..."
          />
        </div>
      </div>

      <!-- Email list -->
      <div class="flex-1 overflow-y-auto p-2 space-y-1" @scroll="onListScroll">
        <div v-if="loading" class="flex items-center justify-center py-10">
          <span class="text-xs text-surface-400">加载中...</span>
        </div>
        <div
          v-else-if="filteredEmails.length === 0"
          class="flex flex-col items-center justify-center py-10"
        >
          <AppIcon name="mail" size="32" class="text-surface-200 mb-2"/>
          <span class="text-xs text-surface-400">暂无邮件</span>
        </div>
        <div
          v-for="email in filteredEmails"
          :key="email.id"
          class="flex items-start gap-3 px-3 py-2.5 rounded-lg cursor-pointer transition-all duration-200 group border bg-white shadow-sm relative overflow-hidden select-none"
          :class="[
            selectedEmailId === email.id
              ? 'bg-blue-50/70 border-brand-200 shadow-md scale-[1.02] z-10'
              : 'border-surface-100 hover:border-surface-300 hover:shadow-md'
          ]"
          @click="selectEmail(email.id)"
        >
          <div v-if="selectedEmailId === email.id" class="absolute left-0 top-0 bottom-0 w-[3px] bg-blue-500 rounded-r"></div>

          <!-- Content -->
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-1.5">
              <span
                class="text-[12px] truncate flex-1 text-surface-700"
              >
                {{ email.subject || '(无主题)' }}
              </span>
            </div>
            <div class="flex items-center gap-1.5 mt-0.5">
              <span class="text-[11px] text-surface-500 truncate">
                {{ email.from_name || email.from_addr }}
              </span>
              <span v-if="email.has_attachments" class="text-surface-300 text-[10px]">📎</span>
            </div>
            <p class="text-[9px] text-surface-400 mt-0.5">
              {{ formatDate(email.received_at || email.created_at) }}
            </p>
          </div>

        </div>
        <!-- 加载更多 -->
        <div v-if="loadingMore" class="flex items-center justify-center py-3">
          <span class="text-[11px] text-surface-400">加载更多...</span>
        </div>
        <div v-else-if="hasMore && filteredEmails.length > 0" class="flex items-center justify-center py-2">
          <button
            class="text-[11px] text-brand-600 hover:text-brand-700 hover:underline"
            @click="loadMore"
          >加载更多</button>
        </div>
        <div v-else-if="!hasMore && filteredEmails.length > 0" class="flex items-center justify-center py-2">
          <span class="text-[10px] text-surface-300">已加载全部 {{ total }} 封</span>
        </div>
      </div>
    </div>

    <!-- Email Detail -->
    <div class="flex-1 flex flex-col overflow-hidden bg-surface-50/30">
      <div v-if="!selectedEmailId" class="flex-1 flex items-center justify-center">
        <div class="text-center">
          <AppIcon name="mail" size="48" class="text-surface-200 mx-auto mb-3"/>
          <p class="text-sm text-surface-400">选择一封邮件查看</p>
        </div>
      </div>

      <template v-else>
        <!-- Detail header -->
        <div class="flex-shrink-0 px-6 py-3 bg-white border-b border-surface-100">
          <div v-if="detailLoading" class="text-xs text-surface-400">加载中...</div>
          <template v-else-if="selectedEmail">
            <div class="flex items-start justify-between gap-4">
              <div class="flex-1 min-w-0">
                <h2 class="text-base font-semibold text-surface-800 break-words">
                  {{ selectedEmail.subject || '(无主题)' }}
                </h2>
                <div class="flex items-center gap-3 mt-1.5">
                  <div class="flex items-center gap-1.5">
                    <div class="w-6 h-6 rounded-full bg-brand-100 flex items-center justify-center text-brand-700 text-[10px] font-medium">
                      {{ (selectedEmail.from_name || selectedEmail.from_addr || '?')[0].toUpperCase() }}
                    </div>
                    <div>
                      <span class="text-[12px] font-medium text-surface-700">
                        {{ selectedEmail.from_name || selectedEmail.from_addr }}
                      </span>
                      <span v-if="selectedEmail.from_name" class="text-[11px] text-surface-400 ml-1">
                        &lt;{{ selectedEmail.from_addr }}&gt;
                      </span>
                    </div>
                  </div>
                </div>
                <div class="mt-1 text-[11px] text-surface-400">
                  <span>收件人: {{ selectedEmail.to_addr }}</span>
                </div>
                <div v-if="selectedEmail.cc_addr" class="mt-0.5 text-[11px] text-surface-400">
                  <span>抄送: {{ selectedEmail.cc_addr }}</span>
                </div>
                <div class="mt-0.5 text-[11px] text-surface-400">
                  <span>{{ formatFullDate(selectedEmail.received_at) }}</span>
                </div>
              </div>
            </div>
          </template>
        </div>

        <!-- Detail body -->
        <div class="flex-1 overflow-y-auto">
          <div v-if="detailLoading" class="flex items-center justify-center py-20">
            <span class="text-xs text-surface-400">加载中...</span>
          </div>
          <template v-else-if="selectedEmail">
            <!-- Attachments -->
            <div
              v-if="attachments.length > 0"
              class="mx-6 mt-4 px-4 py-3 bg-white border border-surface-200 rounded-lg"
            >
              <div class="text-xs font-medium text-surface-600 mb-2">
                附件 ({{ attachments.length }})
              </div>
              <div class="space-y-1.5">
                <div
                  v-for="att in attachments"
                  :key="att.id"
                  class="flex items-center gap-2 text-xs rounded-md px-2 py-1.5 transition-colors group"
                  :class="att.file_asset_id ? 'cursor-pointer hover:bg-surface-50' : 'opacity-60'"
                  @click="openAttachment(att)"
                >
                  <AppIcon name="paperclip" size="14" class="text-surface-400 flex-shrink-0"/>
                  <span class="text-surface-700 truncate">{{ att.filename }}</span>
                  <span class="text-surface-400 flex-shrink-0">{{ formatSize(att.size) }}</span>
                  <span class="flex-1"></span>
                  <div v-if="att.file_asset_id" class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                    <button @click.stop="downloadAttachment(att)" class="w-5 h-5 flex items-center justify-center rounded hover:bg-surface-200 text-surface-400 hover:text-surface-600" title="下载">
                      <AppIcon name="download" size="12" />
                    </button>
                    <button @click.stop="copyAttachment(att)" class="w-5 h-5 flex items-center justify-center rounded hover:bg-surface-200 text-surface-400 hover:text-surface-600" title="复制文件到剪贴板">
                      <AppIcon name="copy" size="12" />
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <!-- Email body -->
            <div class="px-6 py-4">
              <div v-if="renderedHtml" ref="emailBodyEl" class="email-body text-sm text-surface-700 leading-relaxed" v-html="renderedHtml"></div>
              <pre v-else class="text-sm text-surface-700 whitespace-pre-wrap font-sans leading-relaxed">{{ selectedEmail.body_text || '(无正文)' }}</pre>
            </div>
          </template>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.email-body :deep(table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.5rem 0;
}
.email-body :deep(th),
.email-body :deep(td) {
  border: 1px solid #e2e8f0;
  padding: 0.5rem;
  text-align: left;
}
/* 图片拼接用的表格单元格：去掉我们加的 padding/border，避免图片之间出现缝隙 */
.email-body :deep(td):has(> img, > a > img, > .emimg-box),
.email-body :deep(th):has(> img, > a > img, > .emimg-box) {
  padding: 0 !important;
  border: none !important;
}
.email-body :deep(table):has(td:has(img)) {
  margin: 0;
}
.email-body :deep(th) {
  background-color: #f7fafc;
  font-weight: 600;
}
.email-body :deep(img) {
  max-width: 100% !important;
  height: auto !important;
}
.email-body :deep(img[data-emimg]) {
  background: transparent;
}
.email-body :deep(.emimg-box) {
  position: relative;
  display: block;
  width: 100%;
  border-radius: 0;
  background: #f1f5f9;
  overflow: hidden;
  margin: 0;
}
.email-body :deep(.emimg-box.emimg-loaded) {
  background: transparent;
}
.email-body :deep(.emimg-box > img) {
  display: block;
  height: auto !important;
  opacity: 0;
  transition: opacity 0.3s ease;
}
.email-body :deep(.emimg-box.emimg-loaded > img) {
  opacity: 1;
}
.email-body :deep(.emimg-spinner) {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 20px;
  height: 20px;
  margin: -10px 0 0 -10px;
  border-radius: 50%;
  border: 2px solid #e2e8f0;
  border-top-color: #3b82f6;
  animation: emimg-spin 0.8s linear infinite;
  opacity: 1;
  transition: opacity 0.25s;
  pointer-events: none;
}
.email-body :deep(.emimg-box.emimg-loaded .emimg-spinner) {
  opacity: 0;
  visibility: hidden;
}
.email-body :deep(.emimg-box.emimg-error .emimg-spinner) {
  opacity: 0.35;
  animation: none;
}
@keyframes emimg-spin {
  to {
    transform: rotate(360deg);
  }
}
.email-body :deep(a) {
  color: #3b82f6;
  text-decoration: underline;
}
.email-body :deep(ul),
.email-body :deep(ol) {
  padding-left: 1.5rem;
  margin: 0.5rem 0;
}
.email-body :deep(li) {
  margin: 0.25rem 0;
}
.email-body :deep(blockquote) {
  border-left: 3px solid #e2e8f0;
  padding-left: 1rem;
  margin: 0.5rem 0;
  color: #64748b;
}
.email-body :deep(pre) {
  background-color: #f1f5f9;
  padding: 0.75rem;
  border-radius: 0.375rem;
  overflow-x: auto;
}
.email-body :deep(code) {
  background-color: #f1f5f9;
  padding: 0.125rem 0.25rem;
  border-radius: 0.25rem;
  font-size: 0.875rem;
}
</style>
