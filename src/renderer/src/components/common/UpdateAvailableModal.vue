<script setup lang="ts">
import { ref } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { openUrl } from '@tauri-apps/plugin-opener'
import { setSystemSetting } from '@/api/db-client'

const show = ref(false)
const latestVersion = ref('')
const downloadUrl = ref('')
const appVersion = __APP_VERSION__

function open(version: string, url: string) {
  latestVersion.value = version
  downloadUrl.value = url
  show.value = true
}

/** 仅关闭本次会话（下次启动若仍未更新会再提示） */
function close() {
  show.value = false
}

/** 跳过此版本：记录后不再 nag 同一版本 */
async function skipThisVersion() {
  try {
    await setSystemSetting('skipped_update_version', latestVersion.value)
  } catch (e) {
    console.error('[UpdateModal] save skipped version failed:', e)
  }
  show.value = false
}

async function goDownload() {
  try {
    await openUrl(downloadUrl.value)
  } catch (e) {
    console.error('[UpdateModal] open release page failed:', e)
  }
}

defineExpose({ open })
</script>

<template>
  <Teleport to="body">
    <Transition name="update-fade">
      <div v-if="show" class="fixed inset-0 z-[9997] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/30 backdrop-blur-sm" @click="close" />

        <div class="relative bg-white rounded-2xl shadow-2xl border border-surface-200 w-full max-w-[400px] mx-4 overflow-hidden z-10 flex flex-col">
          <div class="flex items-center justify-between px-5 pt-4 pb-2 shrink-0">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg bg-blue-50 flex items-center justify-center shrink-0">
                <AppIcon name="download" size="16" class="text-blue-600" />
              </div>
              <span class="text-[13px] font-semibold text-surface-700">发现新版本</span>
            </div>
            <button @click="close" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition-colors" v-tooltip="'关闭'">
              <AppIcon name="x" size="14" />
            </button>
          </div>

          <div class="px-5 pb-4">
            <p class="text-[12.5px] text-surface-600 leading-relaxed">
              新版本 <span class="font-semibold text-surface-800">v{{ latestVersion }}</span> 已发布（当前 v{{ appVersion }}）。
              安装新版本后，本次更新内容将出现在「新版本亮点」中。
            </p>
          </div>

          <div class="flex items-center justify-end gap-2 px-5 pb-4 pt-1 border-t border-surface-100">
            <button
              @click="skipThisVersion"
              class="px-3 py-1.5 text-[12px] text-surface-500 hover:text-surface-700 hover:bg-surface-100 rounded-lg transition-colors"
            >
              跳过此版本
            </button>
            <button
              @click="goDownload"
              class="px-4 py-1.5 text-[12px] font-medium text-white bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors flex items-center gap-1.5"
            >
              <AppIcon name="download" size="13" />
              去下载
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.update-fade-enter-active,
.update-fade-leave-active {
  transition: opacity 0.18s ease;
}
.update-fade-enter-from,
.update-fade-leave-to {
  opacity: 0;
}
</style>
