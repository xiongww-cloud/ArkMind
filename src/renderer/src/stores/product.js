import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import * as api from '@/api/db-client'

export const UNCATEGORIZED_NAME = '未分类'

export const useProductStore = defineStore('product', () => {
  // ─── State ──────────────────────────────────────────────────────────────────
  const series = ref([]) // flat list of ProductSeries (aka 产品目录)
  const products = ref([]) // flat list of Product
  const isFetching = ref(false)
  const isFetchingSeries = ref(false)

  // ─── UI state ───────────────────────────────────────────────────────────────
  const searchQuery = ref('')
  const selectedSeriesId = ref(null) // series.id | null
  const selectedProductId = ref(null) // product.id | null (for detail panel)
  const sortOrder = ref('asc') // 'asc' | 'desc'

  // ─── Unclassified directory ──────────────────────────────────────────────────
  const uncategorizedSeriesId = computed(() => {
    const found = series.value.find((s) => s.name === UNCATEGORIZED_NAME && !s.parentId)
    return found?.id || null
  })

  // ─── Helper: get all descendant series IDs ───────────────────────────────────
  function getDescendantSeriesIds(seriesId) {
    const ids = [seriesId]
    const queue = [seriesId]
    while (queue.length > 0) {
      const current = queue.shift()
      const children = series.value.filter((s) => s.parentId === current)
      for (const child of children) {
        ids.push(child.id)
        queue.push(child.id)
      }
    }
    return ids
  }

  // ─── Series tree (computed from flat list) ──────────────────────────────────
  const seriesTree = computed(() => {
    const list = series.value
    const map = new Map()
    let uncatNode = null
    const otherRoots = []
    for (const s of list) {
      const node = { ...s, children: [] }
      map.set(s.id, node)
      if (s.name === UNCATEGORIZED_NAME && !s.parentId) {
        uncatNode = node
      }
    }
    for (const s of list) {
      const node = map.get(s.id)
      if (s.parentId && map.has(s.parentId)) {
        map.get(s.parentId).children.push(node)
      } else if (node !== uncatNode) {
        otherRoots.push(node)
      }
    }
    // Sort other roots alphabetically, put 未分类 last
    const sorted = otherRoots.sort((a, b) => a.name.localeCompare(b.name))
    return uncatNode ? [...sorted, uncatNode] : sorted
  })

  // ─── Filtered products ──────────────────────────────────────────────────────
  const filteredProducts = computed(() => {
    let result = products.value
    const query = searchQuery.value.trim()
    if (query) {
      // Global search: ignore directory filter
      const q = query.toLowerCase()
      result = result.filter((p) => p.name.toLowerCase().includes(q))
    } else if (selectedSeriesId.value) {
      const isUncategorized = selectedSeriesId.value === uncategorizedSeriesId.value
      const descendantIds = getDescendantSeriesIds(selectedSeriesId.value)
      result = result.filter((p) => {
        if (isUncategorized && (!p.seriesId || p.seriesId === '')) return true
        return p.seriesId && descendantIds.includes(p.seriesId)
      })
    }
    // Sort by name
    result = [...result].sort((a, b) => {
      const cmp = (a.name || '').localeCompare(b.name || '')
      return sortOrder.value === 'desc' ? -cmp : cmp
    })
    return result
  })

  function toggleSortOrder() {
    sortOrder.value = sortOrder.value === 'asc' ? 'desc' : 'asc'
  }

  const selectedProduct = computed(() => {
    if (!selectedProductId.value) return null
    return products.value.find((p) => p.id === selectedProductId.value) || null
  })

  // ─── Actions: Series (产品目录) ──────────────────────────────────────────────

  async function ensureUncategorized() {
    const existing = series.value.find((s) => s.name === UNCATEGORIZED_NAME && !s.parentId)
    if (!existing) {
      try {
        const s = await api.createProductSeries({
          name: UNCATEGORIZED_NAME,
          category: 'STORAGE',
          parentId: null
        })
        series.value.push(s)
        return s
      } catch (e) {
        console.warn('Failed to create 未分类 directory', e)
      }
    }
    return existing
  }

  async function fetchSeries() {
    isFetchingSeries.value = true
    try {
      const data = await api.fetchProductSeries()
      series.value = (Array.isArray(data) ? data : []).map((s) => ({
        ...s,
        parentId: s.parent_id || null
      }))
      await ensureUncategorized()
    } catch (e) {
      console.warn('获取产品目录失败', e)
    } finally {
      isFetchingSeries.value = false
    }
  }

  async function createSeries(data) {
    // Normalize parentId for backend
    const payload = { ...data }
    if (payload.parentId === null) delete payload.parentId
    const s = await api.createProductSeries(payload)
    const normalized = { ...s, parentId: s.parent_id ?? s.parentId ?? payload.parentId ?? null }
    series.value.push(normalized)
    return normalized
  }

  async function updateSeries(id, data) {
    await api.updateProductSeries(id, data)
    // Merge updated fields immediately (the API may only return success, not data)
    const idx = series.value.findIndex((s) => s.id === id)
    if (idx !== -1) Object.assign(series.value[idx], data)
  }

  async function deleteSeries(id) {
    await api.deleteProductSeries(id)
    series.value = series.value.filter((s) => s.id !== id)
    if (selectedSeriesId.value === id) selectedSeriesId.value = null
    await fetchProducts()
  }

  // ─── Actions: Products ──────────────────────────────────────────────────────

  async function fetchProducts(params) {
    isFetching.value = true
    try {
      // Normalize: if called with a string (series ID), wrap as { seriesId }
      if (typeof params === 'string') params = { seriesId: params }
      if (!params) params = {}
      // H10 修复：始终拉全量产品，不在后端按 seriesId 过滤
      // 否则 products.value 会丢失其他目录的数据，导致 isDuplicateName 查重绕过、Excel 导入同名校验失效
      const data = await api.fetchProducts(params)
      products.value = (Array.isArray(data) ? data : []).map((p) => ({
        ...p,
        seriesId: p.series_id ?? p.seriesId ?? null,
        seriesName: p.series_name ?? p.seriesName ?? ''
      }))
    } catch (e) {
      console.warn('获取产品列表失败', e)
    } finally {
      isFetching.value = false
    }
  }

  async function createProduct(data) {
    const product = await api.createProduct(data)
    products.value.unshift({
      ...product,
      seriesId: product.series_id ?? product.seriesId ?? data.seriesId ?? null,
      seriesName: product.series_name ?? product.seriesName ?? ''
    })
    return product
  }

  async function updateProduct(id, data) {
    await api.updateProduct(id, data)
    // Merge updated fields into existing product (don't replace, the API only returns sent fields)
    const idx = products.value.findIndex((p) => p.id === id)
    if (idx !== -1) {
      Object.assign(products.value[idx], data)
    }
    return products.value[idx]
  }

  async function deleteProduct(id) {
    await api.deleteProduct(id)
    products.value = products.value.filter((p) => p.id !== id)
    if (selectedProductId.value === id) selectedProductId.value = null
  }

  // ─── Actions: Materials ─────────────────────────────────────────────────────

  async function uploadMaterial(productId, formData) {
    const material = await api.uploadFile({
      filePath: formData.get('file')?.path,
      entityType: 'product',
      entityIds: [productId],
      customName: formData.get('customName'),
      note: formData.get('note'),
      creator: formData.get('creator')
    })
    return material
  }

  async function deleteMaterial(productId, materialId) {
    await api.deleteMaterial(materialId)
  }

  async function deleteAllMaterial(productId, materialId) {
    await api.deleteAllMaterial(materialId)
  }

  async function getMaterialAssociations(materialId) {
    const result = await api.getMaterialAssociations(materialId)
    if (result.success) return result.productIds || []
    return []
  }

  async function updateMaterialAssociations(materialId, productIds) {
    return api.updateMaterialAssociations(materialId, productIds)
  }

  // ─── Navigation helpers ─────────────────────────────────────────────────────

  function selectSeries(seriesId) {
    selectedSeriesId.value = seriesId
    selectedProductId.value = null
    fetchProducts()
  }

  function selectProduct(productId) {
    selectedProductId.value = productId
  }

  return {
    series,
    products,
    isFetching,
    isFetchingSeries,
    searchQuery,
    selectedSeriesId,
    selectedProductId,
    sortOrder,
    toggleSortOrder,
    uncategorizedSeriesId,
    seriesTree,
    filteredProducts,
    selectedProduct,
    getDescendantSeriesIds,
    ensureUncategorized,
    fetchSeries,
    createSeries,
    updateSeries,
    deleteSeries,
    fetchProducts,
    createProduct,
    updateProduct,
    deleteProduct,
    uploadMaterial,
    deleteMaterial,
    deleteAllMaterial,
    getMaterialAssociations,
    updateMaterialAssociations,
    selectSeries,
    selectProduct
  }
})
