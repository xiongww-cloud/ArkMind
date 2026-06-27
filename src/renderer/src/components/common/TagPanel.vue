<script setup lang="ts">
import { ref, watch, onMounted, computed, nextTick } from 'vue'
import { entityGetTags, entityAddTag, entityRemoveTag, tagList } from '@/api/db-client'
import { toast } from '@/composables/useToast'

const props = defineProps<{ entityType: string; entityId: string }>()

const tagContainer = ref<HTMLElement | null>(null)
const addBtn = ref<HTMLElement | null>(null)
const tags = ref<any[]>([])
const showDropdown = ref(false)
const searchQuery = ref('')
const allTags = ref<any[]>([])
const activeCategory = ref<string | null>(null)
const dropdownStyle = ref<Record<string, string>>({})
const allCategories = computed(() => {
  const seen = new Set<string>()
  allTags.value.forEach((t: any) => {
    if (t.type && !seen.has(t.type)) seen.add(t.type)
  })
  return Array.from(seen).sort().map(key => ({
    key,
    label: categoryLabel[key] || (key.charAt(0).toUpperCase() + key.slice(1).replace(/_/g, ' '))
  }))
})

const categoryLabel: Record<string,string> = { general:'通用',scene:'场景',tech:'技术',product:'产品',daily:'日常',custom:'自定义' }

const availableTags = computed(() => {
  let list = allTags.value.filter((t: any) => !tags.value.some(et => et.tag_id === t.tag_id))
  if (activeCategory.value) list = list.filter((t: any) => t.type === activeCategory.value)
  const q = searchQuery.value.trim().toLowerCase()
  if (q) list = list.filter((t: any) => t.name.toLowerCase().includes(q))
  return list
})

async function loadTags() {
  try { const r = await entityGetTags(props.entityType, props.entityId); tags.value = r?.tags || [] }
  catch { tags.value = [] }
}

async function loadAllTags() {
  try { const r = await tagList(); allTags.value = Array.isArray(r) ? r : [] }
  catch { allTags.value = [] }
}

async function addTag(tagName: string) {
  try { await entityAddTag(props.entityType, props.entityId, tagName); await loadTags(); searchQuery.value = '' }
  catch { toast({ message: '添加标签失败', type: 'error' }) }
}

async function removeTag(tagId: string) {
  try { await entityRemoveTag(props.entityType, props.entityId, tagId); await loadTags() }
  catch { toast({ message: '删除标签失败', type: 'error' }) }
}

function toggleDropdown() {
  showDropdown.value = !showDropdown.value
  if (showDropdown.value) {
    loadAllTags(); searchQuery.value = ''; activeCategory.value = null
    nextTick(() => {
      if (!addBtn.value) return
      const rect = addBtn.value.getBoundingClientRect()
      const w = 320
      const h = 360
      // 水平边界
      let left = Math.max(8, rect.left)
      if (left + w > window.innerWidth - 8) left = window.innerWidth - w - 8
      // 垂直边界：上方空间不足时改为向下展开
      const spaceAbove = rect.top
      const spaceBelow = window.innerHeight - rect.bottom
      const openUp = spaceAbove > h || spaceAbove > spaceBelow
      dropdownStyle.value = {
        left: left + 'px',
        [openUp ? 'bottom' : 'top']: openUp ? (window.innerHeight - rect.top + 4) + 'px' : (rect.bottom + 4) + 'px',
        height: Math.min(h, (openUp ? spaceAbove : spaceBelow) - 12) + 'px',
      }
    })
  }
}

watch(() => props.entityId, () => { if (props.entityId) loadTags() })
onMounted(() => { if (props.entityId) loadTags() })
</script>

<template>
  <div ref="tagContainer">
    <div class="flex items-center gap-1.5 flex-wrap">
      <span v-for="tag in tags" :key="tag.tag_id"
        class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] border transition-all shrink-0"
        :style="{ background: tag.color + '12', borderColor: tag.color + '25', color: tag.color }">
        <span class="w-1.5 h-1.5 rounded-full shrink-0" :style="{ background: tag.color }"></span>
        {{ tag.name }}
        <button @click="removeTag(tag.tag_id)" class="w-3 h-3 flex items-center justify-center rounded-full hover:bg-red-50 transition-colors shrink-0">
          <svg class="w-2 h-2 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path d="M6 18L18 6M6 6l12 12"/></svg>
        </button>
      </span>
      <button ref="addBtn" @click="toggleDropdown" class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] text-surface-400 border border-dashed border-surface-300 hover:border-brand-400 hover:text-brand-600 transition-colors shrink-0">
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path d="M12 4v16m8-8H4"/></svg>
        标签
      </button>
    </div>
    <Teleport to="body">
      <div v-if="showDropdown" class="fixed z-[100] w-[320px] bg-white rounded-xl shadow-xl border border-surface-200 overflow-hidden flex flex-col" :style="{ ...dropdownStyle, height: '360px' }">
        <div class="flex items-center gap-1 px-3 py-2 border-b border-surface-100 overflow-x-auto">
          <button @click="activeCategory=null" class="shrink-0 px-2 py-0.5 rounded-full text-[10px] transition-colors" :class="activeCategory===null ? 'bg-brand-600 text-white' : 'bg-surface-100 text-surface-500 hover:bg-surface-200'">全部</button>
          <button v-for="cat in allCategories" :key="cat.key" @click="activeCategory=activeCategory===cat.key?null:cat.key" class="shrink-0 px-2 py-0.5 rounded-full text-[10px] transition-colors" :class="activeCategory===cat.key ? 'bg-brand-600 text-white' : 'bg-surface-100 text-surface-500 hover:bg-surface-200'">{{ cat.label }}<span class="ml-1 opacity-60">{{ allTags.filter((t:any)=>t.type===cat.key).length }}</span></button>
        </div>
        <div class="px-3 py-2">
          <input v-model="searchQuery" placeholder="搜索或输入新标签..." class="w-full border border-surface-200 rounded-lg px-2.5 py-1.5 text-[11px] outline-none focus:border-brand-400" @keyup.enter="searchQuery.trim() && addTag(searchQuery.trim())"/>
        </div>
        <div class="flex-1 overflow-y-auto px-1 pb-1">
          <button v-if="searchQuery.trim()" @click="addTag(searchQuery.trim())" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] rounded-lg hover:bg-brand-50 text-brand-600 transition-colors">
            新建「{{ searchQuery.trim() }}」
          </button>
          <button v-for="t in availableTags.slice(0, 12)" :key="t.tag_id" @click="addTag(t.name)" class="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] rounded-lg hover:bg-surface-50 text-left transition-colors">
            <span class="w-2.5 h-2.5 rounded-full shrink-0" :style="{ background: t.color }"></span>
            <span class="flex-1 truncate text-surface-700">{{ t.name }}</span>
            <span class="text-[9px] text-surface-400 shrink-0">{{ t.type }}</span>
          </button>
          <p v-if="availableTags.length===0 && !searchQuery.trim()" class="text-[11px] text-surface-400 text-center py-4">暂无可选标签</p>
        </div>
      </div>
    </Teleport>
    <div v-if="showDropdown" class="fixed inset-0 z-[90]" @click="showDropdown=false;searchQuery=''"></div>
  </div>
</template>
