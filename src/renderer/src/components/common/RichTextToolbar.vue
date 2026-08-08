<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { useTiptapToolbar } from '@/composables/useTiptapToolbar'

const props = defineProps<{ editor: any; atActive?: boolean }>()
const emit = defineEmits<{ at: [] }>()

const tb = useTiptapToolbar(computed(() => props.editor))

// ── @ 按钮点击：先关闭其他下拉再 emit
function onAtClick() { tb.closePopups(); emit('at') }

// 关闭外部点击弹窗
function onDocClick(e: MouseEvent) {
  const el = (e.target as HTMLElement).closest?.('.tiptap-toolbar') as HTMLElement | null
  if (!el) tb.closePopups()
}
onMounted(() => document.addEventListener('click', onDocClick, true))
onUnmounted(() => document.removeEventListener('click', onDocClick, true))
</script>

<template>
  <div class="tiptap-toolbar flex items-center gap-0 px-2 h-10 border-b border-surface-100 shrink-0" @click="tb.closePopups">

    <!-- 标题下拉 -->
    <div class="relative" @click.stop>
      <button @mousedown.prevent @click.stop="tb.togglePopup('heading')"
        class="flex items-center gap-0.5 w-[52px] h-6 justify-center rounded transition-colors duration-200 text-[11px] text-surface-500 hover:bg-surface-100 truncate">
        <span class="truncate">{{ tb.headingLabel.value }}</span>
        <svg class="w-2 h-2 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
      </button>
      <div v-if="tb.headingOpen.value" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[80px]">
        <button @mousedown.prevent="tb.execFormat('formatBlock','p'); tb.headingLabel.value='正文'; tb.headingOpen.value=false" class="w-full px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50">正文</button>
        <button @mousedown.prevent="tb.insertHeading(1); tb.headingLabel.value='标题 1'; tb.headingOpen.value=false" class="w-full px-3 py-1.5 text-[11px] text-left font-bold text-surface-600 hover:bg-surface-50">标题 1</button>
        <button @mousedown.prevent="tb.insertHeading(2); tb.headingLabel.value='标题 2'; tb.headingOpen.value=false" class="w-full px-3 py-1.5 text-[11px] text-left font-bold text-surface-600 hover:bg-surface-50">标题 2</button>
        <button @mousedown.prevent="tb.insertHeading(3); tb.headingLabel.value='标题 3'; tb.headingOpen.value=false" class="w-full px-3 py-1.5 text-[11px] text-left font-bold text-surface-600 hover:bg-surface-50">标题 3</button>
      </div>
    </div>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 加粗/斜体/下划线/删除线 -->
    <div class="flex items-center gap-0.5">
      <button @mousedown.prevent="tb.execFormat('bold')" class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 relative group"
        :class="editor?.isActive('bold') ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'">
        <b class="text-[11px]">B</b>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">加粗</span>
      </button>
      <button @mousedown.prevent="tb.execFormat('italic')" class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 relative group"
        :class="editor?.isActive('italic') ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'">
        <i class="text-[11px]">I</i>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">斜体</span>
      </button>
      <button @mousedown.prevent="tb.execFormat('underline')" class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 relative group"
        :class="editor?.isActive('underline') ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'">
        <u class="text-[11px]">U</u>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">下划线</span>
      </button>
      <button @mousedown.prevent="tb.execFormat('strikeThrough')" class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-[11px] text-surface-500 hover:bg-surface-100 line-through relative group"
        :class="editor?.isActive('strike') ? 'bg-blue-50 text-blue-600' : ''">
        <span>S</span>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">删除线</span>
      </button>
    </div>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 列表下拉 -->
    <div class="relative" @click.stop>
      <button @mousedown.prevent @click.stop="tb.togglePopup('list')"
        class="flex items-center gap-0.5 h-6 px-1.5 justify-center rounded transition-colors duration-200 text-[11px] text-surface-500 hover:bg-surface-100 relative group">
        <AppIcon name="list" size="13"/>
        <svg class="w-2 h-2 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">列表</span>
      </button>
      <div v-if="tb.listOpen.value" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[100px]">
        <button @mousedown.prevent="tb.insertList('ul'); tb.listOpen.value=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list" size="13"/>无序列表</button>
        <button @mousedown.prevent="tb.insertList('ol'); tb.listOpen.value=false" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50"><AppIcon name="list-ol" size="13"/>有序列表</button>
      </div>
    </div>

    <!-- 任务清单 -->
    <button @mousedown.prevent="tb.toggleTaskList()"
      class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 relative group"
      :class="editor?.isActive('taskList') ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'"
      title="任务清单">
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><path d="M8 12l3 3 5-6"/></svg>
      <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">任务清单</span>
    </button>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 对齐下拉 -->
    <div class="relative" @click.stop>
      <button @mousedown.prevent @click.stop="tb.togglePopup('align')"
        class="flex items-center gap-0.5 h-6 px-1.5 justify-center rounded transition-colors duration-200 text-[11px] text-surface-500 hover:bg-surface-100 relative group">
        <AppIcon :name="tb.alignLabel.value === '右对齐' ? 'align-right' : tb.alignLabel.value === '居中' ? 'align-center' : 'align-left'" size="13"/>
        <svg class="w-2 h-2 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">对齐</span>
      </button>
      <div v-if="tb.alignOpen.value" class="absolute top-full left-0 mt-1 z-50 bg-white rounded-lg shadow-lg border border-surface-100 py-1 min-w-[110px]">
        <button @mousedown.prevent="tb.execAlign('left')" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-left" size="13"/>左对齐</button>
        <button @mousedown.prevent="tb.execAlign('center')" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-center" size="13"/>居中</button>
        <button @mousedown.prevent="tb.execAlign('right')" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] text-left text-surface-600 hover:bg-surface-50"><AppIcon name="align-right" size="13"/>右对齐</button>
      </div>
    </div>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 分隔线 / 代码块 / 引用 -->
    <div class="flex items-center gap-0.5">
      <button @mousedown.prevent="tb.execFormat('insertHorizontalRule')"
        class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group">
        <span class="text-[11px]">—</span>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">分隔线</span>
      </button>
      <button @mousedown.prevent="tb.insertBlock('pre')"
        class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group">
        <AppIcon name="code-bracket" size="13"/>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">代码块</span>
      </button>
      <button @mousedown.prevent="tb.insertBlock('blockquote')"
        class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group">
        <AppIcon name="quote" size="13"/>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">引用</span>
      </button>
    </div>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 插入时间 -->
    <button @mousedown.prevent="tb.insertTime()"
      class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"
      title="插入时间">
      <AppIcon name="clock" size="13"/>
      <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">插入时间</span>
    </button>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- @ 插入链接 -->
    <button data-at-btn @mousedown.prevent @click.stop="onAtClick"
      class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 text-surface-500 hover:bg-surface-100 relative group"
      :class="atActive ? 'bg-blue-50 text-blue-600' : ''" title="插入链接">
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M16 12v1a3 3 0 0 0 6 0v-1a10 10 0 1 0-3.92 7.94"/></svg>
      <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">插入链接</span>
    </button>

    <div class="mx-1 w-px h-3 bg-surface-200"></div>

    <!-- 表格下拉 -->
    <div class="relative" @click.stop>
      <button @mousedown.prevent @click.stop="tb.togglePopup('table')"
        class="w-6 h-6 flex items-center justify-center rounded transition-colors duration-200 relative group"
        :class="editor?.isActive('table') ? 'bg-blue-50 text-blue-600' : 'text-surface-500 hover:bg-surface-100'">
        <AppIcon name="table-grid" size="13"/>
        <svg class="w-1.5 h-1.5 ml-0.5 opacity-50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M6 9l6 6 6-6"/></svg>
        <span class="absolute -top-7 left-1/2 -translate-x-1/2 bg-gray-800 text-white text-[10px] px-1.5 py-0.5 rounded whitespace-nowrap opacity-0 group-hover:opacity-100 pointer-events-none">表格</span>
      </button>
      <div v-if="tb.tableMenuOpen.value" class="absolute top-full right-0 mt-1 z-50 bg-white rounded-lg shadow-sm border border-surface-100 p-1 w-[120px]" @click.stop>
        <template v-if="!editor?.isActive('table')">
          <div class="space-y-1.5 mb-2">
            <div class="flex items-center gap-1.5">
              <span class="text-[10px] text-surface-500 w-4">行</span>
              <input v-model.number="tb.tableRows.value" type="number" min="1" max="10" class="flex-1 border border-surface-200 rounded px-1 py-0.5 text-[10px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/>
            </div>
            <div class="flex items-center gap-1.5">
              <span class="text-[10px] text-surface-500 w-4">列</span>
              <input v-model.number="tb.tableCols.value" type="number" min="1" max="8" class="flex-1 border border-surface-200 rounded px-1 py-0.5 text-[10px] text-center outline-none focus:border-brand-400 no-spinner" inputmode="numeric"/>
            </div>
          </div>
          <button @mousedown.prevent="tb.insertTableCustom()" class="w-full py-1 text-[10px] bg-brand-600 text-white rounded hover:bg-brand-700 transition-colors flex items-center justify-center gap-1">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M12 4v16m8-8H4"/></svg>插入表格
          </button>
        </template>
        <template v-else>
          <hr class="border-surface-100 my-1" />
          <button @mousedown.prevent="tb.execFormat('addColumnBefore'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">左侧插入列</button>
          <button @mousedown.prevent="tb.execFormat('addColumnAfter'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">右侧插入列</button>
          <button @mousedown.prevent="tb.execFormat('deleteColumn'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">删除列</button>
          <hr class="border-surface-100 my-0.5" />
          <button @mousedown.prevent="tb.execFormat('addRowBefore'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">上方插入行</button>
          <button @mousedown.prevent="tb.execFormat('addRowAfter'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">下方插入行</button>
          <button @mousedown.prevent="tb.execFormat('deleteRow'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-surface-600 hover:bg-surface-50 rounded">删除行</button>
          <hr class="border-surface-100 my-0.5" />
          <button @mousedown.prevent="tb.execFormat('deleteTable'); tb.tableMenuOpen.value=false" class="w-full px-2 py-1 text-[11px] text-left text-red-500 hover:bg-red-50 rounded">删除表格</button>
        </template>
      </div>
    </div>

  </div>
</template>

<style>
.no-spinner::-webkit-outer-spin-button,.no-spinner::-webkit-inner-spin-button { -webkit-appearance: none; margin: 0; }
.no-spinner { -moz-appearance: textfield; }
</style>
