<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onDeactivated, onBeforeUnmount, watch, nextTick } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import GuideModal from '@/components/common/GuideModal.vue'
import { alert, dangerConfirm } from '@/composables/useConfirm'
import { useTooltip } from '@/composables/useTooltip'
import { toast } from '@/composables/useToast'
import { useUploadProgress } from '@/composables/useUploadProgress'
import {
  fetchSettings, updateSettings, browseFolder,
  updateAiConfig, fetchAiConfig, webSearchTest,
  migrateFiles, pipelineClearVectors,
  pipelineClearKnowledge, pipelineClearAll,
  pipelineRevectorize, pipelineRetryFailed,
  pipelineDebugStatus, knowledgePipelineStatus,
  getLogs, clearLogs,
} from '@/api/db-client'
import { openUrl } from '@tauri-apps/plugin-opener'

// ─── Vite define: 替换为 Cargo.toml 中的版本号
const appVersion = __APP_VERSION__
const aiTab = ref('llm')
const guideModalRef = ref<InstanceType<typeof GuideModal> | null>(null)
const uploadProgress = useUploadProgress()

// ─── License ──────────────────────────────────────────────────────────────
const licenseActivated = ref(false)
const licenseStatus = ref('')
const licenseProduct = ref('')
const licenseExpiresAt = ref('')
const licenseActivatedAt = ref('')
const licenseMachineCode = ref('')
const licenseKey = ref('')
const licenseActivating = ref(false)
const licenseError = ref('')
const licenseSuccess = ref('')
const licenseCodeCopied = ref(false)
const licenseUpdating = ref(false)
const trialActive = ref(false)
const trialRemainingDays = ref(0)
const trialExpired = ref(false)

async function loadLicense() {
  try {
    const { fetchLicenseStatus, getMachineCode } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    licenseActivated.value = status.activated
    licenseStatus.value = status.status || ''
    licenseProduct.value = status.product || ''
    licenseExpiresAt.value = status.expiresAt || ''
    licenseActivatedAt.value = status.activatedAt || ''
    trialActive.value = !!status.trial?.active && !status.trial?.expired
    trialRemainingDays.value = status.trial?.remainingDays || 0
    trialExpired.value = !!status.trial?.expired
    // ★ 从 machine_code 命令获取原始 UUID（而非 license 表中的 HASH）
    const rawMachineCode = await getMachineCode()
    licenseMachineCode.value = rawMachineCode
  } catch (e) {
    console.warn('Failed to load license status:', e)
  }
}

async function copyMachineCode() {
  if (!licenseMachineCode.value) return
  try {
    await navigator.clipboard.writeText(licenseMachineCode.value)
    licenseCodeCopied.value = true
    setTimeout(() => { licenseCodeCopied.value = false }, 2000)
  } catch {
    // fallback
  }
}

async function activateLicenseKey() {
  const key = licenseKey.value.trim()
  if (!key) {
    licenseError.value = '请输入秘钥'
    return
  }
  licenseActivating.value = true
  licenseError.value = ''
  licenseSuccess.value = ''
  try {
    const { activateLicense } = await import('@/api/db-client')
    const result = await activateLicense(key)
    licenseActivated.value = true
    licenseStatus.value = 'active'
    licenseExpiresAt.value = result.expiresAt
    licenseProduct.value = result.product || ''
    licenseSuccess.value = '激活成功'
    licenseKey.value = ''
    // ★ 激活成功后自动关闭弹窗
    setTimeout(() => { licenseSuccess.value = ''; licenseUpdating.value = false }, 1500)
  } catch (e) {
    licenseError.value = e.response?.data?.error || e.message || '激活失败'
  } finally {
    licenseActivating.value = false
  }
}

function formatExpiryDate(dateStr) {
  if (!dateStr) return ''
  try {
    return new Date(dateStr).toISOString().slice(0, 10)
  } catch { return dateStr }
}

// Derive display product name
const licenseProductLabel = computed(() => {
  if (!licenseProduct.value) return ''
  const map: Record<string, string> = { basic: '基础版', advanced: '高级版', enhanced: 'AI增强版' }
  return map[licenseProduct.value] || licenseProduct.value
})

// ─── 全局导出 ──────────────────────────────────────────────────────
const isExporting = ref(false)
const showDomainSelector = ref(false)
const isBackingUp = ref(false)
const isRestoring = ref(false)
const selectedDomains = ref([
  { key: 'projects', label: '项目（含方案、动态、渠道）', selected: true },
  { key: 'products', label: '产品（含系列、物料）', selected: true },
  { key: 'notes', label: '笔记（含笔记本、附件）', selected: true },
  { key: 'todos', label: '待办（含附件）', selected: true },
  { key: 'files', label: '文件资料库', selected: true },
  { key: 'contacts', label: '通讯录（含合作伙伴、联系人）', selected: true },
])

async function handleExportData() {
  isExporting.value = true
  try {
    const { pickSavePath } = await import('@/api/db-client')
    const savePath = await pickSavePath('arkmind_export.ark', ['ark'])
    if (!savePath) { isExporting.value = false; return }
    const domains = selectedDomains.value.filter((d: any) => d.selected).map((d: any) => d.key)
    if (domains.length === 0) { alert({ title: '提示', message: '请至少选择一个业务域' }); isExporting.value = false; return }
    
    // ★ 关闭选择弹窗
    showDomainSelector.value = false
    
    // ★ 复用上传进度弹窗显示导出进度
    const task = uploadProgress.start('导出数据中...')
    
    // ★ 监听进度事件
    const { listen } = await import('@tauri-apps/api/event')
    const unlisten = await listen('export:progress', (event: any) => {
      const { current, total, domain, stage } = event.payload
      if (domain) {
        task.name = `导出中... ${domain} (${current}/${total})`
      }
      if (stage === 'done') {
        uploadProgress.done(task)
        toast.success(`数据已导出至：${savePath}`)
        unlisten()
      }
    })
    
    const { exportAllData } = await import('@/api/db-client')
    await exportAllData(domains, savePath)
  } catch (e) {
    alert({ title: '导出失败', message: (e && e.message) || '未知错误' })
  } finally {
    isExporting.value = false
  }
}

// ─── 全局导入 ──────────────────────────────────────────────────────
const isImporting = ref(false)
async function handleImportData() {
  isImporting.value = true
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      filters: [{ name: 'ArkMind 数据包', extensions: ['ark'] }],
      multiple: false,
    })
    if (!filePath) { isImporting.value = false; return }
    const { importData } = await import('@/api/db-client')
    const result = await importData(filePath as string)
    if (result.success) {
      toast.success(`数据已导入：${(result.imported || []).join('、')}`)
    } else {
      alert({ title: '导入失败', message: result.error || '未知错误' })
    }
  } catch (e) {
    alert({ title: '导入失败', message: (e && e.message) || '未知错误' })
  } finally {
    isImporting.value = false
  }
}

// ─── 一键备份/恢复 ──────────────────────────────────────────────────
async function handleBackupAll() {
  isBackingUp.value = true
  try {
    const { pickSavePath } = await import('@/api/db-client')
    const savePath = await pickSavePath(`arkmind-backup-${new Date().toISOString().slice(0,10)}.zip`, ['zip'])
    if (!savePath) { isBackingUp.value = false; return }

    // 复用上传进度弹窗显示备份进度
    const task = uploadProgress.start('备份中...')

    const { listen } = await import('@tauri-apps/api/event')
    const unlisten = await listen('backup:progress', (event: any) => {
      const { stage, step, total, name, done, pct } = event.payload
      if (stage === 'exporting') {
        task.name = `备份中... ${name} (${step}/${total})`
      }
      if (stage === 'zipping') {
        task.name = `备份中... 打包 ${pct}%（${done}/${total} 文件）`
      }
      if (stage === 'done') {
        uploadProgress.done(task)
        toast.success(`备份已保存到：${savePath}`)
        unlisten()
      }
    })

    const { backupAll } = await import('@/api/db-client')
    const result = await backupAll(savePath)
    if (!result.success) {
      uploadProgress.fail(task, result.error || '未知错误')
    }
  } catch (e) {
    alert({ title: '备份失败', message: (e && e.message) || '未知错误' })
  } finally {
    isBackingUp.value = false
  }
}

async function handleRestoreAll() {
  const ok = await dangerConfirm({
    title: '一键恢复',
    message: '恢复将覆盖当前所有数据（数据库、文件、向量库）。恢复完成后应用将自动重启。是否继续？',
    confirmText: '确认恢复',
  })
  if (!ok) return
  isRestoring.value = true
  try {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const filePath = await open({
      filters: [{ name: 'ZIP 文件', extensions: ['zip'] }],
      multiple: false,
    })
    if (!filePath) { isRestoring.value = false; return }

    // 复用上传进度弹窗显示恢复进度
    const task = uploadProgress.start('恢复中...')

    const { listen } = await import('@tauri-apps/api/event')
    const unlisten = await listen('restore:progress', (event: any) => {
      const { stage, step, total, name } = event.payload
      if (stage === 'exporting') {
        task.name = `恢复中... ${name} (${step}/${total})`
      }
      if (stage === 'done') {
        uploadProgress.done(task)
        unlisten()
      }
    })

    const { restoreAll } = await import('@/api/db-client')
    const result = await restoreAll(filePath as string)
    if (!result.success) {
      uploadProgress.fail(task, result.error || '未知错误')
    }
  } catch (e) {
    alert({ title: '恢复失败', message: (e && e.message) || '未知错误' })
  } finally {
    isRestoring.value = false
  }
}

// ─── AI Config ───────────────────────────────────────────────────────────
// Multi-provider support: each provider has { id, name, apiKey, baseUrl, modelName }
const aiProviders = ref([])
const activeProviderId = ref(null) // system default (saved)
const pendingDefaultId = ref(null) // local radio selection before save

// ─── Preset Models ────────────────────────────────────────────────────
interface PresetModel { name: string; baseUrl: string; modelName: string; website: string }
const LLM_PRESETS: PresetModel[] = [
  { name: '深度求索 DeepSeek V4 Pro', baseUrl: 'https://api.deepseek.com/v1', modelName: 'deepseek-v4-pro', website: 'https://platform.deepseek.com' },
  { name: '深度求索 DeepSeek V4 Flash', baseUrl: 'https://api.deepseek.com/v1', modelName: 'deepseek-v4-flash', website: 'https://platform.deepseek.com' },
  { name: '小米 MiMo V2.5 Pro', baseUrl: 'https://api.xiaomimimo.com/v1', modelName: 'mimo-v2.5-pro', website: 'https://platform.xiaomimimo.com/console/balance' },
  { name: '小米 MiMo V2.5', baseUrl: 'https://api.xiaomimimo.com/v1', modelName: 'mimo-v2.5', website: 'https://platform.xiaomimimo.com/console/balance' },
]
const VISION_PRESETS: PresetModel[] = [
  { name: '小米 MiMo V2.5', baseUrl: 'https://api.xiaomimimo.com/v1', modelName: 'mimo-v2.5', website: 'https://platform.xiaomimimo.com/console/balance' },
  { name: '阿里通义千问 Qwen3.5 Flash', baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', modelName: 'qwen3.5-flash', website: 'https://platform.qianwenai.com/home' },
]
const VEC_PRESETS: PresetModel[] = [
  { name: '硅基流动 BAAI/bge-m3', baseUrl: 'https://api.siliconflow.cn/v1', modelName: 'BAAI/bge-m3', website: 'https://siliconflow.cn' },
  { name: '阿里通义千问 Qwen3.7 Text Embedding', baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1', modelName: 'qwen3.7-text-embedding', website: 'https://platform.qianwenai.com/home' },
]
const selectedPreset = ref('')
const selectedVisionPreset = ref('')
const selectedVecPreset = ref('')
const presetDropdownOpen = ref(false)
const visionPresetDropdownOpen = ref(false)
const vecPresetDropdownOpen = ref(false)

function selectPreset(type: 'llm' | 'vision' | 'vec', preset: PresetModel) {
  if (type === 'llm') { selectedPreset.value = preset.name; providerForm.value.baseUrl = preset.baseUrl; providerForm.value.modelName = preset.modelName; presetDropdownOpen.value = false }
  else if (type === 'vision') { selectedVisionPreset.value = preset.name; visionForm.value.baseUrl = preset.baseUrl; visionForm.value.modelName = preset.modelName; visionPresetDropdownOpen.value = false }
  else { selectedVecPreset.value = preset.name; vecForm.value.baseUrl = preset.baseUrl; vecForm.value.modelName = preset.modelName; vecPresetDropdownOpen.value = false }
}

function applyPreset(type: 'llm' | 'vision' | 'vec') {
  const presets = type === 'llm' ? LLM_PRESETS : type === 'vision' ? VISION_PRESETS : VEC_PRESETS
  const preset = presets.find(p => p.name === (type === 'llm' ? selectedPreset.value : type === 'vision' ? selectedVisionPreset.value : selectedVecPreset.value))
  if (!preset) return
  if (type === 'llm') { providerForm.value.baseUrl = preset.baseUrl; providerForm.value.modelName = preset.modelName }
  else if (type === 'vision') { visionForm.value.baseUrl = preset.baseUrl; visionForm.value.modelName = preset.modelName }
  else { vecForm.value.baseUrl = preset.baseUrl; vecForm.value.modelName = preset.modelName }
}

// Modal state for add/edit provider
const showProviderModal = ref(false)
const editingProvider = ref(null)
const providerForm = ref({
  apiKey: '',
  baseUrl: 'https://api.openai.com/v1',
  modelName: '',
})
const providerFormSaving = ref(false)
const providerFormError = ref('')

const aiSaving = ref(false)
const aiSaved = ref(false)
const aiTesting = ref(false)
const aiTestResult = ref('')
const testingProviderId = ref(null)

function generateProviderId() {
  return 'ap_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 8)
}

async function loadAiConfig() {
  try {
    const config = await fetchAiConfig()
    if (config.providers) {
      aiProviders.value = config.providers.map(p => ({
        ...p,
        apiKey: p.apiKey || '',
      }))
    } else if (config.apiKey) {
      aiProviders.value = [{
        id: 'default',
        name: config.modelName || 'Default',
        apiKey: config.apiKey || '',
        baseUrl: config.baseUrl || 'https://api.openai.com/v1',
        modelName: config.modelName || 'gpt-4o-mini',
      }]
    } else {
      aiProviders.value = []
    }
    activeProviderId.value = config.activeProviderId || aiProviders.value[0]?.id || null
    pendingDefaultId.value = activeProviderId.value
    // 如果默认模型不在列表中，自动切换到第一个可用模型
    if (activeProviderId.value && !aiProviders.value.find(p => p.id === activeProviderId.value)) {
      activeProviderId.value = aiProviders.value[0]?.id || null
      pendingDefaultId.value = activeProviderId.value
    }
    loadVecConfig(config)
    loadVisionConfig(config)
    webSearchKey.value = config.webSearch?.apiKey || ''
    loadNotifyConfig()
    loadSpriteConfig()
  } catch (e) {
    console.warn('Failed to load AI config:', e)
  }
}

function openAddProvider() {
  editingProvider.value = null
  providerForm.value = { apiKey: '', baseUrl: '', modelName: '' }
  providerFormError.value = ''
  selectedPreset.value = ''
  showProviderModal.value = true
}

function openEditProvider(provider) {
  editingProvider.value = provider
  providerForm.value = {
    apiKey: provider.apiKey,
    baseUrl: provider.baseUrl,
    modelName: provider.modelName,
  }
  providerFormError.value = ''
  showProviderModal.value = true
}

function closeProviderModal() {
  showProviderModal.value = false
  editingProvider.value = null
}

async function confirmProviderForm() {
  const { apiKey: formKey, baseUrl, modelName } = providerForm.value
  providerFormError.value = ''
  if (!modelName.trim()) { providerFormError.value = '请输入模型名称'; return }
  if (!editingProvider.value && !formKey.trim()) { providerFormError.value = '请输入 API Key'; return }

  const modelNameTrimmed = modelName.trim()

  providerFormSaving.value = true

  if (editingProvider.value) {
    const idx = aiProviders.value.findIndex(p => p.id === editingProvider.value.id)
    if (idx >= 0) {
      const updated = {
        ...aiProviders.value[idx],
        baseUrl: baseUrl.trim() || 'https://api.openai.com/v1',
        modelName: modelNameTrimmed,
      }
      if (formKey.trim() && !formKey.includes('●●●●')) {
        updated.apiKey = formKey.trim()
      }
      aiProviders.value[idx] = updated
    }
  } else {
    aiProviders.value.push({
      id: generateProviderId(),
      name: modelNameTrimmed,
      apiKey: formKey.trim(),
      baseUrl: baseUrl.trim() || 'https://api.openai.com/v1',
      modelName: modelNameTrimmed,
    })
    // 第一个添加的提供商自动设为默认
    if (aiProviders.value.length === 1) {
      activeProviderId.value = aiProviders.value[0].id
    }
  }

  // Auto-save to backend
  try {
    await updateAiConfig({
      providers: aiProviders.value,
      activeProviderId: activeProviderId.value,
    })
    await loadAiConfig()
  } catch (e) {
    providerFormError.value = '保存失败: ' + (e.message || '网络异常')
    providerFormSaving.value = false
    return
  }

  providerFormSaving.value = false
  showProviderModal.value = false
  editingProvider.value = null
}

async function removeProvider(provider) {
  const ok = await dangerConfirm({ title: '删除模型', message: `确定删除 LLM 模型「${provider.modelName}」？`, confirmText: '删除' })
  if (!ok) return
  const prevProviders = [...aiProviders.value]
  const prevActive = activeProviderId.value
  const prevPending = pendingDefaultId.value
  aiProviders.value = aiProviders.value.filter(p => p.id !== provider.id)
  if (activeProviderId.value === provider.id) {
    activeProviderId.value = aiProviders.value[0]?.id || null
  }
  if (pendingDefaultId.value === provider.id) {
    pendingDefaultId.value = activeProviderId.value
  }
  // Auto-save（失败回滚列表并提示，避免"显示已删、重启复活"）
  try {
    await updateAiConfig({
      providers: aiProviders.value,
      activeProviderId: activeProviderId.value,
    })
    toast({ message: '已删除模型', type: 'success' })
  } catch {
    aiProviders.value = prevProviders
    activeProviderId.value = prevActive
    pendingDefaultId.value = prevPending
    toast({ message: '删除失败，请重试', type: 'error' })
  }
}

function setPendingDefault(providerId) {
  pendingDefaultId.value = providerId
}

async function saveDefaultConfig() {
  aiSaving.value = true
  aiTestResult.value = ''
  try {
    await updateAiConfig({
      providers: aiProviders.value,
      activeProviderId: pendingDefaultId.value,
    })
    activeProviderId.value = pendingDefaultId.value
    aiSaved.value = true
    setTimeout(() => { aiSaved.value = false }, 2000)
    await loadAiConfig()
  } catch (e) {
    alert('保存失败: ' + (e.message || '网络异常'))
  } finally {
    aiSaving.value = false
  }
}

// ─── Vision Model (Multi-Provider, same as LLM) ────────────────
const visionProviders = ref<any[]>([])
const visionActiveId = ref<string | null>(null)
const visionPendingId = ref<string | null>(null)
const showVisionModal = ref(false)
const editingVision = ref<any>(null)
const visionForm = ref({ apiKey: '', baseUrl: 'https://api.openai.com/v1', modelName: '', apiFormat: 'openai' })
const visionFormSaving = ref(false)
const visionFormError = ref('')
const visionSaving = ref(false)
const visionSaved = ref(false)
const visionTesting = ref(false)
const visionTestResult = ref('')
const visionTestingId = ref(null)

function loadVisionConfig(config) {
  const vp = config?.visionProviders
  visionProviders.value = vp ? vp.map((p: any) => ({ ...p, apiKey: p.apiKey || '' })) : []
  visionActiveId.value = config?.activeVisionId || visionProviders.value[0]?.id || null
  visionPendingId.value = visionActiveId.value
  if (visionActiveId.value && !visionProviders.value.find((p: any) => p.id === visionActiveId.value)) {
    visionActiveId.value = visionProviders.value[0]?.id || null
    visionPendingId.value = visionActiveId.value
  }
}

function openAddVision() {
  editingVision.value = null
  visionForm.value = { apiKey: '', baseUrl: '', modelName: '', apiFormat: 'openai' }
  visionFormError.value = ''
  selectedVisionPreset.value = ''
  showVisionModal.value = true
}

function openEditVision(provider) {
  editingVision.value = provider
  visionForm.value = { apiKey: provider.apiKey, baseUrl: provider.baseUrl, modelName: provider.modelName, apiFormat: provider.apiFormat || 'openai' }
  visionFormError.value = ''
  showVisionModal.value = true
}

function closeVisionModal() {
  showVisionModal.value = false
  editingVision.value = null
}

async function confirmVisionForm() {
  const { apiKey: formKey, baseUrl, modelName } = visionForm.value
  visionFormError.value = ''
  if (!modelName.trim()) { visionFormError.value = '请输入模型名称'; return }
  if (!editingVision.value && !formKey.trim()) { visionFormError.value = '请输入 API Key'; return }
  const modelNameTrimmed = modelName.trim()
  visionFormSaving.value = true
  if (editingVision.value) {
    const idx = visionProviders.value.findIndex((p: any) => p.id === editingVision.value.id)
    if (idx >= 0) {
      const updated: any = {
        ...visionProviders.value[idx],
        baseUrl: baseUrl.trim() || 'https://api.openai.com/v1',
        modelName: modelNameTrimmed,
        apiFormat: visionForm.value.apiFormat || 'openai',
      }
      if (formKey.trim() && !formKey.includes('●●●●')) { updated.apiKey = formKey.trim() }
      visionProviders.value[idx] = updated
    }
  } else {
    const newId = 'vp_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 8)
    visionProviders.value.push({ id: newId, name: modelNameTrimmed, apiKey: formKey.trim(), baseUrl: baseUrl.trim() || 'https://api.openai.com/v1', modelName: modelNameTrimmed, apiFormat: visionForm.value.apiFormat || 'openai' })
    if (visionProviders.value.length === 1) { visionActiveId.value = visionProviders.value[0].id }
  }
  try {
    await updateAiConfig({
      visionProviders: visionProviders.value,
      activeVisionId: visionActiveId.value,
    })
    await loadAiConfig()
  } catch (e: any) {
    visionFormError.value = '保存失败: ' + (e.message || '网络异常')
    visionFormSaving.value = false
    return
  }
  visionFormSaving.value = false
  showVisionModal.value = false
  editingVision.value = null
}

async function removeVision(provider: any) {
  const ok = await dangerConfirm({ title: '删除模型', message: `确定删除 视觉模型「${provider.modelName}」？`, confirmText: '删除' })
  if (!ok) return
  const prevProviders = [...visionProviders.value]
  const prevActive = visionActiveId.value
  const prevPending = visionPendingId.value
  visionProviders.value = visionProviders.value.filter((p: any) => p.id !== provider.id)
  if (visionActiveId.value === provider.id) { visionActiveId.value = visionProviders.value[0]?.id || null }
  if (visionPendingId.value === provider.id) { visionPendingId.value = visionActiveId.value }
  try {
    await updateAiConfig({
      visionProviders: visionProviders.value,
      activeVisionId: visionActiveId.value,
    })
    toast({ message: '已删除模型', type: 'success' })
  } catch {
    visionProviders.value = prevProviders
    visionActiveId.value = prevActive
    visionPendingId.value = prevPending
    toast({ message: '删除失败，请重试', type: 'error' })
  }
}

function setVisionPending(id) { visionPendingId.value = id }

async function saveVisionDefault() {
  visionSaving.value = true
  visionTestResult.value = ''
  try {
    await updateAiConfig({
      visionProviders: visionProviders.value,
      activeVisionId: visionPendingId.value,
    })
    visionActiveId.value = visionPendingId.value
    visionSaved.value = true
    setTimeout(() => { visionSaved.value = false }, 2000)
    await loadAiConfig()
  } catch (e: any) {
    alert('保存失败: ' + (e.message || '网络异常'))
  } finally {
    visionSaving.value = false
  }
}

// 联网搜索（博查）配置：保存/清除即时生效，测试按钮先落盘再探测
const webSearchKey = ref('')
const webSearchSaving = ref(false)
const webSearching = ref(false)
const webSearchMsg = ref('')
const webSearchOk = ref(false)

async function applyWebSearchConfig(): Promise<boolean> {
  const key = webSearchKey.value.trim()
  if (key) await updateAiConfig({ webSearch: { apiKey: key } })
  else await updateAiConfig({ webSearch: null })
  return true
}

async function saveWebSearch() {
  if (webSearchSaving.value) return
  webSearchSaving.value = true
  webSearchMsg.value = ''
  try {
    await applyWebSearchConfig()
    toast({ message: webSearchKey.value.trim() ? '联网搜索已启用' : '联网搜索已清除', type: 'success' })
    await loadAiConfig()
  } catch (e: any) {
    toast({ message: '保存失败: ' + (e?.message || e), type: 'error' })
  } finally {
    webSearchSaving.value = false
  }
}

async function testWebSearch() {
  if (webSearching.value) return
  webSearching.value = true
  webSearchMsg.value = ''
  try {
    await applyWebSearchConfig()
    const r = await webSearchTest()
    webSearchOk.value = r.success
    webSearchMsg.value = r.success ? '连接正常，联网能力已就绪' : r.message
    if (r.success) await loadAiConfig()
  } catch (e: any) {
    webSearchOk.value = false
    webSearchMsg.value = String(e?.message || e)
  } finally {
    webSearching.value = false
  }
}

async function testVisionConnection(provider?: any) {
  const base = editingVision.value || provider || {}
  const form = (provider ? {} : visionForm.value)
  const p = {
    ...base,
    baseUrl: form.baseUrl || base.baseUrl,
    apiKey: form.apiKey || base.apiKey,
    modelName: form.modelName || base.modelName,
    id: base.id || '__form__',
  }
  visionTestingId.value = p.id || '__form__'
  visionTestResult.value = ''
  visionTesting.value = true
  const label = p.modelName || p.name || '模型'
  try {
    const { aiTestProvider } = await import('@/api/db-client')
    const r = await aiTestProvider({ baseUrl: p.baseUrl, apiKey: p.apiKey, modelName: p.modelName })
    visionTestResult.value = r.success ? (r.message || `「${label}」连接成功`) : (r.error || `「${label}」连接失败`)
  } catch (e: any) {
    visionTestResult.value = `「${label}」连接失败: ` + (e.message || '网络错误')
  } finally {
    visionTesting.value = false
    visionTestingId.value = null
  }
}

// ─── Vector Engine (单一嵌入模型配置) ────────────────────────────
const vecForm = ref({ baseUrl: 'https://api.openai.com/v1', apiKey: '', modelName: 'text-embedding-3-small' })
const vecSaving = ref(false)
const vecSaved = ref(false)
const vecTesting = ref(false)
const vecTestResult = ref('')
const showVecModal = ref(false)
const vecFormError = ref('')
const vecSavedData = ref<any>(null)
const vecConfigured = computed(() => !!(vecSavedData.value?.apiKey))

// ─── Notification Settings ──────────────────────────────────
import { notificationConfigGet, notificationConfigUpdate, notificationLoopStatus, profileFactsList, profileFactDelete, profileFactSave, type LoopStatus, type ProfileFact } from '@/api/notification-client'
import { getSpriteEnabled, setSpriteEnabled, kbDeleteSession, fetchUserProfile, updateUserProfile } from '@/api/db-client'

const notifyEnabled = ref(true)
const notifyInterval = ref(60)
const notifyIntervalOpen = ref(false)
// ─── 分析师配置（附加要求=单主体弹窗编辑；核心规则与观察池=只读透明）───
const focusText = ref('')                 // analysis_focus 正文（整体一段提示词）
const focusModalOpen = ref(false)         // 编辑弹窗
const focusDraft = ref('')                // 弹窗草稿
const focusSaving = ref(false)
const FOCUS_MAX = 2000
const coreRulesText = ref('')             // 后端下发的内核提示词（只读）
const coreRulesOpen = ref(false)
const watchText = ref('')                 // 当前观察池（只读）
const watchOpen = ref(false)
const loopStatus = ref<LoopStatus>({ success: true, running: false, last_check_at: '', next_check_at: '' })
let loopStatusTimer: ReturnType<typeof setInterval> | null = null

const notifyIntervalOptions = [
  { value: 0, label: '仅启动时' },
  { value: 60, label: '1 小时' },
  { value: 360, label: '6 小时' },
  { value: 720, label: '12 小时' },
  { value: 1440, label: '24 小时' },
]
const notifyIntervalLabel = computed(() => notifyIntervalOptions.find(o => o.value === notifyInterval.value)?.label || '1 小时')

// ─── Sprite Settings ────────────────────────────────────────
const spriteEnabled = ref(false)

async function loadSpriteConfig() {
  try {
    spriteEnabled.value = await getSpriteEnabled()
  } catch {}
}

async function toggleSprite() {
  try {
    await setSpriteEnabled(spriteEnabled.value)
  } catch {
    spriteEnabled.value = !spriteEnabled.value
    toast({ message: '精灵开关保存失败', type: 'error' })
  }
}

async function clearSpriteChat() {
  const ok = await dangerConfirm({
    title: '清空精灵对话',
    message: '确定清空桌面精灵的所有聊天记录？此操作不可撤销。',
    confirmText: '清空',
  })
  if (!ok) return
  try {
    await kbDeleteSession('fab-main-session')
    toast.success('精灵对话已清空')
  } catch (e) {
    toast.error('清空失败')
  }
}

async function loadNotifyConfig() {
  try {
    const r = await notificationConfigGet()
    if (r.config) {
      notifyEnabled.value = r.config.enabled !== '0'
      notifyInterval.value = r.config.startup_only === '1' ? 0 : (parseInt(r.config.interval_minutes) || 60)
      focusText.value = (r.config.analysis_focus || '').trim()
    }
    coreRulesText.value = (r as any).analysis_prompt_default || ''
    watchText.value = ((r as any).analyst_watch || '').trim()
  } catch {}
}

function openFocusModal() {
  focusDraft.value = focusText.value
  focusModalOpen.value = true
}
function closeFocusModal() { focusModalOpen.value = false }

async function saveFocusModal() {
  const val = focusDraft.value.trim()
  if (val.length > FOCUS_MAX) { toast({ message: `不能超过 ${FOCUS_MAX} 字符`, type: 'error' }); return }
  focusSaving.value = true
  try {
    await notificationConfigUpdate({ analysis_focus: val })
    focusText.value = val
    focusModalOpen.value = false
    toast({ message: val ? '附加要求已保存，下次分析生效' : '已清空附加要求', type: 'success' })
  } catch (e: any) {
    toast({ message: e?.message || '保存失败', type: 'error' })
  } finally {
    focusSaving.value = false
  }
}

async function clearFocusRequirements() {
  if (!(await dangerConfirm({ title: '清空附加要求', message: '确定清空全部附加分析要求？分析师将只按核心规则运行。', confirmText: '清空' }))) return
  focusSaving.value = true
  try {
    await notificationConfigUpdate({ analysis_focus: '' })
    focusText.value = ''
    focusDraft.value = ''
    toast({ message: '已清空附加要求', type: 'success' })
  } catch {
    toast({ message: '操作失败', type: 'error' })
  } finally {
    focusSaving.value = false
  }
}

async function loadLoopStatus() {
  try {
    loopStatus.value = await notificationLoopStatus()
  } catch {}
}

async function saveNotifyConfig() {
  try {
    await notificationConfigUpdate({
      enabled: notifyEnabled.value ? '1' : '0',
      interval_minutes: notifyInterval.value === 0 ? '60' : notifyInterval.value.toString(),
      startup_only: notifyInterval.value === 0 ? '1' : '0',
    })
  } catch {
    notifyEnabled.value = !notifyEnabled.value
    toast({ message: '通知设置保存失败', type: 'error' })
  }
}

// ─── Pipeline Management ──────────────────────────────────────
const pipelineData = ref<any>(null)
const pipelineRunning = ref(false)

// ─── Email Account Config ────────────────────────────────────
import { emailListAccounts, emailAddAccount, emailUpdateAccount, emailDeleteAccount, emailLoopStatus, emailConfigGet, emailConfigUpdate, type EmailAccount } from '@/api/email-client'

const emailAccounts = ref<EmailAccount[]>([])
const emailLoopRunning = ref(false)
const showEmailForm = ref(false)
const emailSaving = ref(false)
const editingEmailId = ref<string | null>(null)
const emailForm = ref({ name: '', emailAddress: '', imapHost: '', imapPort: 993, username: '', password: '' })

async function loadEmailAccounts() {
  try {
    const res = await emailListAccounts()
    if (res.success) emailAccounts.value = res.accounts || []
  } catch {}
}
async function loadEmailConfig() {
  try {
    const [status] = await Promise.all([emailLoopStatus(), emailConfigGet()])
    emailLoopRunning.value = status?.running || false
  } catch {}
}
function openAddEmail() {
  editingEmailId.value = null
  emailForm.value = { name: '', emailAddress: '', imapHost: '', imapPort: 993, username: '', password: '' }
  showEmailForm.value = true
}
function editEmailAccount(acc: EmailAccount) {
  editingEmailId.value = acc.id
  emailForm.value = {
    name: acc.name,
    emailAddress: acc.email_address,
    imapHost: acc.imap_host,
    imapPort: acc.imap_port,
    username: acc.username,
    password: '',
  }
  showEmailForm.value = true
}
async function addEmailAccount() {
  const f = emailForm.value
  if (!f.name || !f.emailAddress || !f.imapHost) {
    toast({ message: '请填写完整信息', type: 'error' }); return
  }
  // 编辑模式下密码可以为空（表示不修改）
  if (!editingEmailId.value && !f.password) {
    toast({ message: '请输入密码/授权码', type: 'error' }); return
  }
  emailSaving.value = true
  try {
    if (editingEmailId.value) {
      // 编辑模式：更新账户
      const data: any = {
        name: f.name,
        email_address: f.emailAddress,
        imap_host: f.imapHost,
        imap_port: f.imapPort,
        username: f.username,
      }
      if (f.password) data.password = f.password
      const res = await emailUpdateAccount(editingEmailId.value, data)
      if (res.success) {
        toast({ message: '账户已更新', type: 'success' })
        showEmailForm.value = false
        editingEmailId.value = null
        emailForm.value = { name: '', emailAddress: '', imapHost: '', imapPort: 993, username: '', password: '' }
        await loadEmailAccounts()
      } else {
        toast({ message: '更新失败', type: 'error' })
      }
    } else {
      // 添加模式
      const res = await emailAddAccount(f)
      if (res.success) {
        toast({ message: '账户已添加', type: 'success' })
        showEmailForm.value = false
        emailForm.value = { name: '', emailAddress: '', imapHost: '', imapPort: 993, username: '', password: '' }
        await loadEmailAccounts()
      } else {
        toast({ message: res.error || '添加失败', type: 'error' })
      }
    }
  } catch (e: any) {
    toast({ message: e?.message || '操作失败', type: 'error' })
  } finally { emailSaving.value = false }
}
async function removeEmailAccount(id: string) {
  const acc = emailAccounts.value.find(a => a.id === id)
  const ok = await dangerConfirm({ title: '删除邮箱账户', message: `确定删除账户「${acc?.email_address || acc?.name || id}」？该账户的已同步邮件将一并删除，此操作不可撤销。`, confirmText: '删除' })
  if (!ok) return
  try {
    await emailDeleteAccount(id)
    emailAccounts.value = emailAccounts.value.filter(a => a.id !== id)
    toast({ message: '账户已删除', type: 'success' })
  } catch { toast({ message: '删除失败', type: 'error' }) }
}

// ─── Agent Profile Management ──────────────────────────────────
import { fetchAgentProfiles, fetchAgentAvailableTools, createAgentProfile, updateAgentProfile, deleteAgentProfile, resetAgentProfile, type AgentProfile, type AgentToolInfo } from '@/api/db-client'

const agentProfiles = ref<AgentProfile[]>([])
const availableTools = ref<AgentToolInfo[]>([])
const showAgentModal = ref(false)
const agentSaving = ref(false)
const editingAgentId = ref<string | null>(null)
const agentForm = ref({
  name: '',
  description: '',
  system_prompt: '',
  max_iterations: 20,
  can_delegate: false,
  tools: [] as string[],
  presetTools: [] as string[],
})
const agentFormError = ref('')

async function loadAgentProfiles() {
  try {
    agentProfiles.value = await fetchAgentProfiles()
  } catch {}
}

async function loadAvailableTools() {
  try {
    availableTools.value = await fetchAgentAvailableTools()
  } catch {}
}

function openAddAgent() {
  editingAgentId.value = null
  agentForm.value = {
    name: '',
    description: '',
    system_prompt: '',
    max_iterations: 20,
    can_delegate: false,
    tools: [],
    presetTools: [],
  }
  agentFormError.value = ''
  showAgentModal.value = true
}

function editAgent(agent: AgentProfile) {
  editingAgentId.value = agent.id
  const presetTools = Array.isArray(agent.preset_tools) ? agent.preset_tools : (typeof agent.preset_tools === 'string' && agent.preset_tools ? JSON.parse(agent.preset_tools) : [])
  agentForm.value = {
    name: agent.name,
    description: agent.description || '',
    system_prompt: agent.system_prompt || '',
    max_iterations: agent.max_iterations || 20,
    can_delegate: !!agent.can_delegate,
    tools: agent.tools || [],
    presetTools: presetTools,
  }
  agentFormError.value = ''
  showAgentModal.value = true
}

function closeAgentModal() {
  showAgentModal.value = false
  editingAgentId.value = null
  agentFormError.value = ''
}

function toggleAgentTool(toolName: string) {
  const idx = agentForm.value.tools.indexOf(toolName)
  if (idx >= 0) {
    agentForm.value.tools.splice(idx, 1)
  } else {
    agentForm.value.tools.push(toolName)
  }
}

const toolLabels: Record<string, { name: string; brief: string }> = {
  ext_tool_search: { name: '知识搜索', brief: '全文+语义检索文档、笔记、技术资料' },
  ext_tool_object: { name: '数据查询', brief: '查看/筛选/统计业务对象' },
  ext_tool_chunk_navigate: { name: '文档翻页', brief: '在知识块间前后翻页阅读' },
  ext_tool_relations: { name: '关联查询', brief: '查看实体间的关联关系' },
  ext_tool_manage_project: { name: '项目管理', brief: '创建/编辑/删除项目及动态' },
  ext_tool_manage_todo: { name: '待办管理', brief: '创建/编辑/删除待办事项' },
  ext_tool_manage_partner: { name: '合作伙伴', brief: '管理渠道/厂商/客户' },
  ext_tool_manage_contact: { name: '联系人', brief: '管理合作伙伴下的联系人' },
  ext_tool_manage_product: { name: '产品管理', brief: '管理产品及产品目录' },
  ext_tool_manage_note: { name: '笔记管理', brief: '创建/编辑/删除笔记' },
  ext_tool_manage_notification: { name: '通知管理', brief: '创建/查看系统通知' },
  ext_tool_web_search: { name: '联网搜索', brief: '搜索公开互联网信息' },
  ext_tool_web_fetch: { name: '网页抓取', brief: '抓取网页全文转为 Markdown' },
}

async function saveAgent() {
  const f = agentForm.value
  if (!f.name.trim()) {
    agentFormError.value = '请输入 Agent 名称'
    return
  }
  if (!f.description.trim()) {
    agentFormError.value = '请输入委托说明——主 Agent 靠它决定何时派活'
    return
  }
  if (!f.system_prompt.trim()) {
    agentFormError.value = '请输入系统提示词'
    return
  }
  agentSaving.value = true
  agentFormError.value = ''
  try {
    // 确保预设工具始终包含在内
    const allTools = [...new Set([...f.tools, ...f.presetTools])]
    if (editingAgentId.value) {
      await updateAgentProfile(editingAgentId.value, {
        name: f.name,
        description: f.description,
        system_prompt: f.system_prompt,
        max_iterations: f.max_iterations,
        can_delegate: f.can_delegate ? 1 : 0,
        tools: allTools,
      })
      toast({ message: 'Agent 已更新', type: 'success' })
    } else {
      await createAgentProfile({
        name: f.name,
        description: f.description,
        system_prompt: f.system_prompt,
        max_iterations: f.max_iterations,
        can_delegate: f.can_delegate ? 1 : 0,
        tools: allTools,
      })
      toast({ message: 'Agent 已创建', type: 'success' })
    }
    closeAgentModal()
    await loadAgentProfiles()
  } catch (e: any) {
    agentFormError.value = e?.message || '保存失败'
  } finally {
    agentSaving.value = false
  }
}

async function removeAgent(id: string) {
  const agent = agentProfiles.value.find(a => a.id === id)
  const ok = await dangerConfirm({ title: '删除 Agent', message: `确定删除「${agent?.name || id}」？关联的工具分配将同时删除，此操作不可撤销。`, confirmText: '删除' })
  if (!ok) return
  try {
    await deleteAgentProfile(id)
    agentProfiles.value = agentProfiles.value.filter(a => a.id !== id)
    toast({ message: 'Agent 已删除', type: 'success' })
  } catch (e: any) {
    toast({ message: e?.message || '删除失败', type: 'error' })
  }
}

async function resetAgent(id: string) {
  const agent = agentProfiles.value.find(a => a.id === id)
  const ok = await dangerConfirm({ title: '还原 Agent', message: `确定将「${agent?.name || id}」还原到默认配置？当前配置将被覆盖且无法恢复。`, confirmText: '还原' })
  if (!ok) return
  try {
    await resetAgentProfile(id)
    await loadAgentProfiles()
    toast({ message: 'Agent 已还原到默认配置', type: 'success' })
  } catch (e: any) {
    toast({ message: e?.message || '还原失败', type: 'error' })
  }
}

async function toggleAgentStatus(agent: AgentProfile) {
  const newStatus = agent.status === 'active' ? 'disabled' : 'active'
  try {
    await updateAgentProfile(agent.id, { status: newStatus })
    agent.status = newStatus
    toast({ message: newStatus === 'active' ? 'Agent 已启用' : 'Agent 已禁用', type: 'success' })
  } catch (e: any) {
    toast({ message: e?.message || '操作失败', type: 'error' })
  }
}

// vacuum 时间戳为 epoch 秒字符串
function formatVacuumTime(epochSecs: string): string {
  const n = parseInt(epochSecs, 10)
  if (!n) return ''
  const d = new Date(n * 1000)
  return `${d.getMonth() + 1}-${d.getDate()} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}
async function loadPipeline() {
  try {
    const [debug, status] = await Promise.all([pipelineDebugStatus(), knowledgePipelineStatus()])
    pipelineData.value = debug
    pipelineRunning.value = status?.running || false
  } catch (e) { console.error('[Settings] loadPipeline failed:', e) }
}
async function handleRevectorize() {
  if (!(await dangerConfirm({ title: '重新向量化', message: '将清除所有现有向量数据并重新向量化。这可能需要较长时间，确定继续？', confirmText: '重新向量化' }))) return
  try { const r = await pipelineRevectorize(); toast({ message: r.message || '已启动重新向量化', type: 'success' }) }
  catch { toast({ message: '重新向量化失败', type: 'error' }) }
}
async function handleClearKnowledge() {
  if (!(await dangerConfirm({ title: '重置知识管线', message: '确定重置知识碎片和向量缓存？将重新从源数据物化。', confirmText: '重置' }))) return
  try { await pipelineClearKnowledge(); toast({ message: '已重置知识管线，正在重新物化', type: 'success' }) }
  catch { toast({ message: '重置失败', type: 'error' }) }
}
async function handleClearAll() {
  if (!(await dangerConfirm({ title: '重置全部数据', message: '确定重置全部数据？将重新从业务表抓取数据并重新物化知识。', confirmText: '重置' }))) return
  try { await pipelineClearAll(); toast({ message: '已重置全部数据，正在重新物化打标', type: 'success' }) }
  catch { toast({ message: '重置失败', type: 'error' }) }
}
// ─── 解析失败明细弹窗 / 重试 / 高级操作折叠 ───
const showParseErrorsModal = ref(false)
const showAdvancedOps = ref(false)
async function handleRetryFailed() {
  const count = pipelineData.value?.fileParse?.failed ?? 0
  if (!(await dangerConfirm({ title: '重试失败任务', message: `将把 ${count} 个解析失败的任务重新入队立即执行（不修复根因时结果可能相同，失败原因见明细）。`, confirmText: '重试' }))) return
  try {
    const r = await pipelineRetryFailed()
    toast({ message: `已重新入队 ${r.count ?? 0} 个任务`, type: 'success' })
    await loadPipeline()
  } catch { toast({ message: '重试请求失败', type: 'error' }) }
}

function loadVecConfig(config) {
  const e = config?.embedding
  vecForm.value = {
    baseUrl: e?.baseUrl || 'https://api.openai.com/v1',
    apiKey: e?.apiKey || '',
    modelName: e?.modelName || 'text-embedding-3-small',
  }
  if (e?.apiKey) {
    vecSavedData.value = { ...vecForm.value }
  } else {
    vecSavedData.value = null
  }
}

function openVecModal() {
  if (vecSavedData.value) {
    vecForm.value = { ...vecSavedData.value }
  } else {
    vecForm.value = { baseUrl: '', apiKey: '', modelName: '' }
  }
  vecFormError.value = ''
  vecTestResult.value = ''
  selectedVecPreset.value = ''
  showVecModal.value = true
}

function closeVecModal() {
  showVecModal.value = false
  vecFormError.value = ''
}

async function saveVecConfig() {
  vecSaving.value = true
  vecTestResult.value = ''
  try {
    const current = await fetchAiConfig()
    const oldModelName = current?.embedding?.modelName || ''
    const newModelName = vecForm.value.modelName.trim() || 'text-embedding-3-small'
    const modelChanged = oldModelName && newModelName !== oldModelName

    if (modelChanged) {
      const ok = await dangerConfirm({
        title: '更换向量模型',
        message: `模型从「${oldModelName}」变更为「${newModelName}」，不同模型的向量不可通用，需要清空已有的向量数据库并重新向量化。是否继续？`,
        confirmText: '清空并重新向量化'
      })
      if (!ok) { vecSaving.value = false; return }
    }

    await updateAiConfig({
      embedding: {
        baseUrl: vecForm.value.baseUrl.trim() || 'https://api.openai.com/v1',
        // C5 修复：空密钥或掩码 key 不写回，避免清空已保存的真实密钥
        ...(vecForm.value.apiKey.trim() && !vecForm.value.apiKey.includes('●') ? { apiKey: vecForm.value.apiKey.trim() } : {}),
        modelName: newModelName,
      },
    })

    if (modelChanged) {
      await pipelineClearVectors()
    }

    vecSaved.value = true
    vecSavedData.value = { ...vecForm.value }
    showVecModal.value = false
    setTimeout(() => { vecSaved.value = false }, 2000)
  } catch (e: any) {
    alert('保存失败: ' + (e.message || '网络异常'))
  } finally {
    vecSaving.value = false
  }
}

async function deleteVecConfig() {
  const ok = await dangerConfirm({ title: '删除向量模型', message: '确定删除向量模型配置？', confirmText: '删除' })
  if (!ok) return
  try {
    await updateAiConfig({ embedding: null })
    vecSavedData.value = null
    vecForm.value = { baseUrl: 'https://api.openai.com/v1', apiKey: '', modelName: 'text-embedding-3-small' }
    showVecModal.value = false
  } catch (e: any) {
    alert('删除失败: ' + (e.message || '网络异常'))
  }
}

function maskedVecKey(key: string) {
  if (!key) return ''
  if (key.length <= 8) return '●●●●'
  return key.slice(0, 4) + '●●●●' + key.slice(-4)
}

async function testVecConnection() {
  vecTestResult.value = ''
  vecTesting.value = true
  const { aiTestEmbedding } = await import('@/api/db-client')
  try {
    const apiKey = vecForm.value.apiKey?.trim() || vecSavedData.value?.apiKey || ''
    const r = await aiTestEmbedding({ baseUrl: vecForm.value.baseUrl, apiKey, modelName: vecForm.value.modelName })
    vecTestResult.value = r.success ? (r.message || '连接成功') : (r.error || '连接失败')
  } catch (e: any) {
    vecTestResult.value = '连接失败: ' + (e.message || '网络错误')
  } finally {
    vecTesting.value = false
  }
}

async function testProviderConnection(provider?: any) {
  // 弹窗中调用（无参数）时直接取 providerForm 的值
  // 列表中调用（有参数）时取 provider 本身的值
  let baseUrl: string, apiKey: string, modelName: string, label: string
  if (provider) {
    baseUrl = provider.baseUrl || ''
    apiKey = provider.apiKey || ''
    modelName = provider.modelName || ''
    label = modelName || provider.name || '模型'
  } else {
    const f = providerForm.value
    baseUrl = (f.baseUrl || '').trim()
    apiKey = (f.apiKey || '').trim()
    modelName = (f.modelName || '').trim()
    label = modelName || '模型'
    // 编辑模式下 apiKey 为空时，回退到原始 provider 的 apiKey
    if (!apiKey && editingProvider.value?.apiKey) {
      apiKey = editingProvider.value.apiKey
    }
  }
  testingProviderId.value = provider?.id || '__form__'
  aiTestResult.value = ''
  aiTesting.value = true
  try {
    const { aiTestProvider } = await import('@/api/db-client')
    const data: any = { baseUrl, apiKey, modelName }
    const result = await aiTestProvider(data)
    if (result.success) {
      aiTestResult.value = result.message || `「${label}」连接成功`
    } else {
      aiTestResult.value = result.error || `「${label}」连接失败`
    }
  } catch (e: any) {
    aiTestResult.value = `「${label}」连接失败: ` + (e.message || '网络错误')
  } finally {
    aiTesting.value = false
    testingProviderId.value = null
  }
}

const maskedApiKey = (key) => {
  if (!key) return ''
  if (key.includes('●●●●')) return key
  if (key.length <= 8) return '●●●●'
  return key.slice(0, 4) + '●●●●' + key.slice(-4)
}

// ─── Storage Config ──────────────────────────────────────────────────────
const activeStorage = ref('LOCAL')
const localPath = ref('')
const savedLocalPath = ref('')
const isEditingPath = ref(false)

const storageSaving = ref(false)
const storageSaved = ref(false)
const isBrowsing = ref(false)

async function loadStorageConfig() {
  try {
    const data = await fetchSettings()
    activeStorage.value = data.activeStorage || 'LOCAL'
    localPath.value = data.localPath || ''
    savedLocalPath.value = data.localPath || ''
    isEditingPath.value = !data.localPath
  } catch (e) {
    console.warn('Failed to load storage config:', e)
  }
}

async function saveStorageConfig() {
  storageSaving.value = true
  try {
    const newPath = localPath.value.trim() || null
    const oldPath = savedLocalPath.value || null

    await updateSettings({
      activeStorage: activeStorage.value,
      localPath: newPath,
    })

    // 路径变更时触发文件搬迁
    if (oldPath && newPath && oldPath !== newPath) {
      try {
        await migrateFiles(oldPath, newPath)
      } catch (e) {
        console.warn('文件搬迁失败:', e)
      }
    }

    savedLocalPath.value = newPath || ''
    isEditingPath.value = false
    storageSaved.value = true
    setTimeout(() => { storageSaved.value = false }, 2000)
  } catch (e) {
    alert('保存失败: ' + (e.message || '网络异常'))
  } finally {
    storageSaving.value = false
  }
}

function startEditPath() {
  isEditingPath.value = true
}

async function openFolderPicker() {
  isBrowsing.value = true
  try {
    const data = await browseFolder()
    if (data.path) localPath.value = data.path
  } catch (e) {
    alert('无法打开文件夹选择器: ' + (e.message || '请手动输入路径'))
  } finally {
    isBrowsing.value = false
  }
}

// ─── Logs ────────────────────────────────────────────────────────────────
const logs = ref<any[]>([])
const showLogModal = ref(false)
const logModalRef = ref<HTMLElement | null>(null)
function openLogModal() { showLogModal.value = true; nextTick(() => { if (logModalRef.value) logModalRef.value.scrollTop = logModalRef.value.scrollHeight }) }
let logTimer: ReturnType<typeof setInterval> | null = null

async function refreshLogs() {
  try {
    logs.value = await getLogs()
  } catch {}
}

function clearAllLogs() {
  clearLogs().then(() => { logs.value = [] })
}

// ─── Init ────────────────────────────────────────────────────────────────
let pipelineTimer: ReturnType<typeof setInterval> | null = null
function stopPollTimers() {
  if (logTimer) { clearInterval(logTimer); logTimer = null }
  if (loopStatusTimer) { clearInterval(loopStatusTimer); loopStatusTimer = null }
  if (pipelineTimer) { clearInterval(pipelineTimer); pipelineTimer = null }
}
onActivated(() => {
  // timer 先建后加载：若等 await 完成再建，用户在加载期间离开页面时 onDeactivated 无 timer 可清，interval 永久泄漏
  stopPollTimers()
  logTimer = setInterval(refreshLogs, 3000)
  loopStatusTimer = setInterval(loadLoopStatus, 600000)  // 10分钟刷新一次
  pipelineTimer = setInterval(loadPipeline, 3000)
  Promise.all([
    loadStorageConfig(),
    loadLicense(),
    loadAiConfig(),
    refreshLogs(),
    loadNotifyConfig(),
    loadLoopStatus(),
    loadPipeline(),
    loadEmailAccounts(),
    loadEmailConfig(),
    loadAgentProfiles(),
    loadAvailableTools(),
  ]).catch(() => {})
})

onDeactivated(stopPollTimers)
onBeforeUnmount(stopPollTimers)

// ─── 我的档案（用户身份）─────────────────────────────────
const profName = ref('')
const profTitle = ref('')
const profCompany = ref('')
const profCity = ref('')
const profEmails = ref<string[]>([])
const profileSaving = ref(false)
const profileFacts = ref<ProfileFact[]>([])
const identityMeta = ref<{ updated_at?: string; source?: string }>({})

const profSummary = computed(() => {
  const parts = [profName.value, profTitle.value, profCompany.value, profCity.value].filter(Boolean)
  return parts.length ? parts.join(' · ') : ''
})
const profSourceText = computed(() => {
  if (identityMeta.value.updated_at) {
    const src = identityMeta.value.source === 'auto' ? '从你的发件邮件自动维护' : '手动维护'
    return `${src} · ${identityMeta.value.updated_at} 更新`
  }
  return '接入邮箱后将根据你的发件邮件自动建立，也可手动纠正'
})

async function loadProfile() {
  try {
    const p: any = await fetchUserProfile()
    profName.value = p?.name === '本地管理员' ? '' : (p?.name || '')
    profTitle.value = p?.title || ''
    profCompany.value = p?.company || ''
    profCity.value = p?.work_city || ''
    profEmails.value = p?.email_addresses || []
    identityMeta.value = p?.identity_meta || {}
    const f = await profileFactsList()
    profileFacts.value = f.facts || []
  } catch { /* 档案加载失败静默，保存时再报错 */ }
}
loadProfile()

// 档案编辑弹窗：身份信息 + 画像事实统一管理
const profileEditorOpen = ref(false)
const editName = ref('')
const editTitle = ref('')
const editCompany = ref('')
const editCity = ref('')
interface DraftFact { id: string; content: string }
const draftFacts = ref<DraftFact[]>([])

function openProfileEditor() {
  editName.value = profName.value
  editTitle.value = profTitle.value
  editCompany.value = profCompany.value
  editCity.value = profCity.value
  draftFacts.value = profileFacts.value.map(f => ({ id: f.id, content: f.content }))
  profileEditorOpen.value = true
}

function addDraftFact() { draftFacts.value.push({ id: '', content: '' }) }
function removeDraftFact(i: number) { draftFacts.value.splice(i, 1) }

async function saveProfileEditor() {
  if (profileSaving.value) return
  profileSaving.value = true
  try {
    await updateUserProfile({ name: editName.value.trim(), title: editTitle.value.trim(), company: editCompany.value.trim(), work_city: editCity.value.trim() })
    const oldIds = new Set(profileFacts.value.map(f => f.id))
    const kept = new Set<string>()
    for (const d of draftFacts.value) {
      const c = d.content.trim()
      if (!c) continue
      if (d.id && oldIds.has(d.id)) {
        kept.add(d.id)
        const orig = profileFacts.value.find(f => f.id === d.id)
        if (orig && orig.content !== c) {
          await profileFactDelete(d.id)
          await profileFactSave(c)
        }
      } else {
        await profileFactSave(c)
      }
    }
    for (const id of oldIds) {
      if (!kept.has(id)) await profileFactDelete(id)
    }
    toast({ message: '档案已保存', type: 'success' })
    profileEditorOpen.value = false
    await loadProfile()
  } catch (e: any) {
    toast({ message: '保存失败: ' + (e?.message || e), type: 'error' })
  } finally {
    profileSaving.value = false
  }
}

// ─── 截断文本悬浮提示 ─────────────────────────────────────
const { tooltipText, tooltipX, tooltipY, tooltipVisible, show: showTip, hide: hideTip } = useTooltip()
function onTruncEnter(e: MouseEvent, text: string) {
  const el = e.currentTarget as HTMLElement
  if (el.scrollWidth > el.clientWidth) showTip(el, text)
}
</script>

<template>
  <div class="h-full bg-white rounded-xl border border-surface-200 shadow-sm overflow-y-auto">
    <div class="max-w-[1200px] mx-auto p-6 space-y-4">

      <!-- ══════════ Row 0: 页面标题 ══════════ -->
      <header class="flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold tracking-tight text-surface-900">设置</h1>
          <p class="text-surface-400 text-[13px] mt-1">ArkMind v{{ appVersion }}</p>
        </div>
        <button @click="guideModalRef?.open()" class="btn-secondary text-xs"><AppIcon name="lightbulb" size="12"/>使用指南</button>
      </header>

      <!-- ══════════ Row 1: 状态概览卡片 ══════════ -->
      <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
        <!-- AI 模型 -->
        <div class="bg-white rounded-2xl border border-surface-200 p-3 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center gap-1.5 mb-1.5">
            <div class="w-6 h-6 rounded-lg bg-brand-50 flex items-center justify-center">
              <AppIcon name="sparkles" size="14" class="text-brand-600" />
            </div>
            <span class="text-[11px] text-surface-400">AI 模型</span>
          </div>
          <div class="text-[18px] font-bold text-surface-900">{{ aiProviders.length + visionProviders.length + (vecConfigured ? 1 : 0) }}<span class="text-[10px] font-normal text-surface-400 ml-1">个已配置</span></div>
        </div>
        <!-- AI洞察 -->
        <div class="bg-white rounded-2xl border border-surface-200 p-3 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center gap-1.5 mb-1.5">
            <div class="w-6 h-6 rounded-lg bg-emerald-50 flex items-center justify-center">
              <AppIcon name="bell" size="14" class="text-emerald-600" />
            </div>
            <span class="text-[11px] text-surface-400">AI洞察</span>
          </div>
          <div class="flex items-baseline justify-between">
            <div class="text-[18px] font-bold" :class="notifyEnabled ? 'text-emerald-600' : 'text-surface-300'">{{ notifyEnabled ? '已启用' : '已关闭' }}<span class="text-[10px] font-normal text-surface-400 ml-1">{{ notifyInterval === 0 ? '仅启动时' : notifyIntervalOptions.find(o => o.value === notifyInterval)?.label || '' }}</span></div>
            <span v-if="notifyEnabled && loopStatus.running && loopStatus.next_check_at" class="text-[10px] text-surface-400 shrink-0">下次 {{ loopStatus.next_check_at.slice(11) || loopStatus.next_check_at }}</span>
          </div>
        </div>
        <!-- 桌面精灵 -->
        <div class="bg-white rounded-2xl border border-surface-200 p-3 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center gap-1.5 mb-1.5">
            <div class="w-6 h-6 rounded-lg bg-indigo-50 flex items-center justify-center">
              <AppIcon name="chat-ai" size="14" class="text-indigo-600" />
            </div>
            <span class="text-[11px] text-surface-400">桌面精灵</span>
          </div>
          <div class="text-[18px] font-bold" :class="spriteEnabled ? 'text-indigo-600' : 'text-surface-300'">{{ spriteEnabled ? '已启用' : '已关闭' }}</div>
        </div>
        <!-- 知识管线 -->
        <div class="bg-white rounded-2xl border border-surface-200 p-3 shadow-sm hover:shadow-md transition-shadow">
          <div class="flex items-center gap-1.5 mb-1.5">
            <div class="w-6 h-6 rounded-lg bg-amber-50 flex items-center justify-center">
              <AppIcon name="server" size="14" class="text-amber-600" />
            </div>
            <span class="text-[11px] text-surface-400">知识管线</span>
          </div>
          <div class="text-[18px] font-bold text-surface-900">就绪</div>
        </div>
      </div>

      <!-- ══════════ 卡片区（Columns 瀑布流自动排列） ══════════ -->
      <div class="columns-1 lg:columns-2 gap-4">

          <!-- AI 模型配置 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <div class="px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">AI 模型配置</h2>
            </div>
            <div class="flex gap-0 border-b border-surface-100 px-5 mt-3">
              <button v-for="tab in ['llm','vision','vector','web']" :key="tab"
                @click="aiTab = tab"
                class="px-3 py-2 text-[11px] font-medium border-b-2 transition-colors -mb-px"
                :class="aiTab === tab ? 'border-brand-600 text-brand-700' : 'border-transparent text-surface-400 hover:text-surface-600'">
                {{ {llm:'语言模型',vision:'视觉模型',vector:'向量模型',web:'联网搜索'}[tab] }}
              </button>
            </div>

            <!-- LLM Tab -->
            <div v-show="aiTab === 'llm'" class="p-5 space-y-3">
              <p class="text-[10px] text-surface-400">选择默认的大语言模型，用于 AI 问答、知识库检索等任务</p>
              <div v-for="p in aiProviders" :key="p.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-colors" :class="activeProviderId === p.id ? 'border-brand-300 bg-brand-50' : 'border-surface-200 hover:border-surface-300'">
                <input type="radio" :value="p.id" :checked="pendingDefaultId === p.id" @change="setPendingDefault(p.id)" class="accent-brand-600 w-3.5 h-3.5 shrink-0" />
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] font-medium text-surface-700 truncate">{{ p.modelName }}</p>
                  <p class="text-[10px] text-surface-400 truncate font-mono">{{ p.baseUrl }}</p>
                </div>
                <button @click="testProviderConnection(p)" :disabled="testingProviderId" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-emerald-600" title="测试连接">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/></svg>
                </button>
                <button @click="openEditProvider(p)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="编辑"><AppIcon name="edit" size="12" /></button>
                <button @click="removeProvider(p)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="x" size="12" /></button>
              </div>
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-3">
                  <button v-if="pendingDefaultId !== activeProviderId" @click="saveDefaultConfig" :disabled="aiSaving" class="btn-primary text-xs"><AppIcon name="save" size="12"/>{{ aiSaving ? '保存中...' : '保存默认' }}</button>
                  <p v-if="aiTestResult" class="text-[10px]" :class="aiTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ aiTestResult }}</p>
                  <p v-if="aiSaved" class="text-[10px] text-emerald-600">已保存</p>
                </div>
                <button @click="openAddProvider" class="btn-primary text-xs"><AppIcon name="plus" size="12" /> 添加模型</button>
              </div>
            </div>

            <!-- Vision Tab -->
            <div v-show="aiTab === 'vision'" class="p-5 space-y-3">
              <p class="text-[10px] text-surface-400">用于图片/PDF 视觉解析的模型</p>
              <div v-for="p in visionProviders" :key="p.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-colors" :class="visionActiveId === p.id ? 'border-brand-300 bg-brand-50' : 'border-surface-200 hover:border-surface-300'">
                <input type="radio" :value="p.id" :checked="visionPendingId === p.id" @change="visionPendingId = p.id" class="accent-brand-600 w-3.5 h-3.5 shrink-0" />
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] font-medium text-surface-700 truncate">{{ p.modelName }}</p>
                  <p class="text-[10px] text-surface-400 truncate font-mono">{{ p.baseUrl }}</p>
                </div>
                <button @click="testVisionConnection(p)" :disabled="visionTesting" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-emerald-600" title="测试连接">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/></svg>
                </button>
                <button @click="openEditVision(p)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="编辑"><AppIcon name="edit" size="12" /></button>
                <button @click="removeVision(p)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="x" size="12" /></button>
              </div>
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-3">
                  <button v-if="visionPendingId !== visionActiveId" @click="saveVisionDefault" :disabled="visionSaving" class="btn-primary text-xs"><AppIcon name="save" size="12"/>{{ visionSaving ? '保存中...' : '保存默认' }}</button>
                  <p v-if="visionTestResult" class="text-[10px]" :class="visionTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ visionTestResult }}</p>
                  <p v-if="visionSaved" class="text-[10px] text-emerald-600">已保存</p>
                </div>
                <button @click="openAddVision" class="btn-primary text-xs"><AppIcon name="plus" size="12" /> 添加模型</button>
              </div>
            </div>

            <!-- Vector Tab -->
            <div v-show="aiTab === 'vector'" class="p-5 space-y-3">
              <p class="text-[10px] text-surface-400">用于文本向量化的 Embedding 模型</p>
              <div v-if="vecConfigured" class="flex items-center gap-3 px-3 py-2.5 rounded-lg border border-brand-300 bg-brand-50">
                <div class="w-7 h-7 rounded-full bg-brand-100 flex items-center justify-center shrink-0"><AppIcon name="sparkles" size="14" /></div>
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] font-medium text-surface-700 truncate">{{ vecSavedData.modelName }}</p>
                  <p class="text-[10px] text-surface-400 truncate font-mono">{{ vecSavedData.baseUrl }}</p>
                </div>
                <button @click="testVecConnection" :disabled="vecTesting" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-emerald-600" title="测试连接">
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/></svg>
                </button>
                <button @click="openVecModal" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="编辑"><AppIcon name="edit" size="12" /></button>
                <button @click="deleteVecConfig" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="x" size="12" /></button>
              </div>
              <div class="flex items-center justify-between">
                <div></div>
                <button v-if="!vecSavedData" @click="openVecModal" class="btn-primary text-xs"><AppIcon name="plus" size="12" /> 添加模型</button>
              </div>
              <p v-if="vecTestResult" class="text-[10px]" :class="vecTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ vecTestResult }}</p>
            </div>

            <!-- 联网搜索 Tab -->
            <div v-show="aiTab === 'web'" class="p-5 space-y-3">
              <p class="text-[10px] text-surface-400">接入博查搜索（<a href="https://bochaai.com" @click.prevent="openUrl('https://bochaai.com').catch(() => toast({ message: '打开网页失败', type: 'error' }))" class="text-blue-500 hover:underline cursor-pointer">bochaai.com</a> 注册获取 Key，新用户有免费额度）后，AI 问答与AI洞察可联网查证外部信息；未配置时 AI 不会感知该能力</p>
              <div>
                <label class="text-[11px] text-surface-500">博查 API Key</label>
                <input v-model="webSearchKey" type="password" autocomplete="off" placeholder="sk-..." class="mt-1 w-full px-2.5 py-1.5 text-[12px] font-mono border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
              </div>
              <div class="flex items-center justify-between">
                <p v-if="webSearchMsg" class="text-[10px] truncate flex-1 mr-2" :class="webSearchOk ? 'text-emerald-600' : 'text-red-500'">{{ webSearchMsg }}</p>
                <div v-else></div>
                <div class="flex items-center gap-2 shrink-0">
                  <button @click="testWebSearch" :disabled="!webSearchKey || webSearching" class="btn-secondary text-xs"><AppIcon name="play" size="12"/>{{ webSearching ? '测试中...' : '测试连接' }}</button>
                  <button @click="saveWebSearch" :disabled="webSearchSaving" class="btn-primary text-xs"><AppIcon name="save" size="12"/>{{ webSearchSaving ? '保存中...' : (webSearchKey ? '保存' : '清除配置') }}</button>
                </div>
              </div>
            </div>
          </div>

          <!-- 我的档案 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <div class="px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">我的档案</h2>
              <p class="text-[10px] text-surface-400 mt-1">智能问答与AI洞察依据此档案认识你</p>
            </div>
            <div class="p-5 pt-3 space-y-2.5">
              <p v-if="profSummary" class="text-[13px] font-medium text-surface-700">{{ profSummary }}</p>
              <p v-else class="text-[12px] text-surface-400">暂未识别出你的身份信息</p>
              <p class="text-[10px] text-surface-400">{{ profSourceText }}</p>
              <p class="text-[10px] text-surface-400 truncate">我的邮箱：<span v-if="!profEmails.length" class="text-surface-300">未配置，在「邮件账户」添加后自动识别</span><span v-else>{{ profEmails.join('、') }}</span></p>
              <div class="pt-1 border-t border-surface-100">
                <p class="text-[11px] font-medium text-surface-600 mt-2">画像事实 <span class="text-surface-400 font-normal">（{{ profileFacts.length }}/20）</span></p>
                <p v-if="!profileFacts.length" class="text-[10px] text-surface-400 mt-1">可在AI洞察里点「记入画像」沉淀，或在「编辑」中添加</p>
                <div v-else class="mt-1.5 space-y-1">
                  <div v-for="f in profileFacts.slice(0, 3)" :key="f.id" class="px-2.5 py-1.5 rounded-lg bg-surface-50 border border-surface-100">
                    <span class="block truncate text-[11px] text-surface-600 leading-snug cursor-default" @mouseenter="onTruncEnter($event, f.content)" @mouseleave="hideTip">{{ f.content }}</span>
                  </div>
                  <p v-if="profileFacts.length > 3" class="text-[10px] text-surface-400 pl-1">…共 {{ profileFacts.length }} 条，在「编辑」中查看全部</p>
                </div>
              </div>
              <div class="flex justify-end pt-1">
                <button @click="openProfileEditor" class="btn-primary text-xs"><AppIcon name="edit" size="12"/>编辑</button>
              </div>
            </div>
          </div>

          <!-- 档案编辑弹窗：身份信息 + 画像事实统一管理 -->
          <Teleport to="body">
            <Transition name="modal-fade">
              <div v-if="profileEditorOpen" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="profileEditorOpen = false">
                <div class="bg-white rounded-2xl shadow-2xl w-[520px] max-w-[90vw] max-h-[85vh] flex flex-col">
                  <div class="px-5 pt-4 pb-2 border-b border-surface-100">
                    <h3 class="text-[14px] font-semibold text-surface-800">编辑我的档案</h3>
                    <p class="text-[10px] text-surface-400 mt-0.5">姓名/职位/公司平时由 AI 从你的发件邮件自动维护；手动编辑后该字段以你写的为准</p>
                  </div>
                  <div class="px-5 py-3 space-y-3 overflow-y-auto">
                    <div class="grid grid-cols-2 gap-2">
                      <div>
                        <label class="text-[10px] text-surface-400">姓名</label>
                        <input v-model="editName" maxlength="30" placeholder="姓名" class="w-full px-2.5 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
                      </div>
                      <div>
                        <label class="text-[10px] text-surface-400">职位</label>
                        <input v-model="editTitle" maxlength="30" placeholder="职位" class="w-full px-2.5 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
                      </div>
                      <div>
                        <label class="text-[10px] text-surface-400">公司</label>
                        <input v-model="editCompany" maxlength="50" placeholder="公司" class="w-full px-2.5 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
                      </div>
                      <div>
                        <label class="text-[10px] text-surface-400">工作城市</label>
                        <input v-model="editCity" maxlength="20" placeholder="如：上海" class="w-full px-2.5 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
                      </div>
                    </div>
                    <div class="border-t border-surface-100 pt-3">
                      <div class="flex items-center justify-between">
                        <p class="text-[11px] font-medium text-surface-600">画像事实 <span class="text-surface-400 font-normal">（{{ draftFacts.length }}/20，一句一条，以"用户"为主语）</span></p>
                        <button @click="addDraftFact" :disabled="draftFacts.length >= 20" class="text-[11px] text-brand-600 hover:text-brand-700 disabled:opacity-40">+ 添加</button>
                      </div>
                      <div class="mt-2 space-y-1.5">
                        <div v-for="(f, i) in draftFacts" :key="i" class="flex items-center gap-1.5">
                          <input v-model="f.content" maxlength="500" placeholder="例：用户习惯每周三前交付方案" class="flex-1 px-2.5 py-1.5 text-[12px] border border-surface-200 rounded-lg focus:outline-none focus:border-brand-400" />
                          <button @click="removeDraftFact(i)" class="shrink-0 w-6 h-6 flex items-center justify-center rounded text-surface-300 hover:text-red-500" title="删除"><AppIcon name="x" size="12" /></button>
                        </div>
                        <p v-if="!draftFacts.length" class="text-[10px] text-surface-400">还没有画像事实。AI洞察里的「记入画像」会自动沉淀到这里，也可以直接添加。</p>
                      </div>
                    </div>
                  </div>
                  <div class="px-5 py-3 border-t border-surface-100 flex items-center justify-end gap-2">
                    <button @click="profileEditorOpen = false" class="btn-secondary text-xs">取消</button>
                    <button @click="saveProfileEditor" :disabled="profileSaving" class="btn-primary text-xs">>{{ profileSaving ? '保存中...' : '保存' }}</button>
                  </div>
                </div>
              </div>
            </Transition>
          </Teleport>

          <!-- 准则悬浮完整内容 tooltip -->
          <Teleport to="body">
            <div v-if="tooltipVisible"
                 class="fixed pointer-events-none z-[9999] px-2 py-1 text-[11px] text-white bg-gray-800 rounded shadow-lg max-w-[320px] whitespace-pre-wrap break-words"
                 :style="{ left: tooltipX + 'px', top: tooltipY + 'px', transform: 'translate(-50%, -100%)' }">
              {{ tooltipText }}
            </div>
          </Teleport>

          <!-- AI洞察 -->
          <div class="relative z-20 bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-visible break-inside-avoid mb-4">
            <div class="flex items-center justify-between px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">AI洞察</h2>
              <div v-if="notifyEnabled && notifyInterval !== 0" class="flex items-center gap-1.5">
                <span class="w-1.5 h-1.5 rounded-full" :class="loopStatus.running ? 'bg-emerald-500' : 'bg-surface-300'"></span>
                <span class="text-[10px]" :class="loopStatus.running ? 'text-emerald-600' : 'text-surface-400'">
                  {{ loopStatus.running ? '循环运行中' : '未启动' }}
                </span>
                <span v-if="loopStatus.running && loopStatus.next_check_at" class="text-[10px] text-surface-400">
                  · 下次: {{ loopStatus.next_check_at.slice(11) || loopStatus.next_check_at }}
                </span>
              </div>
            </div>
            <div class="p-5 space-y-4">
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">后台智能分析</p>
                  <p class="text-[10px] text-surface-400">AI 定期分析业务数据，主动提醒关注事项</p>
                </div>
                <label class="relative inline-flex items-center cursor-pointer">
                  <input type="checkbox" v-model="notifyEnabled" class="sr-only peer" @change="saveNotifyConfig">
                  <div class="w-9 h-5 bg-surface-200 peer-focus:ring-2 peer-focus:ring-brand-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-surface-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-brand-600"></div>
                </label>
              </div>
              <div class="flex items-center justify-between">
                <p class="text-[12px] font-medium text-surface-700">分析频率</p>
                <div class="relative" @focusout="(e: FocusEvent) => { if (!(e.currentTarget as HTMLElement)?.contains(e.relatedTarget as HTMLElement)) notifyIntervalOpen = false }">
                  <button @click="notifyIntervalOpen = !notifyIntervalOpen" class="flex items-center justify-between h-7 w-[110px] bg-white border border-surface-200 rounded-lg px-2.5 text-[11px] outline-none transition-colors text-surface-700 hover:border-surface-300">
                    <span>{{ notifyIntervalLabel }}</span>
                    <AppIcon name="chevron-down" class="w-3 h-3 text-surface-400 transition-transform" :class="notifyIntervalOpen ? 'rotate-180' : ''" />
                  </button>
                  <div v-if="notifyIntervalOpen" class="absolute top-full right-0 mt-1 z-50 w-[110px] bg-white rounded-lg shadow-lg border border-surface-100 py-1">
                    <button v-for="opt in notifyIntervalOptions" :key="opt.value" @click="notifyInterval = opt.value; notifyIntervalOpen = false; saveNotifyConfig()" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="notifyInterval === opt.value ? 'text-brand-600 bg-brand-50/50 font-medium' : 'text-surface-600 hover:bg-surface-50'">
                      {{ opt.label }}
                    </button>
                </div>
              </div>
              </div>
              <!-- 附加分析要求：单主体弹窗编辑；核心规则/观察池为行内只读查看 -->
              <div class="border-t border-surface-100 pt-3">
                <div class="flex items-center justify-between gap-2">
                  <div class="min-w-0">
                    <p class="text-[12px] font-medium text-surface-700">附加分析要求</p>
                    <p class="text-[10px] text-surface-400">
                      <span v-if="focusText">已添加 · 下次分析生效</span>
                      <span v-else>可选：给分析师添加定制要求，核心规则不变</span>
                      <button v-if="coreRulesText" @click="coreRulesOpen = !coreRulesOpen" class="ml-2 text-brand-600 hover:underline">{{ coreRulesOpen ? '收起核心规则' : '查看核心规则' }}</button>
                      <button v-if="watchText" @click="watchOpen = !watchOpen" class="ml-2 text-brand-600 hover:underline">{{ watchOpen ? '收起观察池' : '查看观察池' }}</button>
                    </p>
                  </div>
                  <div class="flex items-center gap-2 flex-shrink-0">
                    <button v-if="focusText" @click="clearFocusRequirements" :disabled="focusSaving" class="btn-secondary text-xs !text-amber-600 !border-amber-300 hover:!bg-amber-50"><AppIcon name="trash" size="12"/>清空</button>
                    <button @click="openFocusModal" class="btn-primary text-xs"><AppIcon name="edit" size="12"/>{{ focusText ? '编辑' : '添加要求' }}</button>
                  </div>
                </div>
                <pre v-if="coreRulesOpen" class="mt-2 w-full max-h-56 overflow-y-auto whitespace-pre-wrap text-[10px] leading-relaxed bg-surface-50 border border-surface-100 rounded-lg px-2.5 py-2 text-surface-500">{{ coreRulesText }}</pre>
                <pre v-if="watchOpen && watchText" class="mt-2 w-full max-h-40 overflow-y-auto whitespace-pre-wrap text-[11px] leading-relaxed bg-amber-50/50 border border-amber-100 rounded-lg px-2.5 py-2 text-surface-600">{{ watchText }}</pre>
              </div>
              <!-- 附加要求编辑弹窗（整体一段提示词） -->
              <Teleport to="body">
                <Transition name="modal-fade">
                  <div v-if="focusModalOpen" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="closeFocusModal">
                    <div class="bg-white rounded-2xl shadow-2xl w-[520px] max-w-[90vw] flex flex-col">
                      <div class="px-5 pt-4 pb-2 border-b border-surface-100">
                        <h3 class="text-[14px] font-semibold text-surface-800">附加分析要求</h3>
                        <p class="text-[10px] text-surface-400 mt-0.5">一段完整的定制提示词，整体注入分析师提示词；不能覆盖分析师核心硬规则</p>
                      </div>
                      <div class="px-5 py-3">
                        <textarea v-model="focusDraft" rows="8" :maxlength="FOCUS_MAX"
                          class="w-full text-[12px] leading-relaxed bg-surface-50 border border-surface-200 rounded-lg px-3 py-2.5 outline-none focus:border-brand-400 resize-y"
                          placeholder="例如：重点关注投标截止与大客户跟进；通知简短一些，一事一通知；普通文件上传不用提醒……"></textarea>
                        <div class="flex items-center justify-end mt-1">
                          <span class="text-[10px] text-surface-400">{{ focusDraft.length }}/{{ FOCUS_MAX }} 字</span>
                        </div>
                      </div>
                      <div class="px-5 py-3 border-t border-surface-100 flex items-center justify-end gap-2">
                        <button @click="closeFocusModal" class="btn-secondary text-xs">取消</button>
                        <button @click="saveFocusModal" :disabled="focusSaving" class="btn-primary text-xs">>{{ focusSaving ? '保存中...' : '保存' }}</button>
                      </div>
                    </div>
                  </div>
                </Transition>
              </Teleport>
            </div>
          </div>

          <!-- 运行日志 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <div class="px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">运行日志</h2>
            </div>
            <div class="p-5">
              <div class="flex items-center justify-between mb-3">
                <p class="text-[11px] text-surface-400">共 {{ logs.length }} 条日志</p>
                <button @click="openLogModal" class="btn-primary text-xs"><AppIcon name="eye" size="12"/>查看</button>
              </div>
            </div>
          </div>

          <!-- 桌面精灵 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <div class="flex items-center justify-between px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">桌面精灵</h2>
              <div class="flex items-center gap-1.5">
                <span class="text-[10px]" :class="spriteEnabled ? 'text-indigo-600' : 'text-surface-400'">{{ spriteEnabled ? '已启用' : '已关闭' }}</span>
              </div>
            </div>
            <div class="p-5 space-y-4">
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">启用桌面精灵</p>
                  <p class="text-[10px] text-surface-400">桌面上显示悬浮精灵图标，快速唤起 AI 对话</p>
                </div>
                <label class="relative inline-flex items-center cursor-pointer">
                  <input type="checkbox" v-model="spriteEnabled" class="sr-only peer" @change="toggleSprite">
                  <div class="w-9 h-5 bg-surface-200 peer-focus:ring-2 peer-focus:ring-brand-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-surface-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-brand-600"></div>
                </label>
              </div>
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">清空对话记录</p>
                  <p class="text-[10px] text-surface-400">删除精灵的所有聊天历史</p>
                </div>
                <button class="btn-secondary text-xs !text-red-500 !border-red-300 hover:!bg-red-50" @click="clearSpriteChat"><AppIcon name="trash" size="12"/>清空</button>
              </div>
            </div>
          </div>

          <!-- 知识管线管理 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <div class="flex items-center gap-2 px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">知识管线管理</h2>
              <span class="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded-full" :class="pipelineRunning ? 'bg-emerald-50 text-emerald-600' : 'bg-surface-50 text-surface-400'">
                <span class="w-1.5 h-1.5 rounded-full" :class="pipelineRunning ? 'bg-emerald-500 animate-pulse' : 'bg-surface-300'"></span>
                {{ pipelineRunning ? '运行中' : '空闲' }}
              </span>
            </div>
            <div class="p-5">
              <div class="space-y-3 mb-4" v-if="pipelineData">
                <div>
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-[11px] text-surface-500">物化进度</span>
                    <span class="text-[11px] font-mono text-surface-600">{{ pipelineData.knowledge?.children ?? 0 }} / {{ pipelineData.source?.total ?? 0 }}</span>
                  </div>
                  <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500" :style="{ width: (pipelineData.source?.total ? Math.min(100, (pipelineData.knowledge?.children ?? 0) / pipelineData.source.total * 100) : 0) + '%', background: 'linear-gradient(90deg, #6366f1, #818cf8)' }"></div>
                  </div>
                  <div class="text-[10px] text-surface-400 mt-0.5 text-right">{{ pipelineData.source?.total ? ((pipelineData.knowledge?.children ?? 0) / pipelineData.source.total * 100).toFixed(1) : 0 }}%</div>
                </div>
                <div>
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-[11px] text-surface-500">向量化进度</span>
                    <span class="text-[11px] font-mono text-surface-600">{{ pipelineData.knowledge?.vectorizedValid ?? pipelineData.knowledge?.vectorized ?? 0 }} / {{ pipelineData.knowledge?.childrenNonempty ?? pipelineData.knowledge?.children ?? 0 }}</span>
                  </div>
                  <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500" :style="{ width: ((pipelineData.knowledge?.childrenNonempty ?? pipelineData.knowledge?.children) ? Math.min(100, (pipelineData.knowledge?.vectorizedValid ?? pipelineData.knowledge?.vectorized ?? 0) / (pipelineData.knowledge?.childrenNonempty ?? pipelineData.knowledge?.children) * 100) : 0) + '%', background: 'linear-gradient(90deg, #06b6d4, #22d3ee)' }"></div>
                  </div>
                  <div class="flex items-center justify-between mt-0.5 text-[10px]">
                    <span :class="(pipelineData.knowledge?.embedPending ?? 0) > 0 ? 'text-amber-500 font-medium' : 'text-surface-400'">
                      {{ (pipelineData.knowledge?.embedPending ?? 0) > 0 ? `待向量化 ${pipelineData.knowledge.embedPending} 块` : '全部已向量化' }}
                    </span>
                    <span class="text-surface-400">{{ (pipelineData.knowledge?.childrenNonempty ?? pipelineData.knowledge?.children) ? (((pipelineData.knowledge?.vectorizedValid ?? pipelineData.knowledge?.vectorized ?? 0) / (pipelineData.knowledge?.childrenNonempty ?? pipelineData.knowledge?.children)) * 100).toFixed(1) : 0 }}%</span>
                  </div>
                  <div v-if="pipelineData.knowledge?.lastEmbedError" class="mt-1 text-[10px] text-red-500 truncate" :title="pipelineData.knowledge.lastEmbedError">
                    向量化错误：{{ pipelineData.knowledge.lastEmbedError }}
                  </div>
                </div>
                <div v-if="(pipelineData.fileParse?.total ?? 0) > 0">
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-[11px] text-surface-500">文件解析</span>
                    <span class="text-[11px] font-mono text-surface-600">
                      <span class="text-emerald-600">{{ pipelineData.fileParse?.done ?? 0 }}</span>
                      <span class="text-surface-300 mx-0.5">/</span>
                      <span>{{ pipelineData.fileParse?.total ?? 0 }}</span>
                      <button v-if="pipelineData.fileParse?.failed" @click="showParseErrorsModal = true" class="text-red-400 ml-1 hover:underline cursor-pointer" title="点击查看失败文件明细">({{ pipelineData.fileParse.failed }} 失败)</button>
                      <button v-if="pipelineData.fileParse?.unsupported" @click="showParseErrorsModal = true" class="text-surface-400 ml-1 hover:text-surface-600 hover:underline cursor-pointer" title="点击查看不支持的文件明细">({{ pipelineData.fileParse.unsupported }} 不支持)</button>
                    </span>
                  </div>
                  <div class="h-2 bg-surface-100 rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500" :style="{ width: ((pipelineData.fileParse?.total ? Math.min(100, (pipelineData.fileParse?.done ?? 0) / pipelineData.fileParse.total * 100) : 0) + '%'), background: 'linear-gradient(90deg, #f59e0b, #fbbf24)' }"></div>
                  </div>
                  <div class="flex items-center justify-between mt-1 text-[10px] text-surface-400">
                    <span>{{ pipelineData.fileParse?.processing ? '正在解析 ' + (pipelineData.fileParse.current?.name || '文件') + ' — 第 ' + (pipelineData.fileParse.current?.page ?? 0) + '/' + (pipelineData.fileParse.current?.total ?? '--') + ' 页' : (pipelineData.fileParse?.pending ? '等待解析中...' : '空闲') }}</span>
                    <span>{{ pipelineData.fileParse?.total ? ((pipelineData.fileParse?.done ?? 0) / pipelineData.fileParse.total * 100).toFixed(1) : 0 }}%</span>
                  </div>
                </div>
                <div v-if="pipelineData.vectorVacuum" class="flex items-center justify-between text-[10px] text-surface-400 border-t border-surface-100 pt-2">
                  <span>
                    向量回收：孤儿 {{ pipelineData.vectorVacuum.orphan_vectors ?? 0 }} 个
                    <template v-if="pipelineData.vectorVacuum.dry_run">（演练模式）</template>
                  </span>
                  <span>
                    {{ pipelineData.vectorVacuum.last_run ? '上次清理 ' + formatVacuumTime(pipelineData.vectorVacuum.last_run) : (pipelineData.vectorVacuum.last_check ? '上次检查 ' + formatVacuumTime(pipelineData.vectorVacuum.last_check) : '尚未检查') }}
                  </span>
                </div>
              </div>
              <p v-else class="text-[11px] text-surface-300 text-center py-4 mb-4">加载中...</p>
              <div class="flex flex-wrap gap-2 justify-end">
                <button v-if="((pipelineData?.fileParse?.failed ?? 0) > 0)" @click="handleRetryFailed" class="btn-secondary text-xs !text-red-600 !border-red-300 hover:!bg-red-50"><AppIcon name="refresh" size="12"/>重试失败文件（{{ pipelineData.fileParse.failed }}）</button>
                <button @click="handleRevectorize" class="btn-secondary text-xs !text-blue-600 !border-blue-300 hover:!bg-blue-50"><AppIcon name="refresh" size="12"/>重新向量化</button>
                <button @click="showAdvancedOps = !showAdvancedOps" class="btn-secondary text-xs flex items-center gap-1">高级操作<AppIcon name="chevron-down" size="10" class="transition-transform" :class="showAdvancedOps ? 'rotate-180' : ''" /></button>
              </div>
              <div v-if="showAdvancedOps" class="mt-2 rounded-lg border border-surface-200 bg-surface-50/60 px-3 py-2 space-y-2">
                <div class="flex items-center justify-between gap-3">
                  <div class="min-w-0">
                    <div class="text-[11px] text-surface-600">重置知识管线</div>
                    <div class="text-[9px] text-surface-400">知识碎片与源数据不一致时使用，将全量重新物化</div>
                  </div>
                  <button @click="handleClearKnowledge" class="btn-secondary text-xs !text-amber-600 !border-amber-300 hover:!bg-amber-50 flex-shrink-0"><AppIcon name="undo" size="12"/>重置</button>
                </div>
                <div class="flex items-center justify-between gap-3 border-t border-surface-100 pt-2">
                  <div class="min-w-0">
                    <div class="text-[11px] text-surface-600">重置全部数据</div>
                    <div class="text-[9px] text-surface-400">重新从业务数据抓取并物化，管线严重异常时的最后手段</div>
                  </div>
                  <button @click="handleClearAll" class="btn-secondary text-xs !text-red-600 !border-red-300 hover:!bg-red-50 flex-shrink-0"><AppIcon name="undo" size="12"/>重置</button>
                </div>
              </div>
              <!-- 解析问题明细弹窗 -->
              <Teleport to="body">
                <Transition name="modal-fade">
                  <div v-if="showParseErrorsModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="showParseErrorsModal = false">
                    <div class="bg-white rounded-2xl shadow-2xl w-[560px] max-w-[90vw] max-h-[70vh] flex flex-col">
                      <div class="px-5 pt-4 pb-3 border-b border-surface-100 flex items-center justify-between flex-shrink-0">
                        <h3 class="text-[14px] font-semibold text-surface-800">解析问题明细</h3>
                        <button @click="showParseErrorsModal = false" class="w-6 h-6 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition"><AppIcon name="x" size="13" /></button>
                      </div>
                      <div class="flex-1 overflow-y-auto px-5 py-3 space-y-4">
                        <div v-if="!pipelineData?.fileParse?.failedFiles?.length && !pipelineData?.fileParse?.unsupportedFiles?.length" class="text-[11px] text-surface-400 text-center py-6">暂无问题文件</div>
                        <div v-if="pipelineData?.fileParse?.failedFiles?.length">
                          <div class="text-[11px] font-medium text-red-500 mb-1.5">解析失败（{{ pipelineData.fileParse.failed }}）</div>
                          <div v-for="(f, i) in pipelineData.fileParse.failedFiles" :key="'f'+i" class="flex items-start gap-2 py-1 border-b border-surface-50 last:border-0">
                            <AppIcon name="alert-circle" size="12" class="text-red-400 flex-shrink-0 mt-0.5" />
                            <div class="flex-1 min-w-0">
                              <div class="text-[11px] text-surface-700 truncate">{{ f.name }}</div>
                              <div class="text-[10px] text-surface-400 break-all">{{ f.error || '解析失败' }}</div>
                            </div>
                            <span v-if="f.retries > 0" class="text-[9px] text-surface-400 flex-shrink-0 mt-1">已试{{ f.retries }}次</span>
                          </div>
                          <div v-if="pipelineData.fileParse.failed > pipelineData.fileParse.failedFiles.length" class="text-[10px] text-surface-400 mt-1">另有 {{ pipelineData.fileParse.failed - pipelineData.fileParse.failedFiles.length }} 个未列出</div>
                        </div>
                        <div v-if="pipelineData?.fileParse?.unsupportedFiles?.length">
                          <div class="text-[11px] font-medium text-surface-500 mb-1.5">不支持的格式（{{ pipelineData.fileParse.unsupported }}）</div>
                          <div v-for="(f, i) in pipelineData.fileParse.unsupportedFiles" :key="'u'+i" class="flex items-start gap-2 py-1 border-b border-surface-50 last:border-0">
                            <AppIcon name="file" size="12" class="text-surface-400 flex-shrink-0 mt-0.5" />
                            <div class="flex-1 min-w-0">
                              <div class="text-[11px] text-surface-600 truncate">{{ f.name }}</div>
                              <div class="text-[10px] text-surface-400 break-all">{{ (f.error || '').replace('__UNSUPPORTED__','') || '格式不支持' }}</div>
                            </div>
                          </div>
                          <div v-if="pipelineData.fileParse.unsupported > pipelineData.fileParse.unsupportedFiles.length" class="text-[10px] text-surface-400 mt-1">另有 {{ pipelineData.fileParse.unsupported - pipelineData.fileParse.unsupportedFiles.length }} 个未列出</div>
                        </div>
                      </div>
                      <div class="px-5 py-3 border-t border-surface-100 flex items-center justify-end gap-2 flex-shrink-0">
                        <button v-if="((pipelineData?.fileParse?.failed ?? 0) > 0)" @click="handleRetryFailed" class="btn-secondary text-xs !text-red-600 !border-red-300 hover:!bg-red-50">>重试失败任务</button>
                        <button @click="showParseErrorsModal = false" class="btn-secondary text-xs">关闭</button>
                      </div>
                    </div>
                  </div>
                </Transition>
              </Teleport>
            </div>
          </div>

          <!-- 邮件账户配置 -->
          <div class="relative z-20 bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-visible break-inside-avoid mb-4">
            <div class="px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">邮件账户</h2>
              <p class="text-[10px] text-surface-400 mt-1">配置 IMAP 邮箱账户，自动收取邮件并写入知识库</p>
            </div>
            <div class="p-5 space-y-3">
              <!-- 已配置账户列表 -->
              <div v-for="acc in emailAccounts" :key="acc.id" class="flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-colors" :class="acc.sync_enabled ? 'border-surface-200 hover:border-surface-300' : 'border-surface-100 bg-surface-50'">
                <div class="w-8 h-8 rounded-lg bg-brand-50 flex items-center justify-center flex-shrink-0">
                  <AppIcon name="envelope" size="16" class="text-brand-600" />
                </div>
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] font-medium text-surface-700 truncate">{{ acc.name }}</p>
                  <p class="text-[10px] text-surface-400 truncate font-mono">{{ acc.email_address }}</p>
                </div>
                <button @click="editEmailAccount(acc)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="编辑"><AppIcon name="edit" size="12" /></button>
                <button @click="removeEmailAccount(acc.id)" class="w-6 h-6 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="x" size="12" /></button>
              </div>
              <div v-if="emailAccounts.length === 0" class="text-[11px] text-surface-400 text-center py-3">
                未配置邮件账户
              </div>

              <div class="flex justify-end">
                <button @click="openAddEmail" class="btn-primary text-xs">
                  <AppIcon name="plus" size="12" stroke="2.5" /> 添加账户
                </button>
              </div>

            </div>
          </div>

          <!-- Agent 管理 -->
          <div class="relative z-20 bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-visible break-inside-avoid mb-4">
            <div class="px-5 pt-5 pb-0">
              <h2 class="text-[13px] font-semibold text-surface-700">Agent 管理</h2>
              <p class="text-[10px] text-surface-400 mt-1">创建和配置自定义 Agent，分配工具能力</p>
            </div>
            <div class="p-5 space-y-3">
              <!-- Agent 列表 -->
              <div v-for="agent in agentProfiles" :key="agent.id" class="flex items-center gap-2 px-2.5 py-1.5 rounded-lg border border-surface-200 hover:border-surface-300 transition-colors">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-1.5">
                    <span class="text-[12px] font-medium text-surface-700 truncate">{{ agent.name }}</span>
                    <span class="text-[9px] px-1 py-0.5 rounded bg-surface-50 text-surface-500 flex-shrink-0">工具{{ agent.tools?.length || 0 }}</span>
                    <span class="text-[9px] px-1 py-0.5 rounded bg-surface-50 text-surface-500 flex-shrink-0">迭代{{ agent.max_iterations }}</span>
                    <span v-if="agent.can_delegate" class="text-[9px] px-1 py-0.5 rounded bg-blue-50 text-blue-600 flex-shrink-0">可委托</span>
                    <span v-if="agent.is_builtin" class="text-[9px] px-1 py-0.5 rounded bg-emerald-50 text-emerald-600 flex-shrink-0">系统预设</span>
                  </div>
                  <p class="text-[10px] text-surface-400 truncate mt-0.5">{{ agent.description || '未填委托说明，主 Agent 不知何时该派活给它' }}</p>
                </div>
                <label class="relative inline-flex items-center cursor-pointer flex-shrink-0" :title="agent.status === 'active' ? '已启用' : '已禁用'">
                  <input type="checkbox" :checked="agent.status === 'active'" @change="toggleAgentStatus(agent)" class="sr-only peer">
                  <div class="w-8 h-[18px] bg-surface-200 peer-focus:ring-2 peer-focus:ring-brand-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-surface-300 after:border after:rounded-full after:h-3.5 after:w-3.5 after:transition-all peer-checked:bg-brand-600"></div>
                </label>
                <button v-if="!agent.is_builtin" @click="editAgent(agent)" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-brand-600" title="编辑"><AppIcon name="edit" size="11" /></button>
                <button v-if="!agent.is_builtin" @click="removeAgent(agent.id)" class="w-5 h-5 flex items-center justify-center rounded text-surface-400 hover:text-red-500" title="删除"><AppIcon name="x" size="11" /></button>
              </div>
              <div v-if="agentProfiles.length === 0" class="text-[11px] text-surface-400 text-center py-3">
                尚未创建 Agent
              </div>

              <div class="flex justify-end">
                <button @click="openAddAgent" class="btn-primary text-xs">
                  <AppIcon name="plus" size="12" stroke="2.5" /> 添加 Agent
                </button>
              </div>
            </div>
          </div>

          <!-- Agent 编辑弹窗 -->
          <Teleport to="body">
            <Transition name="modal-fade">
              <div v-if="showAgentModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center">
                <div class="bg-white rounded-2xl shadow-2xl w-[600px] max-w-[90vw] h-[85vh] flex flex-col">
                  <!-- 固定头部 -->
                  <div class="px-6 pt-5 pb-3 border-b border-surface-100 flex-shrink-0">
                    <h3 class="text-[14px] font-semibold text-surface-800">{{ editingAgentId ? '编辑 Agent' : '创建 Agent' }}</h3>
                  </div>
                  <!-- 可滚动内容区 -->
                  <div class="flex-1 overflow-y-auto px-6 py-4 space-y-4">
                    <!-- 名称 -->
                    <div>
                      <label class="block text-[10px] font-medium text-surface-400 mb-1">名称 *</label>
                      <input v-model="agentForm.name" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" placeholder="例如：方案专家" />
                    </div>
                    <!-- 委托说明（原"描述"：主 Agent 派活的路由依据） -->
                    <div>
                      <label class="block text-[10px] font-medium text-surface-400 mb-1">委托说明（主 Agent 靠它派活）*</label>
                      <p class="text-[9px] text-surface-400 mb-1 leading-relaxed">主 Agent 决定是否委托时，只能看到「名称 + 这段文字」，看不到它的提示词和工具列表——这是路由依据，不是简介</p>
                      <input v-model="agentForm.description" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" placeholder="写清负责什么、不负责什么，如：会议纪要与知识卡片的创建整理；不处理项目/待办类任务" />
                    </div>
                    <!-- 系统提示词 -->
                    <div>
                      <label class="block text-[10px] font-medium text-surface-400 mb-1">系统提示词 *</label>
                      <textarea v-model="agentForm.system_prompt" rows="8" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400 resize-y" placeholder="定义 Agent 的身份、目标和行为规则..."></textarea>
                    </div>
                    <!-- 最大迭代次数 + 允许委托 -->
                    <div class="flex items-center gap-4">
                      <div class="flex-1">
                        <label class="block text-[10px] font-medium text-surface-400 mb-1">最大迭代次数</label>
                        <input v-model.number="agentForm.max_iterations" type="number" min="1" max="50" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" />
                      </div>
                      <div class="flex items-center gap-2 pt-4">
                        <label class="relative inline-flex items-center cursor-pointer">
                          <input type="checkbox" v-model="agentForm.can_delegate" class="sr-only peer">
                          <div class="w-9 h-5 bg-surface-200 peer-focus:ring-2 peer-focus:ring-brand-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-surface-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-brand-600"></div>
                        </label>
                        <span class="text-[11px] text-surface-600">允许委托其他 Agent</span>
                      </div>
                    </div>
                    <!-- 工具分配 -->
                    <div>
                      <label class="block text-[10px] font-medium text-surface-400 mb-2">工具分配</label>
                      <div class="border border-surface-200 rounded-lg p-3 max-h-[240px] overflow-y-auto">
                        <div class="grid grid-cols-3 gap-1.5">
                          <div v-for="tool in availableTools" :key="tool.name"
                            class="flex flex-col p-2 rounded-lg border transition-colors relative select-none"
                            :class="[
                              (agentForm.presetTools || []).includes(tool.name) ? 'border-surface-200 bg-surface-50 opacity-60' :
                              (agentForm.tools || []).includes(tool.name) ? 'border-brand-400 bg-brand-50 cursor-pointer ring-1 ring-brand-200' : 'border-surface-200 bg-white cursor-pointer hover:border-brand-200'
                            ]"
                            @click="!(agentForm.presetTools || []).includes(tool.name) && toggleAgentTool(tool.name)">
                            <div class="flex items-start justify-between">
                              <span class="text-[11px] font-medium leading-tight" :class="(agentForm.tools || []).includes(tool.name) ? 'text-brand-700' : 'text-surface-700'">{{ toolLabels[tool.name]?.name || tool.name }}</span>
                              <span class="w-4 h-4 flex items-center justify-center rounded-full text-[9px] cursor-help text-surface-400 hover:text-surface-600 hover:bg-surface-100 flex-shrink-0 ml-1" :title="tool.description">?</span>
                            </div>
                            <span v-if="(agentForm.presetTools || []).includes(tool.name)" class="text-[8px] px-1 py-0.5 rounded bg-surface-100 text-surface-400 w-fit mt-0.5">预设</span>
                            <p class="text-[9px] mt-0.5 leading-relaxed line-clamp-2" :class="(agentForm.tools || []).includes(tool.name) ? 'text-brand-500' : 'text-surface-400'">{{ toolLabels[tool.name]?.brief || '' }}</p>
                            <span v-if="(agentForm.tools || []).includes(tool.name)" class="absolute bottom-1.5 right-1.5 w-4 h-4 rounded-full bg-brand-600 flex items-center justify-center">
                              <AppIcon name="check" size="10" class="text-white" />
                            </span>
                          </div>
                        </div>
                      </div>
                      <p class="text-[9px] text-surface-400 mt-1">已选择 {{ agentForm.tools.length }} 个工具</p>
                    </div>
                    <p v-if="agentFormError" class="text-[10px] text-red-500">{{ agentFormError }}</p>
                  </div>
                  <!-- 固定底部按钮 -->
                  <div class="px-6 py-3 border-t border-surface-100 flex items-center justify-end gap-2 flex-shrink-0">
                    <button @click="closeAgentModal" class="btn-secondary text-xs">取消</button>
                    <button @click="saveAgent" :disabled="agentSaving" class="btn-primary text-xs">
                      {{ agentSaving ? '保存中...' : '保存' }}
                    </button>
                  </div>
                </div>
              </div>
            </Transition>
          </Teleport>

          <!-- 数据管理 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 px-5 pt-5 pb-0">数据管理</h2>
            <div class="p-5 space-y-3">
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">导出数据</p>
                  <p class="text-[10px] text-surface-400">选择性备份业务域数据到本地</p>
                </div>
                <button class="btn-primary text-xs" :disabled="isExporting" @click="showDomainSelector = true"><AppIcon name="download" size="12"/>{{ isExporting ? '导出中...' : '导出数据' }}</button>
              </div>
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">导入数据</p>
                  <p class="text-[10px] text-surface-400">从 .ark 文件恢复业务数据</p>
                </div>
                <button class="btn-primary text-xs" :disabled="isImporting" @click="handleImportData"><AppIcon name="upload" size="12"/>{{ isImporting ? '导入中...' : '导入数据' }}</button>
              </div>
              <hr class="border-surface-100 my-1" />
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">一键备份</p>
                  <p class="text-[10px] text-surface-400">打包所有数据到 .zip 文件</p>
                </div>
                <button class="btn-secondary text-xs" :disabled="isBackingUp" @click="handleBackupAll"><AppIcon name="archive" size="12"/>{{ isBackingUp ? '备份中...' : '一键备份' }}</button>
              </div>
              <div class="flex items-center justify-between">
                <div>
                  <p class="text-[12px] font-medium text-surface-700">一键恢复</p>
                  <p class="text-[10px] text-surface-400">从 .zip 备份恢复（需重启）</p>
                </div>
                <button class="btn-secondary text-xs" :disabled="isRestoring" @click="handleRestoreAll"><AppIcon name="undo" size="12"/>{{ isRestoring ? '恢复中...' : '一键恢复' }}</button>
              </div>
              <!-- Domain Selector Modal -->
              <Teleport to="body"><Transition name="modal-fade"><div v-if="showDomainSelector" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm">
                <div class="bg-white rounded-xl shadow-xl w-[380px] max-h-[80vh] overflow-y-auto" @click.stop>
                  <div class="flex items-center px-5 py-3 border-b border-surface-100"><h3 class="text-sm font-semibold text-surface-800">选择导出范围</h3></div>
                  <div class="p-5 space-y-2">
                    <label v-for="d in selectedDomains" :key="d.key" class="flex items-center gap-2 cursor-pointer py-1">
                      <input type="checkbox" v-model="d.selected" class="w-3.5 h-3.5 rounded border-surface-300 text-brand-600" />
                      <span class="text-[11px] text-surface-600">{{ d.label }}</span>
                    </label>
                  </div>
                  <div class="flex items-center justify-end gap-2 px-5 py-3 border-t border-surface-100">
                    <button @click="showDomainSelector=false" class="btn-secondary text-xs">取消</button>
                    <button @click="handleExportData" class="btn-primary text-xs" :disabled="isExporting">>{{ isExporting ? '导出中...' : '确定导出' }}</button>
                  </div>
                </div>
              </div></Transition></Teleport>
            </div>
          </div>

          <!-- 许可证 -->
          <div class="bg-white rounded-2xl border border-surface-200 shadow-sm hover:shadow-md transition-shadow overflow-hidden break-inside-avoid mb-4">
            <h2 class="text-[13px] font-semibold text-surface-700 px-5 pt-5 pb-0">许可证</h2>
            <div class="p-5 space-y-3">
              <!-- License -->
              <div v-if="licenseActivated">
                <div class="flex items-center justify-between">
                  <div>
                    <p class="text-[12px] font-medium text-surface-700">{{ licenseProductLabel || '已激活' }} <span class="text-[10px] text-emerald-500 font-normal">● 已激活</span></p>
                    <p class="text-[10px] text-surface-400">{{ formatExpiryDate(licenseExpiresAt) ? '有效期至 ' + formatExpiryDate(licenseExpiresAt) : '永久许可' }}</p>
                  </div>
                  <button @click="licenseUpdating = true" class="btn-primary text-xs"><AppIcon name="refresh" size="12"/>更新</button>
                </div>
              </div>
              <div v-if="!licenseActivated">
                <div
                  v-if="trialActive"
                  class="bg-amber-50 border border-amber-100 rounded-lg p-3 mb-3"
                >
                  <p class="text-[11px] font-medium text-amber-700">
                    试用中（剩余 {{ trialRemainingDays }} 天）
                  </p>
                  <p class="text-[10px] text-amber-600/80 mt-0.5">试用结束后需激活许可证才能继续使用</p>
                </div>
                <div
                  v-else-if="trialExpired"
                  class="bg-red-50 border border-red-100 rounded-lg p-3 mb-3"
                >
                  <p class="text-[11px] font-medium text-red-700">试用已结束，请激活许可证</p>
                </div>
                <p class="text-[12px] font-medium text-surface-700 mb-2">激活许可证</p>
                <div class="bg-surface-50 rounded-lg p-3 mb-3">
                  <p class="text-[10px] text-surface-500 mb-2">机器码</p>
                  <div class="flex items-center gap-2">
                    <code class="flex-1 px-2.5 py-1.5 bg-white border border-surface-200 rounded-lg text-[11px] font-mono text-surface-700 select-all">{{ licenseMachineCode || '加载中...' }}</code>
                    <button @click="copyMachineCode" class="btn-secondary text-xs" v-if="licenseMachineCode"><AppIcon name="copy" size="12"/>{{ licenseCodeCopied ? '已复制' : '复制' }}</button>
                  </div>
                </div>
                <div class="flex items-center gap-2">
                  <input v-model="licenseKey" class="flex-1 border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="arkmind-xxxx" @keyup.enter="activateLicenseKey" />
                  <button class="btn-primary text-xs" :disabled="licenseActivating" @click="activateLicenseKey"><AppIcon name="check" size="12"/>{{ licenseActivating ? '激活中...' : '激活' }}</button>
                </div>
                <p v-if="licenseError" class="text-[10px] text-red-500 mt-2">{{ licenseError }}</p>
                <p v-if="licenseSuccess" class="text-[10px] text-emerald-600 mt-2">{{ licenseSuccess }}</p>
              </div>
            </div>
          </div>

      </div>

    </div>
  </div>

  <!-- ===== 模态框 ===== -->

  <!-- LLM Model Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showProviderModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center">
        <div class="bg-white rounded-2xl shadow-2xl w-[480px] max-w-[90vw] p-6">
          <h3 class="text-[14px] font-semibold text-surface-800 mb-4">{{ editingProvider ? '编辑模型' : '添加模型' }}</h3>
          <div class="space-y-3">
            <div class="relative" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) presetDropdownOpen = false }">
              <label class="block text-[10px] font-medium text-surface-400 mb-1">预设模型</label>
              <button @click="presetDropdownOpen = !presetDropdownOpen" class="w-full flex items-center justify-between h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none transition-colors" :class="selectedPreset ? 'text-slate-700' : 'text-slate-400'">
                <span>{{ selectedPreset || '自定义' }}</span>
                <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="presetDropdownOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="presetDropdownOpen" class="absolute top-full left-0 mt-1 z-50 w-full min-w-[200px] bg-white rounded-lg shadow-lg border border-slate-100 py-1 max-h-[200px] overflow-y-auto">
                <button @click="selectPreset('llm', { name: '', baseUrl: '', modelName: '', website: '' })" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedPreset === '' ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  自定义
                </button>
                <div class="border-t border-slate-100 my-1"></div>
                <button v-for="p in LLM_PRESETS" :key="p.name" @click="selectPreset('llm', p)" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedPreset === p.name ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  {{ p.name }}
                </button>
              </div>
            </div>
            <div v-if="selectedPreset">
              <a :href="LLM_PRESETS.find(p => p.name === selectedPreset)?.website ?? ''" @click.prevent="openUrl(LLM_PRESETS.find(p => p.name === selectedPreset)?.website ?? '').catch(() => toast({ message: '打开官网失败', type: 'error' }))" class="text-[10px] text-blue-500 hover:underline cursor-pointer">🌐 访问官网获取 API Key</a>
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">Base URL</label>
              <input v-model="providerForm.baseUrl" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="支持 Open AI 接口协议" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">API Key</label>
              <input v-model="providerForm.apiKey" type="password" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" :placeholder="editingProvider ? '留空则不修改' : 'sk-...'" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">模型名称</label>
              <input v-model="providerForm.modelName" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="gpt-4o-mini" />
            </div>
            <p v-if="providerFormError" class="text-[10px] text-red-500">{{ providerFormError }}</p>
          </div>
          <div class="flex items-center justify-between mt-4">
            <button @click="testProviderConnection()" :disabled="testingProviderId" class="btn-secondary text-xs">>{{ testingProviderId ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeProviderModal" class="btn-secondary text-xs">取消</button>
              <button @click="confirmProviderForm" :disabled="providerFormSaving" class="btn-primary text-xs">>{{ providerFormSaving ? '保存中...' : (editingProvider ? '保存' : '添加') }}</button>
            </div>
          </div>
          <p v-if="aiTestResult" class="text-[10px] mt-3" :class="aiTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ aiTestResult }}</p>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- Vision Model Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showVisionModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center">
        <div class="bg-white rounded-2xl shadow-2xl w-[520px] max-w-[90vw] p-6">
          <h3 class="text-[14px] font-semibold text-surface-800 mb-4">{{ editingVision ? '编辑 视觉模型' : '添加 视觉模型' }}</h3>
          <div class="space-y-3">
            <div class="relative" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) visionPresetDropdownOpen = false }">
              <label class="block text-[10px] font-medium text-surface-400 mb-1">预设模型</label>
              <button @click="visionPresetDropdownOpen = !visionPresetDropdownOpen" class="w-full flex items-center justify-between h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none transition-colors" :class="selectedVisionPreset ? 'text-slate-700' : 'text-slate-400'">
                <span>{{ selectedVisionPreset || '自定义' }}</span>
                <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="visionPresetDropdownOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="visionPresetDropdownOpen" class="absolute top-full left-0 mt-1 z-50 w-full min-w-[200px] bg-white rounded-lg shadow-lg border border-slate-100 py-1 max-h-[200px] overflow-y-auto">
                <button @click="selectPreset('vision', { name: '', baseUrl: '', modelName: '', website: '' })" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedVisionPreset === '' ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  自定义
                </button>
                <div class="border-t border-slate-100 my-1"></div>
                <button v-for="p in VISION_PRESETS" :key="p.name" @click="selectPreset('vision', p)" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedVisionPreset === p.name ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  {{ p.name }}
                </button>
              </div>
            </div>
            <div v-if="selectedVisionPreset">
              <a :href="VISION_PRESETS.find(p => p.name === selectedVisionPreset)?.website ?? ''" @click.prevent="openUrl(VISION_PRESETS.find(p => p.name === selectedVisionPreset)?.website ?? '').catch(() => toast({ message: '打开官网失败', type: 'error' }))" class="text-[10px] text-blue-500 hover:underline cursor-pointer">🌐 访问官网获取 API Key</a>
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">Base URL</label>
              <input v-model="visionForm.baseUrl" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="支持 Open AI 接口协议" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">API Key</label>
              <input v-model="visionForm.apiKey" type="password" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" :placeholder="editingVision ? '留空则不修改' : 'sk-...'" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">模型名称</label>
              <input v-model="visionForm.modelName" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" />
            </div>
            <p v-if="visionFormError" class="text-[10px] text-red-500">{{ visionFormError }}</p>
          </div>
          <div class="flex items-center justify-between mt-4">
            <button @click="testVisionConnection()" :disabled="visionTestingId" class="btn-secondary text-xs">>{{ visionTestingId ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeVisionModal" class="btn-secondary text-xs">取消</button>
              <button @click="confirmVisionForm" :disabled="visionFormSaving" class="btn-primary text-xs">>{{ visionFormSaving ? '保存中...' : (editingVision ? '保存' : '添加') }}</button>
            </div>
          </div>
          <p v-if="visionTestResult" class="text-[10px] mt-3" :class="visionTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ visionTestResult }}</p>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- Vector Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showVecModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center">
        <div class="bg-white rounded-2xl shadow-2xl w-[480px] max-w-[90vw] p-6">
          <h3 class="text-[14px] font-semibold text-surface-800 mb-4">向量模型配置</h3>
          <div class="space-y-3">
            <div class="relative" @focusout="(e) => { if (!e.currentTarget.contains(e.relatedTarget)) vecPresetDropdownOpen = false }">
              <label class="block text-[10px] font-medium text-surface-400 mb-1">预设模型</label>
              <button @click="vecPresetDropdownOpen = !vecPresetDropdownOpen" class="w-full flex items-center justify-between h-8 bg-white border border-slate-200 rounded-lg px-2.5 text-[11px] outline-none transition-colors" :class="selectedVecPreset ? 'text-slate-700' : 'text-slate-400'">
                <span>{{ selectedVecPreset || '自定义' }}</span>
                <AppIcon name="chevron-down" class="w-3 h-3 text-slate-400 transition-transform" :class="vecPresetDropdownOpen ? 'rotate-180' : ''" />
              </button>
              <div v-if="vecPresetDropdownOpen" class="absolute top-full left-0 mt-1 z-50 w-full min-w-[200px] bg-white rounded-lg shadow-lg border border-slate-100 py-1 max-h-[200px] overflow-y-auto">
                <button @click="selectPreset('vec', { name: '', baseUrl: '', modelName: '', website: '' })" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedVecPreset === '' ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  自定义
                </button>
                <div class="border-t border-slate-100 my-1"></div>
                <button v-for="p in VEC_PRESETS" :key="p.name" @click="selectPreset('vec', p)" class="w-full px-3 py-1.5 text-[11px] text-left transition-colors" :class="selectedVecPreset === p.name ? 'text-blue-600 bg-blue-50/50 font-medium' : 'text-slate-600 hover:bg-slate-50'">
                  {{ p.name }}
                </button>
              </div>
            </div>
            <div v-if="selectedVecPreset">
              <a :href="VEC_PRESETS.find(p => p.name === selectedVecPreset)?.website ?? ''" @click.prevent="openUrl(VEC_PRESETS.find(p => p.name === selectedVecPreset)?.website ?? '').catch(() => toast({ message: '打开官网失败', type: 'error' }))" class="text-[10px] text-blue-500 hover:underline cursor-pointer">🌐 访问官网获取 API Key</a>
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">Base URL</label>
              <input v-model="vecForm.baseUrl" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="支持 Open AI 接口协议" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">API Key</label>
              <input v-model="vecForm.apiKey" type="password" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" :placeholder="vecConfigured ? '留空则不修改' : 'sk-...'" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">模型名称</label>
              <input v-model="vecForm.modelName" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="text-embedding-3-small" />
            </div>
            <p v-if="vecFormError" class="text-[10px] text-red-500">{{ vecFormError }}</p>
          </div>
          <div class="flex items-center justify-between mt-4">
            <button @click="testVecConnection()" :disabled="vecTesting" class="btn-secondary text-xs">>{{ vecTesting ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeVecModal" class="btn-secondary text-xs">取消</button>
              <button @click="saveVecConfig" :disabled="vecSaving" class="btn-primary text-xs">>{{ vecSaving ? '保存中...' : '保存' }}</button>
            </div>
          </div>
          <p v-if="vecTestResult" class="text-[10px] mt-3" :class="vecTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ vecTestResult }}</p>
        </div>
      </div>
    </Transition>
  </Teleport>

  <!-- Log Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showLogModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="showLogModal = false">
        <div class="bg-white rounded-2xl shadow-2xl w-[720px] max-w-[90vw] h-[70vh] flex flex-col">
          <div class="flex items-center justify-between px-5 py-3 border-b border-surface-200">
            <h3 class="text-[13px] font-semibold text-surface-800">运行日志（{{ logs.length }} 条）</h3>
            <button @click="showLogModal = false" class="text-surface-400 hover:text-surface-600 transition-colors">
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path d="M6 18L18 6M6 6l12 12"/></svg>
            </button>
          </div>
          <div ref="logModalRef" class="flex-1 overflow-y-auto p-4 font-mono text-[10px] leading-relaxed">
            <div v-if="logs.length === 0" class="text-surface-400 text-center py-8">暂无日志</div>
            <div v-for="(log, i) in logs" :key="i" class="flex gap-2 mb-0.5 hover:bg-surface-50 px-1 rounded">
              <span class="text-surface-400 whitespace-nowrap">{{ log.timestamp }}</span>
              <span :class="{
                'text-red-500': log.level === 'ERROR',
                'text-amber-500': log.level === 'WARN',
                'text-blue-500': log.level === 'INFO',
                'text-surface-400': log.level === 'DEBUG' || log.level === 'TRACE'
              }" class="whitespace-nowrap font-medium">{{ log.level }}</span>
              <span class="text-surface-400 whitespace-nowrap">[{{ log.module }}]</span>
              <span class="text-surface-700 break-all">{{ log.message }}</span>
            </div>
          </div>
          <div class="flex items-center justify-between px-5 py-2 border-t border-surface-200">
            <p class="text-[10px] text-surface-400">每 3 秒自动刷新 · 重启后清空</p>
            <div class="flex gap-2">
              <button @click="clearAllLogs" class="btn-secondary text-xs !text-red-500 !border-red-300 hover:!bg-red-50">>清空</button>
              <button @click="refreshLogs" class="btn-secondary text-xs">>刷新</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
  <!-- Email Account Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showEmailForm" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center">
        <div class="bg-white rounded-2xl shadow-2xl w-[480px] max-w-[90vw] p-6">
          <h3 class="text-[14px] font-semibold text-surface-800 mb-4">{{ editingEmailId ? '编辑邮箱账户' : '添加邮箱账户' }}</h3>
          <div class="space-y-3">
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">账户名称</label>
              <input v-model="emailForm.name" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" placeholder="如：工作邮箱" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">邮箱地址</label>
              <input v-model="emailForm.emailAddress" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="user@example.com" />
            </div>
            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="block text-[10px] font-medium text-surface-400 mb-1">IMAP 服务器</label>
                <input v-model="emailForm.imapHost" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="imap.example.com" />
              </div>
              <div>
                <label class="block text-[10px] font-medium text-surface-400 mb-1">端口</label>
                <input v-model.number="emailForm.imapPort" type="number" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="993" />
              </div>
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">用户名</label>
              <input v-model="emailForm.username" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" placeholder="通常是邮箱地址" />
            </div>
            <div>
              <label class="block text-[10px] font-medium text-surface-400 mb-1">密码 / 授权码</label>
              <input v-model="emailForm.password" type="password" class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" :placeholder="editingEmailId ? '留空则不修改' : 'IMAP 授权码'" />
            </div>
          </div>
          <div class="flex items-center justify-end gap-2 mt-5">
            <button @click="showEmailForm = false; editingEmailId = null" class="btn-secondary text-xs">取消</button>
            <button @click="addEmailAccount" :disabled="emailSaving" class="btn-primary text-xs">>{{ emailSaving ? '保存中...' : (editingEmailId ? '保存' : '添加') }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
  <GuideModal ref="guideModalRef" />
</template>

<style scoped>
/* Provider card */
.provider-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  transition: border-color 0.15s, background 0.15s;
}
.provider-card.active {
  border-color: #93c5fd;
  background: #eff6ff;
}
.provider-card:hover {
  border-color: #cbd5e1;
}
.provider-main {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 1;
}
.provider-radio {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}
.provider-info {
  min-width: 0;
}
.provider-name {
  font-size: 13px;
  font-weight: 600;
  color: #334155;
  line-height: 1.3;
}
.provider-detail {
  font-size: 11px;
  color: #94a3b8;
  margin-top: 2px;
  display: flex;
  align-items: center;
  gap: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.provider-model {
  font-family: 'SF Mono', 'Fira Code', monospace;
  font-size: 10px;
}
.provider-sep {
  color: #cbd5e1;
}
.provider-url {
  font-family: 'SF Mono', 'Fira Code', monospace;
  font-size: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
}
.provider-key {
  margin-top: 3px;
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
}
.key-label {
  color: #94a3b8;
  font-weight: 500;
}
.provider-key code {
  font-family: 'SF Mono', 'Fira Code', monospace;
  color: #94a3b8;
  font-size: 10px;
}
.provider-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  margin-left: 8px;
}
.provider-action-btn {
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  color: #94a3b8;
  transition: color 0.15s, background 0.15s;
}
.provider-action-btn:hover {
  color: #475569;
  background: #f1f5f9;
}
.provider-action-del:hover {
  color: #ef4444;
  background: #fef2f2;
}
/* Modal */
.modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(0,0,0,0.3);
  display: flex;
  align-items: center;
  justify-content: center;
}
.modal-card {
  background: #fff;
  border-radius: 14px;
  padding: 20px 24px;
  width: 480px;
  max-width: 90vw;
  box-shadow: 0 25px 60px rgba(0,0,0,0.15);
}
.modal-title {
  font-size: 15px;
  font-weight: 600;
  color: #1d1d1f;
}
.modal-fade-enter-active,
.modal-fade-leave-active {
  transition: opacity 0.2s;
}
.modal-fade-enter-active .modal-card,
.modal-fade-leave-active .modal-card {
  transition: transform 0.2s;
}
.modal-fade-enter-from,
.modal-fade-leave-to {
  opacity: 0;
}
.modal-fade-enter-from .modal-card {
  transform: scale(0.95) translateY(10px);
}
.modal-fade-leave-to .modal-card {
  transform: scale(0.95) translateY(10px);
}
</style>
