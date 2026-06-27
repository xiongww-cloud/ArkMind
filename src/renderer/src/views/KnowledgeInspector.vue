<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { pipelineDebugStatus, pipelineClearKnowledge, pipelineClearVectors, pipelineClearAll, aiSyncReset, aiSyncRetryFailed, aiSyncTrigger, aiSyncDiagnose, kbDiagnose, kbDataBrowse, pipelineBrowseSource, pipelineGetSourceContent, pipelineRetagAll, kbBrowseParents, kbBrowseVectors } from '@/api/db-client'
import { dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { sanitizeHtml } from '@/utils/sanitize'

const loading = ref(true)
const status = ref<any>({})
const diagnose = ref<any>({})
const syncDiagnose = ref<any[]>([])
const kbBrowse = ref<any[]>([])
const sourceRows = ref<any[]>([])
const sourceTypeFilter = ref('')
const sourceDetail = ref<any>(null)
const detailHtml = ref('')
const tab = ref<'overview'|'source'|'chunks'|'sourcedata'>('overview')
const chunkTab = ref<'children'|'parents'|'vectors'>('children')
const parentBrowse = ref<any[]>([])
const parentPage = ref(1)
const parentTotal = ref(0)
const childPage = ref(1)
const childTotal = ref(0)
const vectorBrowse = ref<any[]>([])
const chunkDetail = ref<any>(null)
const chunkDetailHtml = ref('')

onMounted(async () => { await refresh() })

async function refresh() {
  loading.value = true
  await Promise.all([
    loadStatus(), loadDiagnose(), loadSyncDiagnose(), loadKbBrowse(),
  ].map(p => p.catch(() => {})))
  loading.value = false
}

async function loadStatus() { status.value = await pipelineDebugStatus() || {} }
async function loadDiagnose() { diagnose.value = await kbDiagnose() || {} }
async function loadSyncDiagnose() { syncDiagnose.value = (await aiSyncDiagnose())?.tables || [] }
async function loadKbBrowse(pg?: number) { const p = pg ?? childPage.value; childPage.value = p; const r = await kbDataBrowse(undefined, p); kbBrowse.value = r?.rows || []; childTotal.value = r?.total || 0 }
async function loadParentBrowse(pg?: number) { const p = pg ?? parentPage.value; parentPage.value = p; const r = await kbBrowseParents(undefined, p); parentBrowse.value = r?.rows || []; parentTotal.value = r?.total || 0 }
async function loadVectorBrowse() { vectorBrowse.value = (await kbBrowseVectors(100))?.rows || [] }

function showChunkDetail(row: any) {
  chunkDetail.value = row
  chunkDetailHtml.value = ''
  setTimeout(async () => {
    try {
      const markedModule = await import('marked')
      const Marked = markedModule.Marked || markedModule.default?.Marked || markedModule.default
      if (!Marked) {
        console.warn('[marked] Marked 构造函数未找到, 使用全局 marked')
        const { marked } = markedModule
        chunkDetailHtml.value = await sanitizeHtml((marked?.parse || marked)(row.content || ''))
        return
      }
      const marked = new Marked({ gfm: true, breaks: true })
      chunkDetailHtml.value = await sanitizeHtml(marked.parse(row.content || ''))
    } catch (e) {
      console.error('[marked] 渲染失败:', e, '内容前50字:', (row.content || '').slice(0, 50))
      chunkDetailHtml.value = row.content || ''
    }
  }, 50)
}
function closeChunkDetail() { chunkDetail.value = null; chunkDetailHtml.value = '' }

async function loadSource() {
  loading.value = true
  try {
    const r = await pipelineBrowseSource(sourceTypeFilter.value || undefined)
    sourceRows.value = r?.rows || []
  } catch { sourceRows.value = [] }
  loading.value = false
}

async function showSourceDetail(row: any) {
  sourceDetail.value = row
  detailHtml.value = ''
  try {
    const r = await pipelineGetSourceContent(row.id)
    if (r?.row) {
      try {
        const markedModule = await import('marked')
        const Marked = markedModule.Marked || markedModule.default?.Marked || markedModule.default
        if (Marked) {
          const marked = new Marked({ gfm: true, breaks: true })
          detailHtml.value = await sanitizeHtml(marked.parse(r.row.content || ''))
        } else {
          const { marked } = markedModule
          detailHtml.value = await sanitizeHtml((marked?.parse || marked)(r.row.content || ''))
        }
      } catch (e) {
        console.error('[marked] 源数据渲染失败:', e)
        detailHtml.value = r.row.content || ''
      }
      sourceDetail.value = r.row
    }
  } catch (e) { console.error('[Inspector] loadSourceDetail failed:', e) }
}

function closeDetail() { sourceDetail.value = null; detailHtml.value = '' }

async function handleClearVectors() {
  const ok = await dangerConfirm({ title: '清除向量数据', message: '确定清除所有向量缓存标记？', confirmText: '清除' })
  if (!ok) return; await pipelineClearVectors(); toast({ message: '向量缓存已清除', type: 'success' }); await loadStatus()
}
async function handleClearKnowledge() {
  const ok = await dangerConfirm({ title: '清除物化数据', message: '确定清除所有知识块？', confirmText: '清除' })
  if (!ok) return; await pipelineClearKnowledge(); toast({ message: '物化数据已清除', type: 'success' }); await refresh()
}
async function handleClearAll() {
  const ok = await dangerConfirm({ title: '清空全部管线', message: '确定清空所有管线数据？此操作不可逆。', confirmText: '清空全部' })
  if (!ok) return; await pipelineClearAll(); toast({ message: '所有管线数据已清空', type: 'success' }); await refresh()
}
async function handleResetQueue() { await aiSyncReset(); toast({ message: '同步队列已重置', type: 'success' }) }
async function handleRetryFailed() { await aiSyncRetryFailed(); toast({ message: '失败任务已重置', type: 'success' }) }
async function handleTriggerSync() { await aiSyncTrigger(); toast({ message: '管线已触发', type: 'success' }) }
async function handleRetagAll() {
  const r = await pipelineRetagAll()
  toast({ message: r?.message || '已重置打标队列', type: 'success' })
  await loadStatus()
}
</script>

<template>
  <div class="h-full flex flex-col bg-white rounded-xl border border-surface-200 shadow-sm">
    <header class="flex items-center justify-between px-6 py-4 border-b border-surface-100">
      <div>
        <h2 class="text-lg font-bold text-surface-800">管线调试器</h2>
        <p class="text-xs text-surface-400 mt-0.5">
          数据摄入中台 · 知识管线 · 标签管线 · 向量化
          <button @click="refresh" class="ml-2 text-blue-600 hover:text-blue-700 font-medium">刷新</button>
        </p>
      </div>
      <div class="flex gap-1 bg-surface-100 rounded-lg p-0.5">
        <button v-for="t in ['overview','source','chunks','sourcedata']" :key="t" @click="tab = t as any; if(t==='sourcedata')loadSource()" class="px-3 py-1 text-xs rounded-md transition" :class="tab===t?'bg-white shadow text-surface-800':'text-surface-500'">{{ {overview:'总览',source:'队列',chunks:'知识块',sourcedata:'源数据'}[t] }}</button>
      </div>
    </header>

    <div v-if="loading && tab !== 'sourcedata'" class="flex-1 flex items-center justify-center text-surface-400 text-sm">加载中...</div>

    <div v-else class="flex-1 overflow-y-auto p-5 space-y-5">

      <!-- ═══ Overview ═══ -->
      <template v-if="tab==='overview'">
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">数据摄入中台</h3>
          <div class="grid grid-cols-4 gap-2">
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">文件已解析</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.fileParse?.done || 0 }}</p></div>
            <div class="bg-amber-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">待解析</p><p class="text-lg font-bold text-amber-600 mt-0.5">{{ status?.fileParse?.pending || 0 }}</p></div>
            <div class="bg-red-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">解析失败</p><p class="text-lg font-bold text-red-600 mt-0.5">{{ status?.fileParse?.failed || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">总文件</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ (status?.fileParse?.done||0)+(status?.fileParse?.pending||0)+(status?.fileParse?.failed||0) }}</p></div>
          </div>
        </div>
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">知识管线</h3>
          <div class="grid grid-cols-4 gap-2">
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">同步待处理</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.sync?.pending || 0 }}</p></div>
            <div class="bg-emerald-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">同步已完成</p><p class="text-lg font-bold text-emerald-600 mt-0.5">{{ status?.sync?.done || 0 }}</p></div>
            <div class="bg-red-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">同步失败</p><p class="text-lg font-bold text-red-600 mt-0.5">{{ status?.sync?.failed || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">同步总数</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.sync?.total || 0 }}</p></div>
          </div>
          <div class="grid grid-cols-4 gap-2 mt-2">
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">知识块</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.knowledge?.chunks || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">父块</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.knowledge?.parents || 0 }}</p></div>
            <div class="bg-blue-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">已向量化</p><p class="text-lg font-bold text-blue-600 mt-0.5">{{ status?.knowledge?.vectorized || 0 }}</p></div>
            <div class="bg-purple-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">实体</p><p class="text-lg font-bold text-purple-600 mt-0.5">{{ status?.entity?.entities||0 }} / {{ status?.entity?.mentions||0 }}</p></div>
          </div>
        </div>
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">标签管线</h3>
          <div class="grid grid-cols-3 gap-2">
            <div class="bg-amber-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">待打标</p><p class="text-lg font-bold text-amber-600 mt-0.5">{{ status?.tag?.pending || 0 }}</p></div>
            <div class="bg-emerald-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">已完成</p><p class="text-lg font-bold text-emerald-600 mt-0.5">{{ status?.tag?.done || 0 }}</p></div>
            <div class="bg-red-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">失败</p><p class="text-lg font-bold text-red-600 mt-0.5">{{ status?.tag?.failed || 0 }}</p></div>
          </div>
        </div>
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">操作</h3>
          <div class="flex flex-wrap gap-2">
            <button @click="handleTriggerSync" class="px-3 py-1.5 text-xs rounded-lg bg-brand-600 text-white hover:bg-brand-700 transition">触发同步</button>
            <button @click="handleResetQueue" class="px-3 py-1.5 text-xs rounded-lg bg-amber-500 text-white hover:bg-amber-600 transition">重置同步队列</button>
            <button @click="handleRetryFailed" class="px-3 py-1.5 text-xs rounded-lg bg-orange-500 text-white hover:bg-orange-600 transition">重试失败任务</button>
            <button @click="handleRetagAll" class="px-3 py-1.5 text-xs rounded-lg bg-violet-500 text-white hover:bg-violet-600 transition">重新打标</button>
            <button @click="handleClearVectors" class="px-3 py-1.5 text-xs rounded-lg border border-blue-300 text-blue-600 hover:bg-blue-50 transition">清除向量缓存</button>
            <button @click="handleClearKnowledge" class="px-3 py-1.5 text-xs rounded-lg border border-amber-300 text-amber-600 hover:bg-amber-50 transition">清除物化数据</button>
            <button @click="handleClearAll" class="px-3 py-1.5 text-xs rounded-lg bg-red-500 text-white hover:bg-red-600 transition">清空全部管线</button>
          </div>
        </div>
      </template>

      <!-- ═══ Queue ═══ -->
      <template v-if="tab==='source'">
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">source_data 各类型数量</h3>
          <div class="grid grid-cols-4 gap-2">
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">文件</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.source?.files || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">笔记</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.source?.notes || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">产品</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.source?.products || 0 }}</p></div>
            <div class="bg-surface-50 rounded-lg p-3"><p class="text-[10px] text-surface-400">项目</p><p class="text-lg font-bold text-surface-800 mt-0.5">{{ status?.source?.projects || 0 }}</p></div>
          </div>
        </div>
        <div>
          <h3 class="text-xs font-semibold text-surface-500 uppercase mb-2">同步队列按表分布</h3>
          <div v-if="syncDiagnose.length" class="space-y-1">
            <div v-for="t in syncDiagnose" :key="t.table" class="flex items-center justify-between px-3 py-2 bg-surface-50 rounded-lg text-sm">
              <span class="font-medium text-surface-700">{{ t.table }}</span>
              <div class="flex gap-3 text-xs">
                <span>总数 <strong>{{ t.total }}</strong></span>
                <span class="text-amber-600">待处理 <strong>{{ t.pending }}</strong></span>
                <span class="text-emerald-600">完成 <strong>{{ t.done }}</strong></span>
                <span class="text-red-600">失败 <strong>{{ t.failed }}</strong></span>
              </div>
            </div>
          </div>
          <p v-else class="text-xs text-surface-400">无队列数据</p>
        </div>
      </template>

      <!-- ═══ Chunks ═══ -->
      <template v-if="tab==='chunks'">
        <div class="flex flex-col h-full">
          <!-- Sub-tabs -->
          <div class="flex items-center gap-1 mb-3 shrink-0 bg-surface-100 rounded-lg p-0.5 self-start">
            <button v-for="ct in ['children','parents','vectors']" :key="ct" @click="chunkTab=ct as any; childPage=1; parentPage=1; if(ct==='children')loadKbBrowse(1); else if(ct==='parents')loadParentBrowse(1); else loadVectorBrowse()" class="px-3 py-1 text-xs rounded-md transition" :class="chunkTab===ct?'bg-white shadow text-surface-800':'text-surface-500'">{{ {children:'子块',parents:'父块',vectors:'向量'}[ct] }}</button>
          </div>
          <!-- Content: 3-column layout for children/parents, list+detail for vectors -->
          <div class="flex gap-3 flex-1 min-h-0">
            <!-- List -->
            <div class="w-[320px] shrink-0 overflow-y-auto border border-surface-200 rounded-lg">
              <div v-if="chunkTab==='children' && kbBrowse.length" class="divide-y divide-surface-100">
                <div v-for="r in kbBrowse" :key="r.rowid" @click="showChunkDetail(r)" class="px-3 py-2 text-xs cursor-pointer transition" :class="chunkDetail?.rowid === r.rowid ? 'bg-blue-50' : 'hover:bg-surface-50'">
                  <div class="flex items-center gap-1.5">
                    <span class="text-[9px] font-medium px-1 py-0.5 rounded shrink-0 bg-sky-100 text-sky-700">子块</span>
                    <span class="text-[9px] text-surface-400 font-mono">#{{ r.rowid }}</span>
                  </div>
                  <p class="truncate mt-0.5 text-surface-700">{{ (r.content||'').slice(0,100) }}</p>
                  <p class="text-[10px] text-surface-400 mt-0.5">{{ (r.created_at||'').slice(0,10) }}</p>
                </div>
              </div>
              <div class="flex items-center justify-between px-1 py-2 text-xs text-surface-500 border-t border-surface-100">
                <span>共 {{ childTotal }} 条</span>
                <div class="flex gap-1">
                  <button @click="childPage>1&&loadKbBrowse(childPage-1)" class="px-2 py-0.5 rounded hover:bg-surface-100" :class="childPage<=1?'opacity-30':'cursor-pointer'">上一页</button>
                  <span class="px-2 py-0.5">第 {{ childPage }} / {{ Math.ceil(childTotal/50)||1 }} 页</span>
                  <button @click="childPage*50<childTotal&&loadKbBrowse(childPage+1)" class="px-2 py-0.5 rounded hover:bg-surface-100" :class="childPage*50>=childTotal?'opacity-30':'cursor-pointer'">下一页</button>
                </div>
              </div>
              <div v-if="chunkTab==='parents' && parentBrowse.length" class="divide-y divide-surface-100">
                <div v-for="r in parentBrowse" :key="r.rowid" @click="showChunkDetail(r)" class="px-3 py-2 text-xs cursor-pointer transition" :class="chunkDetail?.rowid === r.rowid ? 'bg-blue-50' : 'hover:bg-surface-50'">
                  <div class="flex items-center gap-1.5">
                    <span class="text-[9px] font-medium px-1 py-0.5 rounded shrink-0 bg-amber-100 text-amber-700">父块</span>
                    <span class="text-[9px] text-surface-400 font-mono">#{{ r.rowid }}</span>
                  </div>
                  <p class="truncate mt-0.5 text-surface-700">{{ (r.content||'').slice(0,100) }}</p>
                  <p class="text-[10px] text-surface-400 mt-0.5">{{ (r.created_at||'').slice(0,10) }} · 索引 {{ r.chunk_index||0 }}</p>
                </div>
              </div>
              <div v-if="chunkTab==='parents'" class="flex items-center justify-between px-1 py-2 text-xs text-surface-500 border-t border-surface-100">
                <span>共 {{ parentTotal }} 条</span>
                <div class="flex gap-1">
                  <button @click="parentPage>1&&loadParentBrowse(parentPage-1)" class="px-2 py-0.5 rounded hover:bg-surface-100" :class="parentPage<=1?'opacity-30':'cursor-pointer'">上一页</button>
                  <span class="px-2 py-0.5">第 {{ parentPage }} / {{ Math.ceil(parentTotal/50)||1 }} 页</span>
                  <button @click="parentPage*50<parentTotal&&loadParentBrowse(parentPage+1)" class="px-2 py-0.5 rounded hover:bg-surface-100" :class="parentPage*50>=parentTotal?'opacity-30':'cursor-pointer'">下一页</button>
                </div>
              </div>
              <div v-if="chunkTab==='vectors' && vectorBrowse.length" class="divide-y divide-surface-100">
                <div v-for="r in vectorBrowse" :key="r.rowid" @click="showChunkDetail(r)" class="px-3 py-2 text-xs cursor-pointer transition" :class="chunkDetail?.rowid === r.rowid ? 'bg-blue-50' : 'hover:bg-surface-50'">
                  <div class="flex items-center gap-1.5">
                    <span class="text-[9px] font-medium px-1 py-0.5 rounded shrink-0 bg-purple-100 text-purple-700">向量</span>
                    <span class="text-[9px] text-surface-400 font-mono">#{{ r.rowid }}</span>
                  </div>
                  <p class="truncate mt-0.5 text-surface-700">{{ (r.content||'').slice(0,100) }}</p>
                  <p class="text-[10px] text-surface-400 mt-0.5">doc_id: {{ (r.doc_id||'').slice(0,16) }}</p>
                </div>
              </div>
              <p v-if="(chunkTab==='children' && !kbBrowse.length) || (chunkTab==='parents' && !parentBrowse.length) || (chunkTab==='vectors' && !vectorBrowse.length)" class="text-xs text-surface-400 text-center py-8">{{ chunkTab==='vectors' ? '向量为空 — 请检查 LanceDB 数据目录或重新同步' : '暂无数据' }}</p>
            </div>
            <!-- Detail -->
            <div class="flex-1 border border-surface-200 rounded-lg overflow-hidden flex flex-col">
              <div v-if="chunkDetail" class="flex flex-col h-full">
                <div class="flex items-center justify-between px-4 py-2 bg-surface-50 border-b border-surface-200 shrink-0">
                  <div class="text-xs text-surface-600">
                    <span class="font-medium">#{{ chunkDetail.rowid }}</span>
                    <span class="ml-2 text-surface-400">({{ (chunkDetail.content||'').length }} 字符)</span>
                    <span v-if="chunkDetail.doc_title" class="ml-2 text-surface-400">标题: {{ (chunkDetail.doc_title||'').slice(0,20) }}</span>
                    <span v-if="chunkDetail.source_data_id" class="ml-2 text-surface-400">源数据ID: {{ chunkDetail.source_data_id }}</span>
                  </div>
                  <button @click="closeChunkDetail" class="text-xs text-surface-400 hover:text-surface-600">关闭</button>
                </div>
                <div class="flex-1 overflow-y-auto p-4">
                  <div v-if="chunkDetailHtml" v-html="chunkDetailHtml" class="prose prose-sm max-w-none text-surface-700 overflow-x-auto"></div>
                  <div v-else class="text-xs text-surface-400">加载中...</div>
                </div>
              </div>
              <div v-else class="flex-1 flex items-center justify-center text-xs text-surface-400">选择一条记录查看详情</div>
            </div>
          </div>
        </div>
      </template>

      <!-- ═══ source_data ═══ -->
      <template v-if="tab==='sourcedata'">
        <div class="flex flex-col h-full">
          <div class="flex items-center gap-2 mb-3 shrink-0">
            <select v-model="sourceTypeFilter" @change="loadSource" class="border border-surface-200 rounded-lg px-3 py-1.5 text-xs outline-none">
              <option value="">全部类型</option>
              <option value="FILE">文件</option>
              <option value="NOTE">笔记</option>
              <option value="PRODUCT">产品</option>
              <option value="PROJECT">项目</option>
            </select>
            <span class="text-xs text-surface-400">{{ sourceRows.length }} 条</span>
            <button @click="loadSource" class="text-xs text-blue-600 hover:text-blue-700">刷新</button>
          </div>
          <div class="flex gap-3 flex-1 min-h-0">
            <!-- Left: list -->
            <div class="w-[320px] shrink-0 overflow-y-auto border border-surface-200 rounded-lg">
              <div v-if="sourceRows.length" class="divide-y divide-surface-100">
                <div v-for="r in sourceRows" :key="r.id" @click="showSourceDetail(r)" class="px-3 py-2 text-xs cursor-pointer transition" :class="sourceDetail?.id === r.id ? 'bg-blue-50' : 'hover:bg-surface-50'">
                  <div class="flex items-center gap-1.5">
                    <span class="text-[9px] font-medium px-1 py-0.5 rounded shrink-0" :class="{'bg-blue-100 text-blue-700':r.source_type==='FILE','bg-green-100 text-green-700':r.source_type==='NOTE','bg-amber-100 text-amber-700':r.source_type==='PRODUCT','bg-purple-100 text-purple-700':r.source_type==='PROJECT'}">{{ r.source_type }}</span>
                    <span class="text-[9px] text-surface-400 font-mono">#{{ r.id }}</span>
                  </div>
                  <p class="truncate mt-0.5 text-surface-700">{{ r.title || '(无标题)' }}</p>
                  <p class="text-[10px] text-surface-400 mt-0.5">{{ r.content_len || 0 }}字 · {{ r.updated_at?.slice(0,10) }}</p>
                </div>
              </div>
              <p v-if="!sourceRows.length" class="text-xs text-surface-400 text-center py-8">暂无数据</p>
            </div>
            <!-- Right: preview -->
            <div class="flex-1 border border-surface-200 rounded-lg overflow-hidden flex flex-col">
              <div v-if="sourceDetail" class="flex flex-col h-full">
                <div class="flex items-center justify-between px-4 py-2 bg-surface-50 border-b border-surface-200 shrink-0">
                  <div class="text-xs text-surface-600">
                    <span class="font-medium">{{ sourceDetail.source_type }}</span>
                    <span v-if="sourceDetail.title" class="ml-2 text-surface-500">· {{ sourceDetail.title }}</span>
                    <span class="ml-2 text-surface-400">({{ sourceDetail.content?.length || 0 }} 字符)</span>
                  </div>
                </div>
                <div class="flex-1 overflow-y-auto p-4">
                  <div v-if="detailHtml" v-html="detailHtml" class="prose prose-sm max-w-none text-surface-700 overflow-x-auto"></div>
                  <div v-else class="text-xs text-surface-400">加载中...</div>
                </div>
              </div>
              <div v-else class="flex-1 flex items-center justify-center text-xs text-surface-400">选择一条记录查看详情</div>
            </div>
          </div>
        </div>
      </template>

    </div>
  </div>
</template>

<style scoped>
.prose :deep(h1) { font-size: 1.25rem; font-weight: 700; margin: 0.5em 0 0.25em; }
.prose :deep(h2) { font-size: 1.1rem; font-weight: 600; margin: 0.5em 0 0.25em; }
.prose :deep(h3) { font-size: 1rem; font-weight: 600; margin: 0.4em 0 0.2em; }
.prose :deep(p) { margin: 0.3em 0; }
.prose :deep(ul), .prose :deep(ol) { padding-left: 1.5em; margin: 0.3em 0; }
.prose :deep(li) { margin: 0.15em 0; }
.prose :deep(code) { background: #f1f5f9; border-radius: 3px; padding: 1px 4px; font-size: 0.9em; }
.prose :deep(pre) { background: #f1f5f9; border-radius: 6px; padding: 12px; overflow-x: auto; font-size: 0.85em; }
.prose :deep(table) { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
.prose :deep(th), .prose :deep(td) { border: 1px solid #e2e8f0; padding: 4px 8px; font-size: 0.9em; }
.prose :deep(th) { background: #f8fafc; font-weight: 600; }
.prose :deep(blockquote) { border-left: 3px solid #e2e8f0; padding-left: 1em; margin: 0.5em 0; color: #64748b; }
</style>