<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { alert, dangerConfirm } from '@/composables/useConfirm'
import { toast } from '@/composables/useToast'
import { useUploadProgress } from '@/composables/useUploadProgress'
import {
  fetchSettings, updateSettings, browseFolder,
  updateAiConfig, fetchAiConfig,
  migrateFiles, pipelineClearVectors,
  getLogs, clearLogs,
} from '@/api/db-client'

// ─── Vite define: 替换为 Cargo.toml 中的版本号
const appVersion = __APP_VERSION__
const aiTab = ref('llm')
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

async function loadLicense() {
  try {
    const { fetchLicenseStatus, getMachineCode } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    licenseActivated.value = status.activated
    licenseStatus.value = status.status || ''
    licenseProduct.value = status.product || ''
    licenseExpiresAt.value = status.expiresAt || ''
    licenseActivatedAt.value = status.activatedAt || ''
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
    const { save } = await import('@tauri-apps/plugin-dialog')
    const savePath = await save({
      defaultPath: 'arkmind_export.ark',
      filters: [{ name: 'ArkMind 数据包', extensions: ['ark'] }],
    })
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
    const { save } = await import('@tauri-apps/plugin-dialog')
    const savePath = await save({
      defaultPath: `arkmind-backup-${new Date().toISOString().slice(0,10)}.zip`,
      filters: [{ name: 'ZIP 文件', extensions: ['zip'] }],
    })
    if (!savePath) { isBackingUp.value = false; return }
    const { backupAll } = await import('@/api/db-client')
    const result = await backupAll(savePath)
    if (result.success) {
      toast.success(`备份已保存到：${savePath}`)
    } else {
      alert({ title: '备份失败', message: result.error || '未知错误' })
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
    const { restoreAll } = await import('@/api/db-client')
    const result = await restoreAll(filePath as string)
    if (result.success) {
      toast.success('数据已恢复，应用即将自动重启...')
    } else {
      alert({ title: '恢复失败', message: result.error || '未知错误' })
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
  aiProviders.value = aiProviders.value.filter(p => p.id !== provider.id)
  if (activeProviderId.value === provider.id) {
    activeProviderId.value = aiProviders.value[0]?.id || null
  }
  if (pendingDefaultId.value === provider.id) {
    pendingDefaultId.value = activeProviderId.value
  }
  // Auto-save
  updateAiConfig({
    providers: aiProviders.value,
    activeProviderId: activeProviderId.value,
  }).catch(() => {})
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
const visionForm = ref({ apiKey: '', baseUrl: 'https://api.openai.com/v1', modelName: '', apiFormat: 'openai', prompt: '从文档图片中提取并组织关键信息，以结构化 Markdown 格式输出。规则：1. 保留所有标题层级（# ## ###）；2. 每个标题后面必须换行，标题和正文之间空一行；3. 表格内容提炼为简洁的文本段落，每条规格独立一行；4. 不要输出原始表格格式；5. 保留所有数值、型号、技术参数；6. 每个特性用一句话概括' })
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
  visionForm.value = { apiKey: '', baseUrl: '', modelName: '', apiFormat: 'openai', prompt: '' }
  visionFormError.value = ''
  selectedVisionPreset.value = ''
  showVisionModal.value = true
}

function openEditVision(provider) {
  editingVision.value = provider
  visionForm.value = { apiKey: provider.apiKey, baseUrl: provider.baseUrl, modelName: provider.modelName, apiFormat: provider.apiFormat || 'openai', prompt: provider.prompt || '' }
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
        prompt: visionForm.value.prompt?.trim() || '',
      }
      if (formKey.trim() && !formKey.includes('●●●●')) { updated.apiKey = formKey.trim() }
      visionProviders.value[idx] = updated
    }
  } else {
    const newId = 'vp_' + Date.now().toString(36) + '_' + Math.random().toString(36).slice(2, 8)
    visionProviders.value.push({ id: newId, name: modelNameTrimmed, apiKey: formKey.trim(), baseUrl: baseUrl.trim() || 'https://api.openai.com/v1', modelName: modelNameTrimmed, apiFormat: visionForm.value.apiFormat || 'openai', prompt: visionForm.value.prompt?.trim() || '' })
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
  visionProviders.value = visionProviders.value.filter((p: any) => p.id !== provider.id)
  if (visionActiveId.value === provider.id) { visionActiveId.value = visionProviders.value[0]?.id || null }
  if (visionPendingId.value === provider.id) { visionPendingId.value = visionActiveId.value }
  await updateAiConfig({
    visionProviders: visionProviders.value,
    activeVisionId: visionActiveId.value,
  }).catch(() => {})
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
import { notificationConfigGet, notificationConfigUpdate, notificationLoopStatus, type LoopStatus } from '@/api/notification-client'
import { getSpriteEnabled, setSpriteEnabled } from '@/api/db-client'

const notifyEnabled = ref(true)
const notifyInterval = ref(60)
const loopStatus = ref<LoopStatus>({ success: true, running: false, last_check_at: '', next_check_at: '' })
let loopStatusTimer: ReturnType<typeof setInterval> | null = null

const intervalOptions = [
  { value: 0, label: '仅启动时' },
  { value: 30, label: '30分钟' },
  { value: 60, label: '1小时' },
  { value: 120, label: '2小时' },
  { value: 360, label: '6小时' },
  { value: 720, label: '12小时' },
  { value: 1440, label: '24小时' },
]

// ─── Sprite Settings ────────────────────────────────────────
const spriteEnabled = ref(true)

async function loadSpriteConfig() {
  try {
    spriteEnabled.value = await getSpriteEnabled()
  } catch {}
}

async function toggleSprite() {
  try {
    await setSpriteEnabled(spriteEnabled.value)
  } catch {}
}

async function loadNotifyConfig() {
  try {
    const r = await notificationConfigGet()
    if (r.config) {
      notifyEnabled.value = r.config.enabled !== '0'
      notifyInterval.value = r.config.startup_only === '1' ? 0 : (parseInt(r.config.interval_minutes) || 60)
    }
  } catch {}
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
  } catch {}
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
        apiKey: vecForm.value.apiKey.trim(),
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
onMounted(async () => {
  await Promise.all([
    loadStorageConfig(),
    loadLicense(),
    loadAiConfig(),
    refreshLogs(),
    loadNotifyConfig(),
    loadLoopStatus(),
  ])
  logTimer = setInterval(refreshLogs, 3000)
  loopStatusTimer = setInterval(loadLoopStatus, 600000)  // 10分钟刷新一次
})

onUnmounted(() => {
  if (logTimer) clearInterval(logTimer)
  if (loopStatusTimer) clearInterval(loopStatusTimer)
})
</script>

<template>
  <div class="max-w-3xl mx-auto py-8 px-4 space-y-6">

    <!-- AI 模型配置 ── Tab 切换 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm overflow-hidden">
      <div class="px-5 pt-5 pb-0">
        <h2 class="text-[13px] font-semibold text-surface-700">AI 模型配置</h2>
      </div>
      <div class="flex gap-0 border-b border-surface-100 px-5 mt-3">
        <button v-for="tab in ['llm','vision','vector']" :key="tab"
          @click="aiTab = tab"
          class="px-3 py-2 text-[11px] font-medium border-b-2 transition-colors -mb-px"
          :class="aiTab === tab ? 'border-brand-600 text-brand-700' : 'border-transparent text-surface-400 hover:text-surface-600'">
          {{ {llm:'语言模型',vision:'视觉模型',vector:'向量模型'}[tab] }}
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
            <button v-if="pendingDefaultId !== activeProviderId" @click="saveDefaultConfig" :disabled="aiSaving" class="inline-flex items-center gap-1 h-6 rounded-md px-2.5 text-[11px] font-medium bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-50 transition-colors">{{ aiSaving ? '保存中...' : '保存默认' }}</button>
            <p v-if="aiTestResult" class="text-[10px]" :class="aiTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ aiTestResult }}</p>
            <p v-if="aiSaved" class="text-[10px] text-emerald-600">已保存</p>
          </div>
          <button @click="openAddProvider" class="btn-primary text-[11px]"><AppIcon name="plus" size="12" /> 添加模型</button>
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
            <button v-if="visionPendingId !== visionActiveId" @click="saveVisionDefault" :disabled="visionSaving" class="inline-flex items-center gap-1 h-6 rounded-md px-2.5 text-[11px] font-medium bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-50 transition-colors">{{ visionSaving ? '保存中...' : '保存默认' }}</button>
            <p v-if="visionTestResult" class="text-[10px]" :class="visionTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ visionTestResult }}</p>
            <p v-if="visionSaved" class="text-[10px] text-emerald-600">已保存</p>
          </div>
          <button @click="openAddVision" class="btn-primary text-[11px]"><AppIcon name="plus" size="12" /> 添加模型</button>
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
          <button v-if="!vecSavedData" @click="openVecModal" class="btn-primary text-[11px]"><AppIcon name="plus" size="12" /> 添加模型</button>
        </div>
        <p v-if="vecTestResult" class="text-[10px]" :class="vecTestResult.includes('成功')?'text-emerald-600':'text-red-500'">{{ vecTestResult }}</p>
      </div>
    </div>

    <!-- 智能通知 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm overflow-hidden">
      <div class="flex items-center justify-between px-5 pt-5 pb-0">
        <h2 class="text-[13px] font-semibold text-surface-700">智能通知</h2>
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
          <div class="flex items-center gap-1.5 flex-wrap">
            <button v-for="opt in intervalOptions" :key="opt.value"
              @click="notifyInterval = opt.value; saveNotifyConfig()"
              class="px-2.5 py-1 rounded-md text-[11px] transition-all"
              :class="notifyInterval === opt.value
                ? 'bg-brand-600 text-white shadow-sm'
                : 'bg-surface-100 text-surface-600 hover:bg-surface-200'">
              {{ opt.label }}
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- 桌面精灵 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm overflow-hidden">
      <h2 class="text-[13px] font-semibold text-surface-700 px-5 pt-5 pb-0">桌面精灵</h2>
      <div class="p-5 space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[12px] font-medium text-surface-700">启动桌面精灵</p>
            <p class="text-[10px] text-surface-400">关闭后精灵隐藏，打开后立即显示</p>
          </div>
          <label class="relative inline-flex items-center cursor-pointer">
            <input type="checkbox" v-model="spriteEnabled" class="sr-only peer" @change="toggleSprite">
            <div class="w-9 h-5 bg-surface-200 peer-focus:ring-2 peer-focus:ring-brand-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-surface-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-brand-600"></div>
          </label>
        </div>
      </div>
    </div>

    <!-- 数据与许可证 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm overflow-hidden">
      <h2 class="text-[13px] font-semibold text-surface-700 px-5 pt-5 pb-0">数据与许可证</h2>
      <div class="p-5 space-y-4">
        <!-- Export -->
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[12px] font-medium text-surface-700">导出数据</p>
            <p class="text-[10px] text-surface-400">选择性备份业务域数据到本地</p>
          </div>
          <button class="btn-primary text-[11px]" :disabled="isExporting" @click="showDomainSelector = true">{{ isExporting ? '导出中...' : '导出数据' }}</button>
        </div>
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[12px] font-medium text-surface-700">导入数据</p>
            <p class="text-[10px] text-surface-400">从 .ark 文件恢复业务数据</p>
          </div>
          <button class="btn-primary text-[11px]" :disabled="isImporting" @click="handleImportData">{{ isImporting ? '导入中...' : '导入数据' }}</button>
        </div>
        <hr class="border-surface-100" />
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[12px] font-medium text-surface-700">一键备份</p>
            <p class="text-[10px] text-surface-400">打包所有数据（数据库、文件、向量库）到单个 .zip 文件</p>
          </div>
          <button class="btn-secondary text-[11px]" :disabled="isBackingUp" @click="handleBackupAll">{{ isBackingUp ? '备份中...' : '一键备份' }}</button>
        </div>
        <div class="flex items-center justify-between">
          <div>
            <p class="text-[12px] font-medium text-surface-700">一键恢复</p>
            <p class="text-[10px] text-surface-400">从 .zip 备份文件恢复所有数据（需重启应用）</p>
          </div>
          <button class="btn-secondary text-[11px]" :disabled="isRestoring" @click="handleRestoreAll">{{ isRestoring ? '恢复中...' : '一键恢复' }}</button>
        </div>
        <!-- Domain Selector Modal -->
        <Teleport to="body"><Transition name="modal-fade"><div v-if="showDomainSelector" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" @click.self="showDomainSelector=false">
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
              <button @click="handleExportData" class="btn-primary text-xs" :disabled="isExporting">{{ isExporting ? '导出中...' : '确定导出' }}</button>
            </div>
          </div>
        </div></Transition></Teleport>

        <hr class="border-surface-100" />

        <!-- License -->
        <div v-if="licenseActivated">
          <div class="flex items-center justify-between">
            <div>
              <p class="text-[12px] font-medium text-surface-700">{{ licenseProductLabel || '已激活' }} <span class="text-[10px] text-emerald-500 font-normal">● 已激活</span></p>
              <p class="text-[10px] text-surface-400">{{ formatExpiryDate(licenseExpiresAt) ? '有效期至 ' + formatExpiryDate(licenseExpiresAt) : '永久许可' }}</p>
            </div>
            <button @click="licenseUpdating = true" class="btn-primary text-[11px]">更新</button>
          </div>
        </div>
        <!-- License Update Modal -->
        <Teleport to="body"><Transition name="modal-fade"><div v-if="licenseUpdating" class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" @click.self="licenseUpdating=false">
          <div class="bg-white rounded-xl shadow-xl w-[400px]" @click.stop>
            <div class="flex items-center px-5 py-3 border-b border-surface-100"><h3 class="text-sm font-semibold text-surface-800">更新许可证</h3></div>
            <div class="p-5 space-y-3">
              <div>
                <p class="text-[10px] text-surface-500 mb-1.5">机器码</p>
                <div class="flex items-center gap-2">
                  <code class="flex-1 px-3 py-2 bg-surface-50 border border-surface-200 rounded-lg text-[11px] font-mono text-surface-700 select-all break-all">{{ licenseMachineCode }}</code>
                  <button @click="copyMachineCode" class="px-2.5 py-2 text-[11px] rounded-lg border border-surface-200 text-surface-500 hover:bg-surface-100 whitespace-nowrap">{{ licenseCodeCopied ? '已复制' : '复制' }}</button>
                </div>
              </div>
              <div>
                <p class="text-[10px] text-surface-500 mb-1.5">许可证密钥</p>
                <input v-model="licenseKey" class="w-full border border-surface-200 rounded-lg px-3 py-2 text-[12px] font-mono outline-none focus:border-brand-400" placeholder="输入许可证密钥" @keyup.enter="activateLicenseKey" />
              </div>
              <p v-if="licenseError" class="text-[10px] text-red-500">{{ licenseError }}</p>
              <p v-if="licenseSuccess" class="text-[10px] text-emerald-600">{{ licenseSuccess }}</p>
            </div>
            <div class="flex items-center justify-end gap-2 px-5 py-3 border-t border-surface-100">
              <button @click="licenseUpdating=false" class="btn-secondary text-xs">取消</button>
              <button @click="activateLicenseKey" class="btn-primary text-xs" :disabled="licenseActivating">{{ licenseActivating ? '激活中...' : '激活' }}</button>
            </div>
          </div>
        </div></Transition></Teleport>
        <div v-if="!licenseActivated">
          <p class="text-[12px] font-medium text-surface-700 mb-2">激活许可证</p>
          <div class="bg-surface-50 rounded-lg p-3 mb-3">
            <p class="text-[10px] text-surface-500 mb-2">机器码（提供给管理员获取秘钥）</p>
            <div class="flex items-center gap-2">
              <code class="flex-1 px-2.5 py-1.5 bg-white border border-surface-200 rounded-lg text-[11px] font-mono text-surface-700 select-all">{{ licenseMachineCode || '加载中...' }}</code>
              <button @click="copyMachineCode" class="px-2.5 py-1.5 text-[11px] rounded-lg border border-surface-200 text-surface-500 hover:bg-surface-100" v-if="licenseMachineCode">{{ licenseCodeCopied ? '已复制' : '复制' }}</button>
            </div>
          </div>
          <div class="flex items-center gap-2">
            <input v-model="licenseKey" class="flex-1 border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none focus:border-brand-400" placeholder="PMS-xxxx" @keyup.enter="activateLicenseKey" />
            <button class="px-3 py-1.5 text-[11px] rounded-lg bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-50" :disabled="licenseActivating" @click="activateLicenseKey">{{ licenseActivating ? '激活中...' : '激活' }}</button>
          </div>
          <p v-if="licenseError" class="text-[10px] text-red-500 mt-2">{{ licenseError }}</p>
          <p v-if="licenseSuccess" class="text-[10px] text-emerald-600 mt-2">{{ licenseSuccess }}</p>
        </div>
      </div>
    </div>


    <!-- 日志 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm p-5">
      <div class="flex items-center justify-between mb-3">
        <h2 class="text-[13px] font-semibold text-surface-700">运行日志</h2>
        <button @click="openLogModal" class="h-6 px-3 text-[11px] rounded-md bg-brand-600 text-white hover:bg-brand-700 disabled:opacity-50 transition-colors">查看</button>
      </div>
      <p class="text-[11px] text-surface-400">共 {{ logs.length }} 条日志 · 每 3 秒自动刷新</p>
    </div>

    <!-- 关于 -->
    <div class="bg-white rounded-2xl border border-surface-200 shadow-sm p-5">
      <h2 class="text-[13px] font-semibold text-surface-700 mb-2">关于</h2>
      <p class="text-[11px] text-surface-400">ArkMind v{{ appVersion }} · Vue 3 + Tauri</p>
    </div>
  </div>

  <!-- ===== 模态框（保持原逻辑不变，只改样式） ===== -->

  <!-- LLM Model Modal -->
  <Teleport to="body">
    <Transition name="modal-fade">
      <div v-if="showProviderModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="closeProviderModal">
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
              <a :href="LLM_PRESETS.find(p => p.name === selectedPreset)?.website" target="_blank" class="text-[10px] text-blue-500 hover:underline">🌐 访问官网获取 API Key</a>
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
            <button @click="testProviderConnection()" :disabled="testingProviderId" class="btn-secondary text-xs">{{ testingProviderId ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeProviderModal" class="btn-secondary text-xs">取消</button>
              <button @click="confirmProviderForm" :disabled="providerFormSaving" class="btn-primary text-xs">{{ providerFormSaving ? '保存中...' : (editingProvider ? '保存' : '添加') }}</button>
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
      <div v-if="showVisionModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="closeVisionModal">
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
              <a :href="VISION_PRESETS.find(p => p.name === selectedVisionPreset)?.website" target="_blank" class="text-[10px] text-blue-500 hover:underline">🌐 访问官网获取 API Key</a>
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
            <button @click="testVisionConnection()" :disabled="visionTestingId" class="btn-secondary text-xs">{{ visionTestingId ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeVisionModal" class="btn-secondary text-xs">取消</button>
              <button @click="confirmVisionForm" :disabled="visionFormSaving" class="btn-primary text-xs">{{ visionFormSaving ? '保存中...' : (editingVision ? '保存' : '添加') }}</button>
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
      <div v-if="showVecModal" class="fixed inset-0 z-[100] bg-black/30 flex items-center justify-center" @click.self="closeVecModal">
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
              <a :href="VEC_PRESETS.find(p => p.name === selectedVecPreset)?.website" target="_blank" class="text-[10px] text-blue-500 hover:underline">🌐 访问官网获取 API Key</a>
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
            <button @click="testVecConnection()" :disabled="vecTesting" class="btn-secondary text-xs">{{ vecTesting ? '测试中...' : '测试连接' }}</button>
            <div class="flex items-center gap-2">
              <button @click="closeVecModal" class="btn-secondary text-xs">取消</button>
              <button @click="saveVecConfig" :disabled="vecSaving" class="btn-primary text-xs">{{ vecSaving ? '保存中...' : '保存' }}</button>
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
              <button @click="clearAllLogs" class="px-3 py-1 text-[11px] text-red-500 hover:text-red-600 border border-surface-200 rounded-lg hover:bg-red-50 transition-colors">清空</button>
              <button @click="refreshLogs" class="px-3 py-1 text-[11px] text-surface-500 hover:text-surface-700 border border-surface-200 rounded-lg hover:bg-surface-50 transition-colors">刷新</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
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
