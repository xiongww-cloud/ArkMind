<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import AppHeader from '@/components/layout/Header.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import Toast from '@/components/common/Toast.vue'
import UploadProgress from '@/components/common/UploadProgress.vue'
import ActivationModal from '@/components/common/ActivationModal.vue'
import { useAuthStore } from '@/stores/auth'
import { listen } from '@tauri-apps/api/event'

const sidebarCollapsed = ref(false)
const authStore = useAuthStore()
const showActivation = ref(false)
let licensePollTimer = null

function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value
}

async function checkActivation() {
  try {
    const { fetchLicenseStatus } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    if (!status.activated) {
      showActivation.value = true
    } else if (status.status === 'expired') {
      showActivation.value = true
    }
  } catch (e) {
    console.warn('License check failed, showing modal:', e)
    showActivation.value = true
  }
}

async function pollLicenseStatus() {
  if (showActivation.value) return
  try {
    const { fetchLicenseStatus } = await import('@/api/db-client')
    const status = await fetchLicenseStatus()
    if (status.status === 'expired') {
      showActivation.value = true
    }
  } catch {
    // Silent fail
  }
}

function onActivated() {
  showActivation.value = false
}

const LICENSE_POLL_INTERVAL_MS = 5 * 60 * 1000

onMounted(() => {
  authStore.loadUser()
  checkActivation()
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
          <KeepAlive>
            <component :is="Component" />
          </KeepAlive>
        </router-view>
      </main>
    </div>
    <ConfirmDialog />
    <Toast />
    <UploadProgress />
    <ActivationModal v-if="showActivation" @activated="onActivated" />
  </div>
</template>
