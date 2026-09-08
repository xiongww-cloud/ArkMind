<script setup lang="ts">
import { ref, onMounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { fetchLicenseStatus, activateLicense, getMachineCode } from '@/api/db-client'

const props = defineProps<{
  /** 试用提醒模式：可关闭，显示剩余天数 */
  trialReminder?: boolean
  /** 试用剩余天数（trialReminder 模式下使用） */
  trialRemainingDays?: number
}>()

const emit = defineEmits(['activated', 'close'])

const machineCode = ref('')
const key = ref('')
const activating = ref(false)
const error = ref('')
const success = ref('')
const codeCopied = ref(false)
const loading = ref(true)
/** 试用提醒模式下点击"去激活"后切换到激活表单 */
const showActivationForm = ref(false)

async function fetchLicenseWithRetry(maxRetries = 3, delay = 500) {
  for (let i = 0; i < maxRetries; i++) {
    try {
      return await fetchLicenseStatus()
    } catch (e) {
      if (i < maxRetries - 1) await new Promise(r => setTimeout(r, delay))
      else throw e
    }
  }
}

onMounted(async () => {
  try {
    machineCode.value = await getMachineCode()
  } catch (e) {
    console.error('获取机器码失败:', e)
    error.value = '无法获取机器码，请重启应用后重试'
  }

  // 试用提醒模式：只显示提醒，不自动关闭
  if (props.trialReminder) {
    loading.value = false
    return
  }

  try {
    const status = await fetchLicenseWithRetry()
    if (status.activated && status.status !== 'expired') {
      emit('activated')
      return
    }
    if (status.status === 'expired') {
      error.value = '许可证已过期，请重新激活'
    } else if (status.trial?.expired) {
      error.value = '试用期已结束，请输入许可证秘钥激活'
    }
  } catch (e) {
    console.error('获取许可证状态失败:', e)
    if (!error.value) error.value = '许可证状态查询失败，请重启应用后重试'
  } finally {
    loading.value = false
  }
})

async function copyMachineCode() {
  if (!machineCode.value) return
  try {
    await navigator.clipboard.writeText(machineCode.value)
    codeCopied.value = true
    setTimeout(() => { codeCopied.value = false }, 2000)
  } catch { /* ignore */ }
}

async function handleActivate() {
  const trimmed = key.value.trim()
  if (!trimmed) {
    error.value = '请输入激活秘钥'
    return
  }
  activating.value = true
  error.value = ''
  success.value = ''
  try {
    const result = await activateLicense(trimmed)
    if (result.success) {
      success.value = result.message || '激活成功'
      setTimeout(() => { emit('activated') }, 800)
    } else {
      error.value = result.error || '激活失败'
    }
  } catch (e) {
    error.value = e.response?.data?.error || e.message || '激活失败'
  } finally {
    activating.value = false
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[9999] flex items-center justify-center bg-slate-900/60 backdrop-blur-sm">
    <div class="bg-white rounded-2xl shadow-2xl w-full max-w-md mx-4 p-8">
      <!-- ═══ 试用提醒模式（轻量、可关闭） ═══ -->
      <template v-if="trialReminder && !showActivationForm">
        <div class="text-center mb-6">
          <div class="inline-flex items-center justify-center w-12 h-12 rounded-xl bg-amber-50 mb-3">
            <AppIcon name="clock" class="w-6 h-6 text-amber-600" />
          </div>
          <h1 class="text-lg font-bold text-slate-800">试用期提醒</h1>
          <p class="text-xs text-slate-400 mt-1">您正在试用 ArkMind</p>
        </div>

        <div class="bg-amber-50 border border-amber-100 rounded-xl p-5 text-center mb-6">
          <p class="text-3xl font-bold text-amber-600">{{ trialRemainingDays || 0 }}</p>
          <p class="text-xs text-amber-600 mt-1">天后试用期结束</p>
          <p class="text-[11px] text-slate-400 mt-3">试用结束后需激活许可证才能继续使用，请提前联系管理员获取秘钥。</p>
        </div>

        <div class="flex items-center gap-2">
          <button
            @click="emit('close')"
            class="flex-1 py-2.5 rounded-lg text-sm font-medium text-slate-600 bg-slate-100 hover:bg-slate-200 transition-colors"
          >
            知道了
          </button>
          <button
            @click="showActivationForm = true"
            class="flex-1 py-2.5 rounded-lg text-sm font-semibold text-white bg-blue-600 hover:bg-blue-700 transition-colors"
          >
            去激活
          </button>
        </div>
      </template>

      <!-- ═══ 激活表单模式（强制激活，不可关闭） ═══ -->
      <template v-else>
        <!-- Header -->
        <div class="text-center mb-6">
          <div class="inline-flex items-center justify-center w-12 h-12 rounded-xl bg-blue-50 mb-3">
            <AppIcon name="lock" class="w-6 h-6 text-blue-600" />
          </div>
          <h1 class="text-lg font-bold text-slate-800">软件激活</h1>
          <p class="text-xs text-slate-400 mt-1">输入许可证秘钥以激活系统</p>
        </div>

        <!-- Loading -->
        <div v-if="loading" class="flex items-center justify-center py-8">
          <svg class="animate-spin h-6 w-6 text-blue-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
            <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
          </svg>
        </div>

        <template v-else>
          <!-- Machine Code -->
          <div class="bg-slate-50 rounded-xl p-4 mb-4">
            <label class="block text-xs font-medium text-slate-500 mb-2">本机机器码</label>
            <div class="flex items-stretch gap-2">
              <code class="flex-1 px-3 py-2.5 bg-white border border-slate-200 rounded-lg text-sm font-mono text-slate-700 select-all">{{ machineCode }}</code>
              <button
                @click="copyMachineCode"
                class="flex items-center gap-1 px-3 py-2.5 border border-slate-200 bg-white rounded-lg text-xs text-slate-500 hover:text-blue-600 hover:border-blue-300 transition-colors flex-shrink-0"
              >
                <AppIcon name="clipboard" size="13" stroke="2" />
                <span>{{ codeCopied ? '已复制' : '复制' }}</span>
              </button>
            </div>
            <p class="text-[11px] text-slate-400 mt-2">将此机器码发送给管理员以获取激活秘钥</p>
          </div>

          <!-- Key Input -->
          <div class="mb-4">
            <label class="block text-xs font-medium text-slate-500 mb-2">激活秘钥</label>
            <input
              v-model="key"
              type="text"
              class="w-full px-3 py-2.5 border border-slate-200 rounded-lg text-sm font-mono text-slate-700 focus:border-blue-500 focus:ring-3 focus:ring-blue-50 outline-none transition-all"
              placeholder="Arkmind-xxxxxxxxxxxxxxxx.xxxxxxxxxxxx"
              @keyup.enter="handleActivate"
            />
          </div>

          <!-- Error / Success -->
          <p v-if="error" class="text-xs text-red-500 mb-3">{{ error }}</p>
          <p v-if="success" class="text-xs text-emerald-600 mb-3">{{ success }}</p>

          <!-- Activate -->
          <button
            @click="handleActivate"
            :disabled="activating || !machineCode"
            class="w-full py-2.5 rounded-lg text-sm font-semibold transition-all"
            :class="activating || !machineCode ? 'bg-slate-200 text-slate-400 cursor-not-allowed' : 'bg-blue-600 text-white hover:bg-blue-700'"
          >
            {{ activating ? '验证中...' : '激活' }}
          </button>
        </template>
      </template>
    </div>
  </div>
</template>
