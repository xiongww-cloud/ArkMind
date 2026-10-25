<script setup lang="ts">
import { ref, onMounted } from 'vue'
import Sidebar from '@/components/layout/Sidebar.vue'
import AppHeader from '@/components/layout/Header.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import Toast from '@/components/common/Toast.vue'
import UploadProgress from '@/components/common/UploadProgress.vue'
import GuideModal from '@/components/common/GuideModal.vue'
import WhatsNewModal from '@/components/common/WhatsNewModal.vue'
import { useAuthStore } from '@/stores/auth'
import { invoke } from '@tauri-apps/api/core'

const sidebarCollapsed = ref(false)
const authStore = useAuthStore()
const guideModalRef = ref<InstanceType<typeof GuideModal> | null>(null)
const whatsNewModalRef = ref<InstanceType<typeof WhatsNewModal> | null>(null)

function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value
}

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

onMounted(() => {
  authStore.loadUser()
  checkGuide()
  checkWhatsNew()
  // 上报 WebView 真实 UA 给网页抓取工具（失败静默——抓取回退默认诚实 UA）
  invoke('web_fetch_set_ua', { ua: navigator.userAgent }).catch(() => {})
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
    <GuideModal ref="guideModalRef" />
    <WhatsNewModal ref="whatsNewModalRef" />
  </div>
</template>
