<script setup lang="ts">
import { ref, computed } from 'vue'
import { useProductStore, CATEGORY_LABELS, CATEGORIES } from '@/stores/product'
import { confirm, alert } from '@/composables/useConfirm'
import AppIcon from '@/components/common/AppIcon.vue'
import { formatDateTime } from '@/utils/time'

const store = useProductStore()

const showForm = ref(false)
const editingId = ref(null)
const form = ref({ name: '', category: '', parentId: '' })
const isSaving = ref(false)

// ─── Build tree for display ───────────────────────────────────────────────
const allSeries = computed(() => store.series)

/** Series grouped by category for display */
const seriesByCategory = computed(() => {
  const groups = {}
  for (const cat of CATEGORIES) {
    groups[cat] = allSeries.value.filter(s => s.category === cat)
  }
  return groups
})

function getSeriesName(id) {
  const s = allSeries.value.find(s => s.id === id)
  return s ? s.name : ''
}

// ─── Form ─────────────────────────────────────────────────────────────────
function openCreate(parentId) {
  editingId.value = null
  form.value = { name: '', category: store.selectedCategory || '', parentId: parentId || '' }
  showForm.value = true
}

function openEdit(series) {
  editingId.value = series.id
  form.value = { name: series.name, category: series.category, parentId: series.parentId || '' }
  showForm.value = true
}

function cancelForm() {
  showForm.value = false
  editingId.value = null
}

async function saveSeries() {
  const name = form.value.name.trim()
  const category = form.value.category
  if (!name) { alert({ title: '提示', message: '系列名称不可为空' }); return }
  if (!category) { alert({ title: '提示', message: '请选择产品大类' }); return }
  isSaving.value = true
  try {
    if (editingId.value) {
      await store.updateSeries(editingId.value, {
        name,
        category,
        parentId: form.value.parentId || null,
      })
    } else {
      await store.createSeries({
        name,
        category,
        parentId: form.value.parentId || null,
      })
    }
    cancelForm()
    await store.fetchSeries()
  } catch (e) {
    alert({ title: '保存失败', message: e.message || '网络异常' })
  } finally {
    isSaving.value = false
  }
}

async function deleteSeries(id) {
  const ok = await confirm({ title: '删除系列', message: '确定删除该产品系列？产品不会被删除，仅解除关联。' })
  if (!ok) return
  try {
    await store.deleteSeries(id)
  } catch (e) {
    alert({ title: '删除失败', message: e.message || '网络异常' })
  }
}

// ─── Init ─────────────────────────────────────────────────────────────────
store.fetchSeries()
</script>

<template>
  <div class="space-y-4">
    <!-- Header -->
    <div class="flex items-center gap-3">
      <h2 class="text-sm font-semibold text-surface-700">
        产品目录
        <span class="text-surface-400 font-normal">({{ store.series.length }})</span>
      </h2>
      <button class="btn-primary" @click="openCreate('')">
        <AppIcon name="plus" class="w-3.5 h-3.5" />
        新建系列
      </button>
    </div>

    <!-- Create/Edit Form -->
    <div v-if="showForm" class="card !p-4 border-brand-200 bg-brand-50/30">
      <div class="flex items-end gap-3 flex-wrap">
        <div class="flex-1 min-w-[160px]">
          <label class="block text-xs font-medium text-surface-500 mb-1">系列名称 *</label>
          <input v-model="form.name" type="text" class="input-field" placeholder="如 OceanStor" @keyup.enter="saveSeries" />
        </div>
        <div class="w-36">
          <label class="block text-xs font-medium text-surface-500 mb-1">所属大类 *</label>
          <select v-model="form.category" class="input-field">
            <option value="">选择大类</option>
            <option v-for="cat in CATEGORIES" :key="cat" :value="cat">{{ CATEGORY_LABELS[cat] }}</option>
          </select>
        </div>
        <div class="w-44">
          <label class="block text-xs font-medium text-surface-500 mb-1">父系列</label>
          <select v-model="form.parentId" class="input-field">
            <option value="">无（根节点）</option>
            <option v-for="s in allSeries" :key="s.id" :value="s.id">{{ CATEGORY_LABELS[s.category] }} / {{ s.name }}</option>
          </select>
        </div>
      </div>
      <div class="flex items-center gap-2 mt-3">
        <button class="btn-primary" :disabled="isSaving" @click="saveSeries">
          {{ isSaving ? '保存中...' : (editingId ? '更新' : '创建') }}
        </button>
        <button class="btn-ghost" @click="cancelForm"><AppIcon name="x" class="w-3 h-3" />取消</button>
      </div>
    </div>

    <!-- Series List by Category -->
    <div v-if="store.series.length === 0 && !store.isFetchingSeries" class="card text-center py-8">
      <p class="text-sm text-surface-400">暂无产品系列，点击「新建系列」开始管理</p>
    </div>

    <div v-for="cat in CATEGORIES" :key="cat" class="space-y-2">
      <template v-if="seriesByCategory[cat] && seriesByCategory[cat].length > 0">
        <h3 class="text-xs font-semibold text-surface-500 uppercase tracking-wide">{{ CATEGORY_LABELS[cat] }}</h3>
        <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
          <div
            v-for="s in seriesByCategory[cat]"
            :key="s.id"
            class="card !p-4 group"
          >
            <div class="flex items-start justify-between mb-1.5">
              <div class="min-w-0">
                <p class="text-sm font-semibold text-surface-800 truncate">{{ s.name }}</p>
                <p v-if="s.parentId" class="text-[11px] text-surface-400 mt-0.5">
                  <AppIcon name="chevron-right" class="w-2.5 h-2.5 inline" />
                  子系列: {{ getSeriesName(s.parentId) }}
                </p>
              </div>
            </div>
            <p class="text-[10px] text-surface-400">{{ formatDateTime(s.updatedAt) }}</p>
            <div class="flex items-center gap-1.5 mt-2 pt-2 border-t border-surface-100">
              <button @click="openCreate(s.id)" class="btn-secondary !text-[10px] !px-2 !h-5 flex-shrink-0" title="添加子系列">
                <AppIcon name="plus" class="w-3 h-3" />
                子系列
              </button>
              <button @click="openEdit(s)" class="btn-ghost !p-1 !h-5 flex-shrink-0" title="编辑">
                <AppIcon name="edit" class="w-3 h-3" />
              </button>
              <button @click="deleteSeries(s.id)" class="btn-ghost !p-1 !h-5 text-red-400 hover:text-red-600 flex-shrink-0" title="删除">
                <AppIcon name="trash" class="w-3 h-3" />
              </button>
            </div>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>
