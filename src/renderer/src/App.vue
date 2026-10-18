<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import AppHeader from '@/components/layout/Header.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import Toast from '@/components/common/Toast.vue'
import UploadProgress from '@/components/common/UploadProgress.vue'
import ActivationModal from '@/components/common/ActivationModal.vue'
import GuideModal from '@/components/common/GuideModal.vue'
import WhatsNewModal from '@/components/common/WhatsNewModal.vue'
import UpdateAvailableModal from '@/components/common/UpdateAvailableModal.vue'
import { useAuthStore } from '@/stores/auth'
import { invoke } from '@tauri-apps/api/core'

const sidebarCollapsed = ref(false)
const authStore = useAuthStore()
const showActivation = ref(false)
const showTrialReminder = ref(false)
const trialRemainingDays = ref(0)
const guideModalRef = ref<InstanceType<typeof GuideModal> | null>(null)
const whatsNewModalRef = ref<InstanceType<typeof WhatsNewModal> | null>(null)
const updateModalRef = ref<InstanceType<typeof UpdateAvailableModal> | null>(null)
let licensePollTimer = null

function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value
}

const TRIAL_REMINDER_DAYS_KEY = 'trial_reminder_shown_date'

function todayStr() {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/** 试用提醒每天最多弹一次 */
function shouldShowTrialReminder(remainingDays: number): boolean {
  if (remainingDays > 7 || remainingDays <= 0) return false
  try {
    const last = localStorage.getItem(TRIAL_REMINDER_DAYS_KEY)
    if (last === todayStr()) return false
  } catch {
    /* ignore */
  }
  return true
}

function markTrialReminderShown() {
  try {
    localStorage.setItem(TRIAL_REMINDER_DAYS_KEY, todayStr())
  } catch {
    /* ignore */
  }
}

async function checkActivation() {
  try {
    const { fetchLicenseStatus } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    const trial = status.trial

    if (status.activated) {
      return
    }
    // 许可证已过期 → 强制激活
    if (status.status === 'expired') {
      showActivation.value = true
      return
    }
    // 试用已过期且未激活 → 强制激活
    if (trial?.expired) {
      showActivation.value = true
      return
    }
    // 试用进行中且剩余 ≤7 天 → 每日提醒
    if (trial?.active && trial.remainingDays <= 7) {
      if (shouldShowTrialReminder(trial.remainingDays)) {
        trialRemainingDays.value = trial.remainingDays
        showTrialReminder.value = true
      }
    }
  } catch (e) {
    console.warn('License check failed, will retry on next poll:', e)
  }
}

async function pollLicenseStatus() {
  if (showActivation.value || showTrialReminder.value) return
  try {
    const { fetchLicenseStatus } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    if (status.activated) return
    const trial = status.trial
    if (status.status === 'expired' || (!status.activated && trial?.expired)) {
      showActivation.value = true
      return
    }
    if (trial?.active && trial.remainingDays <= 7 && shouldShowTrialReminder(trial.remainingDays)) {
      trialRemainingDays.value = trial.remainingDays
      showTrialReminder.value = true
    }
  } catch {
    // Silent fail
  }
}

function onActivated() {
  showActivation.value = false
  showTrialReminder.value = false
}

function onDismissTrialReminder() {
  markTrialReminderShown()
  showTrialReminder.value = false
}

const LICENSE_POLL_INTERVAL_MS = 5 * 60 * 1000

async function checkGuide() {
  try {
    const { getSystemSetting } = await import('@/api/db-client')
    const r = await getSystemSetting('guide_seen')
    if (!r.value || r.value !== '1') {
      guideModalRef.value?.open()
    }
  } catch (e) {
    console.warn('Guide check failed:', e)
  }
}

async function checkWhatsNew() {
  try {
    const { getSystemSetting } = await import('@/api/db-client')
    const r = await getSystemSetting('last_seen_version')
    if (!r.value || r.value !== __APP_VERSION__) {
      whatsNewModalRef.value?.open()
    }
  } catch (e) {
    console.warn('WhatsNew check failed:', e)
  }
}

/** 启动自动检查更新（延时避开启动高峰；网络失败静默——宁漏报不打扰） */
async function checkForAppUpdate() {
  try {
    const { checkUpdate } = await import('@/api/update-client')
    const { getSystemSetting } = await import('@/api/db-client')
    const r = await checkUpdate()
    if (!r.has_update) return
    const skipped = await getSystemSetting('skipped_update_version')
    if (skipped?.value === r.latest) return
    updateModalRef.value?.open(r.latest, r.url)
  } catch {
    /* 双源均不可达等网络问题：静默跳过本次检查 */
  }
}

onMounted(() => {
  authStore.loadUser()
  checkActivation()
  checkGuide()
  checkWhatsNew()
  setTimeout(() => { checkForAppUpdate() }, 8000)
  // 上报 WebView 真实 UA 给网页抓取工具（失败静默——抓取回退默认诚实 UA）
  invoke('web_fetch_set_ua', { ua: navigator.userAgent }).catch(() => {})
  licensePollTimer = setInterval(pollLicenseStatus, LICENSE_POLL_INTERVAL_MS)
})

onUnmounted(() => {
  if (licensePollTimer) {
    clearInterval(licensePollTimer)
    licensePollTimer = null
  }
})
</script>

<template>
  <div class="flex h-screen overflow-hidden bg-surface-50">
    <Sidebar :collapsed="sidebarCollapsed" @toggle="toggleSidebar" />
    <div class="flex flex-1 flex-col overflow-hidden">
      <AppHeader @toggle-sidebar="toggleSidebar" />
      <main class="flex-1 overflow-y-auto p-[10px]">
        <router-view v-slot="{ Component }">
          <KeepAlive :max="6">
            <component :is="Component" />
          </KeepAlive>
        </router-view>
      </main>
    </div>
    <ConfirmDialog />
    <Toast />
    <UploadProgress />
    <ActivationModal
      v-if="showActivation"
      @activated="onActivated"
      @close="showActivation = false"
    />
    <ActivationModal
      v-if="showTrialReminder"
      trial-reminder
      :trial-remaining-days="trialRemainingDays"
      @activated="onActivated"
      @close="onDismissTrialReminder"
    />
    <GuideModal ref="guideModalRef" />
    <WhatsNewModal ref="whatsNewModalRef" />
    <UpdateAvailableModal ref="updateModalRef" />
  </div>
</template>
