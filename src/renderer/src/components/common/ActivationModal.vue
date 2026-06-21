<script setup lang="ts">
import { ref, onMounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { fetchLicenseStatus, activateLicense, getMachineCode } from '@/api/db-client'

const emit = defineEmits(['activated'])

const machineCode = ref('')
const key = ref('')
const activating = ref(false)
const error = ref('')
const success = ref('')
const codeCopied = ref(false)
const loading = ref(true)

onMounted(async () => {
  try {
    // 并行获取机器码和许可证状态
    const [code, status] = await Promise.all([
      getMachineCode(),
      fetchLicenseStatus(),
    ])
    machineCode.value = code
    // Only auto-dismiss if truly active (not expired)
    if (status.activated && status.status !== 'expired') {
      emit('activated')
      return
    }
    if (status.status === 'expired') {
      error.value = '许可证已过期，请重新激活'
    }
  } catch (e) {
    error.value = '无法获取机器码，请检查网络连接'
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
            placeholder="PMS-xxxxxxxxxxxxxxxx.xxxxxxxxxxxx"
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
    </div>
  </div>
</template>
