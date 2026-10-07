<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch, computed, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { emailGetDetail, emailRenderHtml, emailDelete, type EmailDetail, type EmailAttachment } from '@/api/email-client'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { lazyLoadEmailImages } from '@/utils/emailLazyImages'
import { renderedCacheGet, renderedCacheSet } from '@/utils/emailImageCache'
import EmailAttachmentCards from '@/components/email/EmailAttachmentCards.vue'
import DOMPurify from 'dompurify'

const route = useRoute()
const router = useRouter()
const email = ref<EmailDetail | null>(null)
const attachments = ref<EmailAttachment[]>([])
const loading = ref(true)

const emailId = computed(() => String(route.params.id || ''))

const renderedHtml = ref('')
const emailBodyEl = ref<HTMLElement | null>(null)
let cancelLazy: (() => void) | null = null
let lazyEmailId: string | null = null

// 正文 DOM 真正挂载后再启动懒加载（避免 loading 期间容器不存在）
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
    renderedHtml.value = html
      ? DOMPurify.sanitize(html, {
          ADD_TAGS: ['img', 'a'],
          ADD_ATTR: ['src', 'alt', 'width', 'height', 'href', 'target', 'id', 'style', 'border'],
          ADD_DATA_URI_TAGS: ['img'],
        })
      : ''
    if (lazy) lazyEmailId = id
  } catch (e) {
    console.error('渲染邮件正文失败:', e)
    renderedHtml.value = ''
  }
}

async function loadDetail() {
  loading.value = true
  try {
    const res = await emailGetDetail(emailId.value)
    if (res.success) {
      email.value = res.email
      attachments.value = res.attachments || []
      await loadRenderedHtml(emailId.value)
    }
  } catch (e) {
    console.error('加载邮件详情失败:', e)
  } finally {
    loading.value = false
    await nextTick()
    startLazyIfPending()
  }
}

async function handleDelete() {
  const ok = await dangerConfirm({ title: '删除邮件', message: '确定删除此邮件？删除后不再显示在列表中。', confirmText: '删除' })
  if (!ok) return
  try {
    await emailDelete(emailId.value)
    router.push('/email')
  } catch (e: any) {
    toast({ message: `删除失败: ${e?.message || e}`, type: 'error' })
  }
}

function formatDate(dateStr: string): string {
  if (!dateStr) return ''
  try {
    return new Date(dateStr).toLocaleString('zh-CN')
  } catch {
    return dateStr
  }
}

function getCategoryLabel(cat: string): string {
  const map: Record<string, string> = {
    knowledge: '已入库',
    notification: '已通知',
    ignore: '已忽略',
    pending: '待处理',
  }
  return map[cat] || cat
}

function getCategoryColor(cat: string): string {
  const map: Record<string, string> = {
    knowledge: 'text-emerald-700 bg-emerald-50 border-emerald-200',
    notification: 'text-amber-700 bg-amber-50 border-amber-200',
    ignore: 'text-surface-500 bg-surface-50 border-surface-200',
    pending: 'text-blue-700 bg-blue-50 border-blue-200',
  }
  return map[cat] || 'text-surface-500 bg-surface-50 border-surface-200'
}

onMounted(loadDetail)
watch(emailId, (newId, oldId) => {
  if (newId && newId !== oldId) loadDetail()
})
onBeforeUnmount(() => {
  cancelLazy?.()
  cancelLazy = null
})
</script>

<template>
  <div class="h-full flex flex-col bg-surface-50">
    <!-- Header -->
    <div class="flex-shrink-0 px-6 py-3 bg-white border-b border-surface-200 flex items-center gap-3">
      <button
        class="text-surface-400 hover:text-surface-600 transition-colors"
        @click="router.push('/email')"
      >
        <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path d="M15 19l-7-7 7-7" />
        </svg>
      </button>
      <h1 class="text-sm font-medium text-surface-800 truncate flex-1">
        {{ email?.subject || '邮件详情' }}
      </h1>
      <button
        class="text-surface-400 hover:text-red-500 transition-colors"
        @click="handleDelete"
        v-tooltip="'删除'"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
        </svg>
      </button>
    </div>

    <div v-if="loading" class="flex-1 flex items-center justify-center">
      <span class="text-xs text-surface-400">加载中...</span>
    </div>

    <div v-else-if="email" class="flex-1 overflow-y-auto min-h-0">
      <!-- Email header info -->
      <div class="px-6 py-4 bg-white border-b border-surface-100">
        <h2 class="text-base font-semibold text-surface-800">
          {{ email.subject || '(无主题)' }}
        </h2>
        <div class="flex items-center gap-2 mt-2">
          <span
            v-if="email.ai_category"
            class="px-2 py-0.5 text-[11px] rounded border"
            :class="getCategoryColor(email.ai_category)"
          >
            {{ getCategoryLabel(email.ai_category) }}
          </span>
        </div>
        <div class="mt-3 space-y-1.5 text-xs text-surface-500">
          <div class="flex items-start gap-2">
            <span class="flex-shrink-0 w-12 text-surface-400">发件人</span>
            <span class="text-surface-700">
              {{ email.from_name }} &lt;{{ email.from_addr }}&gt;
            </span>
          </div>
          <div class="flex items-start gap-2">
            <span class="flex-shrink-0 w-12 text-surface-400">收件人</span>
            <span class="text-surface-700">{{ email.to_addr }}</span>
          </div>
          <div class="flex items-start gap-2">
            <span class="flex-shrink-0 w-12 text-surface-400">时间</span>
            <span class="text-surface-700">{{ formatDate(email.received_at) }}</span>
          </div>
        </div>
      </div>

      <!-- Email body -->
      <div class="mx-6 my-4 px-4 py-4 bg-white border border-surface-200 rounded-lg">
        <div v-if="renderedHtml" ref="emailBodyEl" class="email-body text-sm text-surface-700 leading-relaxed" v-html="renderedHtml"></div>
        <pre v-else class="text-sm text-surface-700 whitespace-pre-wrap font-sans leading-relaxed">{{ email.body_text || '(无正文)' }}</pre>
        <!-- 附件：图片按正文同款渲染，文件用卡片（无独立附件区） -->
        <EmailAttachmentCards :email-id="emailId" :attachments="attachments" />
      </div>
    </div>
  </div>
</template>

<script lang="ts">
import { computed } from 'vue'
export default { name: 'EmailDetail' }
</script>

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
