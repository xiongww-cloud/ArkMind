<script setup lang="ts">
import { ref } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { setSystemSetting } from '@/api/db-client'

const show = ref(false)

interface ChangelogEntry {
  icon: string
  iconBg: string
  iconColor: string
  title: string
  items: string[]
}

const changelog: ChangelogEntry[] = [
  {
    icon: 'envelope',
    iconBg: 'bg-blue-50',
    iconColor: 'text-blue-600',
    title: '邮件系统（全新）',
    items: [
      '接入 IMAP 邮箱，支持收件箱/已发送同步',
      '三栏布局：账户树 + 邮件列表 + 详情预览',
      '附件自动存入资料库，按内容 hash 去重',
      '内嵌图片自动渲染，大邮件懒加载',
      '邮件内容自动索引到知识库，AI 可检索',
    ],
  },
  {
    icon: 'chat-ai',
    iconBg: 'bg-purple-50',
    iconColor: 'text-purple-600',
    title: '智能问答增强',
    items: [
      'Agent 支持按发件人、主题等筛选查询邮件',
      '通知支持操作按钮（查看邮件/项目/待办、创建待办等）',
    ],
  },
  {
    icon: 'settings',
    iconBg: 'bg-slate-100',
    iconColor: 'text-slate-600',
    title: 'UI 改进',
    items: [
      '修复标题栏弹窗文字无法选中的问题',
      '通知面板展示结构化操作建议',
    ],
  },
]

function open() {
  show.value = true
}

async function close() {
  try {
    await setSystemSetting('last_seen_version', __APP_VERSION__)
  } catch (e) {
    console.error('[WhatsNewModal] save version failed:', e)
  }
  show.value = false
}

defineExpose({ open })
</script>

<template>
  <Teleport to="body">
    <Transition name="whatsnew-fade">
      <div v-if="show" class="fixed inset-0 z-[9998] flex items-center justify-center">
        <div class="absolute inset-0 bg-black/30 backdrop-blur-sm" />

        <div class="relative bg-white rounded-2xl shadow-2xl border border-surface-200 w-full max-w-[520px] mx-4 overflow-hidden z-10 flex flex-col">
          <!-- Header -->
          <div class="flex items-center justify-between px-6 pt-5 pb-3 border-b border-surface-100 shrink-0">
            <div class="flex items-center gap-2.5">
              <div class="w-8 h-8 rounded-lg overflow-hidden">
                <img src="/icon.png" class="w-8 h-8 object-cover" alt="ArkMind" />
              </div>
              <div>
                <span class="text-[13px] font-semibold text-surface-700">新版本亮点</span>
                <span class="ml-2 text-[11px] text-surface-400">v{{ __APP_VERSION__ }}</span>
              </div>
            </div>
            <button @click="close" class="w-7 h-7 flex items-center justify-center rounded-lg text-surface-400 hover:text-surface-600 hover:bg-surface-100 transition-colors" title="关闭">
              <AppIcon name="x" size="14" />
            </button>
          </div>

          <!-- Body -->
          <div class="flex-1 overflow-y-auto px-6 py-4 max-h-[420px]">
            <div v-for="(section, i) in changelog" :key="i" class="mb-5 last:mb-0">
              <div class="flex items-center gap-2.5 mb-2.5">
                <div class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0" :class="section.iconBg">
                  <AppIcon :name="section.icon" size="16" :class="section.iconColor" />
                </div>
                <h3 class="text-[14px] font-semibold text-surface-800">{{ section.title }}</h3>
              </div>
              <ul class="ml-10 space-y-1.5">
                <li v-for="(item, j) in section.items" :key="j" class="text-[12px] text-surface-600 leading-relaxed flex items-start gap-2">
                  <span class="w-1 h-1 rounded-full bg-surface-300 mt-[7px] shrink-0" />
                  <span>{{ item }}</span>
                </li>
              </ul>
            </div>
          </div>

          <!-- Footer -->
          <div class="flex items-center justify-end px-6 py-3 border-t border-surface-100 shrink-0">
            <button @click="close"
              class="px-5 py-1.5 rounded-lg text-[12px] font-medium bg-brand-600 text-white hover:bg-brand-700 transition-colors">
              知道了
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.whatsnew-fade-enter-active,
.whatsnew-fade-leave-active {
  transition: opacity 0.2s ease;
}
.whatsnew-fade-enter-from,
.whatsnew-fade-leave-to {
  opacity: 0;
}
</style>
