<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { useRouter, useRoute } from 'vue-router'

const props = defineProps({
  collapsed: { type: Boolean, default: false }
})

const emit = defineEmits(['toggle'])

const router = useRouter()
const route = useRoute()

const navItems = [
  { path: '/dashboard', label: '总览', icon: 'dashboard' },
  { path: '/ai-insights', label: 'AI洞察', icon: 'bell' },
  { path: '/projects', label: '项目', icon: 'folder-open' },
  { path: '/notes', label: '笔记', icon: 'file-doc' },
  { path: '/todos', label: '待办', icon: 'list' },
  { path: '/contacts', label: '通讯录', icon: 'users' },
  { path: '/email', label: '邮件', icon: 'envelope' },
  { path: '/products', label: '产品', icon: 'cube' },
  { path: '/assets', label: '资料', icon: 'archive' },
  { path: '/knowledge-graph', label: '知识观测台', icon: 'network' },
  { path: '/knowledge/brain', label: '智能问答', icon: 'chat' },
  // { path: '/agent-debug', label: 'Agent调试', icon: 'bar-chart' },
  // { path: '/inspector', label: '索引检查', icon: 'search' },
  { path: '/settings', label: '系统设置', icon: 'settings' }
]

function isActive(path) {
  return route.path.startsWith(path)
}

function navigate(path) {
  router.push(path)
}
</script>

<template>
  <aside class="flex flex-col border-r border-surface-200 bg-white w-16 flex-shrink-0">
    <!-- App Logo -->
    <div class="flex h-[46px] items-center justify-center pt-1.5 pb-1">
      <div class="flex h-8 w-8 items-center justify-center rounded-lg overflow-hidden">
        <img src="/icon.png" class="h-8 w-8 object-cover" alt="Arkmind" />
      </div>
    </div>

    <!-- Navigation -->
    <nav class="flex-1 space-y-1 p-2 -mt-1">
      <button
        v-for="item in navItems"
        :key="item.path"
        @click="navigate(item.path)"
        class="group relative flex w-full items-center justify-center rounded-lg py-2.5 transition-all duration-150"
        :class="isActive(item.path)
          ? 'bg-brand-50 text-brand-700'
          : 'text-surface-400 hover:bg-surface-50 hover:text-surface-600'"
      >
<AppIcon :name="item.icon" class="h-5 w-5" />
        <!-- Tooltip -->
        <span class="absolute left-full ml-2 px-2 py-1 bg-gray-800 text-white text-[11px] rounded whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50 shadow-lg">
          {{ item.label }}
        </span>
      </button>
    </nav>
  </aside>
</template>
