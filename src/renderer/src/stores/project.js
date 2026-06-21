import { defineStore } from 'pinia'
import { ref, computed, reactive } from 'vue'
import * as api from '@/api/db-client'
import { useAuthStore } from '@/stores/auth'
import { toast } from '@/composables/useToast'
import { formatDateTime } from '@/utils/time'

export const useProjectStore = defineStore('project', () => {
  // ─── Helper: parse items from backend (SQLite json_group_array returns a JSON string) ──
  function parseItems(raw) {
    if (Array.isArray(raw)) return raw
    if (typeof raw === 'string') {
      try { return JSON.parse(raw) } catch { return [] }
    }
    return []
  }

  // ─── Persisted backend-ID map (survives page refresh) ──────────────────────
  const STORAGE_KEY = 'pms_backend_project_ids'
  function loadBackendIdMap() {
    try {
      const raw = localStorage.getItem(STORAGE_KEY)
      return raw ? JSON.parse(raw) : {}
    } catch { return {} }
  }
  function saveBackendIdMap(map) {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(map)) } catch { /* quota exceeded */ }
  }

  // ── Project ID generator: PRO-YYYYMMDDHHmm-RANDOM(4) ──────────────────────
  function generateProjectId() {
    return crypto.randomUUID()
  }

  // State — starts empty, populated by ProjectHub's syncFromBackend() on mount
  const projects = ref([])


  // Getters
  const projectCount = computed(() => projects.value.length)

  // ── Dynamics store (isolated from project objects — backend fetch never touches project state) ──
  const dynamicsMap = ref({})  // { [projectId]: dynamic[] }

  const getProjectDynamics = computed(() => (projectId) => {
    // Return backend-fetched dynamics if available, otherwise fall back to project.timeline
    if (projectId in dynamicsMap.value) return dynamicsMap.value[projectId] || []
    const project = projects.value.find(p => p.id === projectId)
    const tl = project?.timeline; return Array.isArray(tl) ? tl : []
  })

  const projectsByStatus = computed(() => (status) =>
    projects.value.filter(p => p.status === status)
  )

  const getProjectById = computed(() => (id) =>
    projects.value.find(p => p.id === id)
  )

  const getProjectPlans = computed(() => (projectId) => {
    const project = projects.value.find(p => p.id === projectId)
    const pl = project?.plans; return Array.isArray(pl) ? pl : []
  })

  const stats = computed(() => ({
    total: projects.value.length,
    lead: projects.value.filter(p => p.status === 'Lead').length,
    solutionDesign: projects.value.filter(p => p.status === 'SolutionDesign').length,
    poc: projects.value.filter(p => p.status === 'PoC').length,
    quotation: projects.value.filter(p => p.status === 'Quotation').length,
    bidding: projects.value.filter(p => p.status === 'Bidding').length,
    won: projects.value.filter(p => p.status === 'Won').length,
    ordered: projects.value.filter(p => p.status === 'Ordered').length,
    lost: projects.value.filter(p => p.status === 'Lost').length,
    onHold: projects.value.filter(p => p.status === 'OnHold').length
  }))

  // Actions
  function addProject(project) {
    const id = generateProjectId()
    projects.value.push({
      ...project,
      id,
      createdAt: new Date().toISOString().split('T')[0],
      timeline: [],
      plans: [{
        id: 'plan-' + Date.now(),
        name: '默认方案',
        items: [],
        history: []
      }]
    })
    // Sync to backend (fire-and-forget)
    const created = projects.value.find(p => p.id === id)
    if (created) syncProjectToBackend(id).catch(e => console.warn('后端同步项目失败', e))
    return id
  }

  function addPlan(projectId, planName) {
    const project = projects.value.find(p => p.id === projectId)
    if (project) {
      if (!project.plans) project.plans = []
      const planId = 'plan-' + Date.now(); let result = planId
      project.plans.push({
        id: planId,
        name: planName || '新方案',
        items: [],
        history: []
      })
      // Sync to backend
      syncAndCreateSolution(projectId, planId, planName || '新方案')
      return result
    }
  }

  async function syncAndCreateSolution(projectId, planId, name) {
    try {
      const backendProjectId = await syncProjectToBackend(projectId)
      const solution = await api.createSolution(backendProjectId, name)
      const project = projects.value.find(p => p.id === projectId)
      const plan = project?.plans?.find(pl => pl.id === planId)
      if (plan) plan._backendSolutionId = solution.id
    } catch (e) {
      console.warn('后端同步方案失败', e)
    }
  }

  function updatePlan(projectId, planId, updates) {
    const project = projects.value.find(p => p.id === projectId)
    if (Array.isArray(project?.plans)) {
      const plan = project.plans.find(pl => pl.id === planId)
      if (plan) {
        Object.assign(plan, updates)
        // Sync rename to backend
        if (updates.name && plan._backendSolutionId) {
          api.renameSolution(plan._backendSolutionId, updates.name)
            .catch(e => console.warn('后端重命名方案失败', e))
        }
      }
    }
  }

  async function deletePlan(projectId, planId) {
    const project = projects.value.find(p => p.id === projectId)
    if (!Array.isArray(project?.plans)) return
    const idx = project.plans.findIndex(pl => pl.id === planId)
    if (idx === -1) return
    const plan = project.plans[idx]
    // 先调后端删除，成功后再移除本地数据
    try {
      if (plan._backendSolutionId) {
        await api.deleteSolution(plan._backendSolutionId)
      }
      project.plans.splice(idx, 1)
    } catch (e) {
      console.warn('后端删除方案失败', e)
    }
  }

  function savePlanSnapshot(projectId, planId) {
    const project = projects.value.find(p => p.id === projectId)
    if (Array.isArray(project?.plans)) {
      const plan = project.plans.find(pl => pl.id === planId)
      if (plan) {
        plan.history.unshift({
          timestamp: new Date().toISOString(),
          items: JSON.parse(JSON.stringify(plan.items))
        })
      }
    }
  }

  function updateProjectStatus(projectId, newStatus) {
    const project = projects.value.find(p => p.id === projectId)
    if (project) {
      project.status = newStatus
      // Sync to backend with operator for audit log
      const authStore = useAuthStore()
      return syncAndUpdateStatus(projectId, newStatus, authStore.currentUser.name)
    }
  }

  async function syncAndUpdateStatus(projectId, newStatus, operator) {
    try {
      const backendId = await syncProjectToBackend(projectId)
      await api.updateProjectStatus(backendId, newStatus, operator || '系统')
      const project = projects.value.find(p => p.id === projectId)
      if (project) project.updatedAt = new Date().toISOString()
    } catch (e) {
      console.warn('后端同步项目状态失败', e)
    }
  }

  function addTimelineEvent(projectId, event) {
    const newEvent = {
      date: formatDateTime(new Date().toISOString()),
      ...event
    }
    // Store in isolated dynamicsMap
    const current = dynamicsMap.value[projectId] || []
    dynamicsMap.value = { ...dynamicsMap.value, [projectId]: [newEvent, ...current] }
    syncAndAddDynamic(projectId, newEvent)
  }

  async function syncAndAddDynamic(projectId, event) {
    try {
      const backendId = await syncProjectToBackend(projectId)
      const result = await api.createProjectDynamic({ projectId: backendId, ...event, author: event.operator })
      const project = projects.value.find(p => p.id === projectId)
      if (project) project.updatedAt = new Date().toISOString()
      return result
    } catch (e) {
      console.warn('后端同步项目动态失败', e)
      throw e
    }
  }

  /**
   * Fetch dynamics from backend → store in isolated dynamicsMap.
   * NEVER mutates project.timeline — dynamics and project state are independent.
   */
  async function fetchProjectDynamics(projectId) {
    try {
      // 支持 CUID 和 PRO-xxx 两种 ID
      let backendId = null
      const localProject = projects.value.find(p => p.id === projectId)
      if (localProject) {
        // 项目已存在于 store，直接用 syncProjectToBackend
        backendId = await syncProjectToBackend(localProject.id)
      } else {
        // 可能是 CUID 或尚未加载的项目
        const byBackend = projects.value.find(p => p._backendId === projectId)
        if (byBackend) {
          backendId = await syncProjectToBackend(byBackend.id)
        } else {
          // 直接尝试作为 CUID 查询
          const raw = await api.fetchDynamics(projectId)
          const dynamics = Array.isArray(raw) ? raw.map(d => ({
            date: d.date || d.created_at || '',
            content: d.content,
            operator: d.operator,
            _id: d.id,
            type: d.type || 'system',
          })) : []
          dynamicsMap.value = { ...dynamicsMap.value, [projectId]: dynamics }
          return dynamics
        }
      }
      const raw = await api.fetchDynamics(backendId)
      const dynamics = Array.isArray(raw) ? raw.map(d => ({
        date: d.date || d.created_at || '',
        content: d.content,
        operator: d.operator,
        _id: d.id,
        type: d.type || 'system',
      })) : []
      dynamicsMap.value = { ...dynamicsMap.value, [projectId]: dynamics }
      return dynamics
    } catch (e) {
      console.warn('获取项目动态失败', e)
      // 保留现有缓存，不覆盖
      if (projectId in dynamicsMap.value) return dynamicsMap.value[projectId]
      // 也尝试通过 _backendId 查找项目
      const project = projects.value.find(p => p.id === projectId || p._backendId === projectId)
      const tl = project?.timeline; return Array.isArray(tl) ? tl : []
    }
  }

  /** Re-fetch project details from backend and merge into local store */
  async function fetchProjectDetail(projectId) {
    let project = projects.value.find(p => p.id === projectId)

    // Backend-driven: create project from API if not in local store.
    // BEFORE creating, check for existing project that already maps to this backend ID
    // OR shares the same name — prevents duplicates when navigating via backend CUID
    // from the Dashboard's "最近项目" list.
    if (!project) {
      try {
        const remote = await api.fetchProject(projectId)
        if (!remote) return null

        // Check if any existing project already references this backend ID
        project = projects.value.find(p => p._backendId === projectId)
        // Fallback: check by name match or displayId
        if (!project) {
          project = projects.value.find(p => p.name === remote.name || p.id === remote.displayId)
        }

        if (project) {
          // Existing project found — merge backend fields and ensure the ID mapping is set
          project._backendId = projectId
          project.name = remote.name
          project.customer = remote.customer || ''
          project.budget = remote.budget || ''
          project.description = remote.description || ''
          project.status = remote.status
          project.owner = remote.owner || ''
          project.userId = remote.userId || ''
          project.displayId = remote.displayId || ''
          project.createdAt = remote.created_at || project.createdAt
          project.updatedAt = remote.updated_at || null
          project.isArchived = remote.isArchived || false
          project.bidTime = remote.bid_time || ''
          project.customerContact = remote.customer_contact || ''
          project.customerPhone = remote.customer_phone || ''

          // Persist the mapping so Hub's syncFromBackend finds it next time
          const idMap = loadBackendIdMap()
          idMap[project.id] = projectId
          saveBackendIdMap(idMap)
        } else {
          // Truly new (or not yet synced) — create from backend data
          project = reactive({
            id: remote.id,
            name: remote.name,
            customer: remote.customer || '',
            budget: remote.budget || '',
            description: remote.description || '',
            status: remote.status,
            owner: remote.owner || '',
            displayId: remote.displayId || '',
            createdAt: remote.created_at || new Date().toISOString().slice(0, 10),
            updatedAt: remote.updated_at || null,
            bidTime: remote.bid_time || '',
            customerContact: remote.customer_contact || '',
            customerPhone: remote.customer_phone || '',
            plans: [],
          })
          projects.value.push(project)
        }
      } catch {
        return null
      }
    }

    try {
      // Use project.id (PRO-xxx frontend ID), not projectId param (may be CUID)
      const backendId = await syncProjectToBackend(project.id)
      const remote = await api.fetchProject(backendId)
      // Merge only the fields that are owned by the backend
      if (remote) {
        project.name = remote.name
        project.customer = remote.customer || ''
        project.budget = remote.budget || ''
        project.description = remote.description || ''
        project.status = remote.status
        project.owner = remote.owner || ''
        project.userId = remote.userId || ''
        project.displayId = remote.displayId || ''
        project.createdAt = remote.created_at || project.createdAt
        project.updatedAt = remote.updated_at || null
        project.isArchived = remote.isArchived || false
        project.bidTime = remote.bid_time || ''
        project.customerContact = remote.customer_contact || ''
        project.customerPhone = remote.customer_phone || ''
        console.log('[fetchProjectDetail] 已从后端刷新项目数据:', project.name)
      }
      // Also fetch solutions from backend
      try {
        const solutions = await api.fetchProjectSolutions(backendId)
        if (Array.isArray(solutions) && solutions.length) {
          project.plans = solutions.map(s => ({
            id: s.id,
            _backendSolutionId: s.id,
            name: s.name,
            items: [],
            history: Array.isArray(s.versions) ? s.versions.map(v => ({
              versionId: v.id,
              versionNum: v.versionNum,
              tags: v.tags ? v.tags.split(',').filter(Boolean) : [],
              content: v.content,
              createdAt: v.createdAt,
              amount: v.amount,
              operator: v.creator,
              items: parseItems(v.items).map(di => ({
                uid: di.id,
                productId: di.productId || '',
                rowType: di.rowType || 'device',
                name: di.name || '',
                model: di.rowType === 'subnet' ? '' : (di.model || ''),
                customName: di.rowType === 'subnet' ? '' : (di.deviceName || ''),
                unitPrice: di.rowType === 'subnet' ? 0 : (di.price || 0),
                quantity: di.rowType === 'subnet' ? 0 : (di.count || 1),
                unit: di.rowType === 'subnet' ? '' : (di.unit || '台'),
                mergedText: di.rowType === 'subnet' ? '' : (di.parameters || ''),
                params: [],
              })),
            })) : [],
          }))
        }
      } catch (e) {
        console.warn('从后端刷新方案列表失败', e)
      }
      return project
    } catch (e) {
      console.warn('从后端刷新项目详情失败', e)
      return project
    }
  }

  /** Add dynamic via API and unshift to isolated dynamicsMap */
  async function addProjectDynamic(projectId, content, operator, type = 'system') {
    try {
      const backendId = await syncProjectToBackend(projectId)
      const authStore = useAuthStore()
      const result = await api.createProjectDynamic({ projectId: backendId, content, author: operator || authStore.currentUser.name, type })
      const newEvent = {
        date: result.date ? formatDateTime(result.date) : formatDateTime(new Date().toISOString()),
        content: result.content,
        operator: result.operator,
        _id: result.id,
        type: result.type || type,
      }
      // Unshift to isolated dynamicsMap (never touches project.timeline)
      const current = dynamicsMap.value[projectId] || []
      dynamicsMap.value = { ...dynamicsMap.value, [projectId]: [newEvent, ...current] }
      // Refresh project updatedAt so UI reflects the change immediately
      const project = projects.value.find(p => p.id === projectId)
      if (project) project.updatedAt = new Date().toISOString()
      return newEvent
    } catch (e) {
      console.error('添加项目动态失败', e)
      throw e
    }
  }

  async function deleteDynamic(dynamicId, projectId) {
    try {
      await api.deleteProjectDynamic(dynamicId)
      // 直接更新本地缓存，不依赖后端重新拉取
      if (projectId in dynamicsMap.value) {
        dynamicsMap.value = { ...dynamicsMap.value, [projectId]: dynamicsMap.value[projectId].filter(d => d._id !== dynamicId) }
      }
    } catch (e) {
      console.warn('删除动态失败', e)
    }
  }

  async function editDynamic(dynamicId, projectId, content) {
    try {
      await api.editProjectDynamic(dynamicId, content)
      if (projectId in dynamicsMap.value) {
        dynamicsMap.value = { ...dynamicsMap.value, [projectId]: dynamicsMap.value[projectId].map(d => d._id === dynamicId ? { ...d, content } : d) }
      }
    } catch (e) {
      console.warn('编辑动态失败', e)
    }
  }

  /** Update project-level fields and sync to backend. Returns a promise so callers can await the full round-trip. */
  function updateProject(projectId, updates) {
    const project = projects.value.find(p => p.id === projectId)
    if (project) {
      Object.assign(project, updates)
      return syncAndUpdateProject(projectId, updates)
    }
  }

  function updateProjectInList(projectId, updates) {
    const p = projects.value.find(p => p.id === projectId || p._backendId === projectId)
    if (p) Object.assign(p, updates)
  }

  async function syncAndUpdateProject(projectId, updates) {
    try {
      const backendId = await syncProjectToBackend(projectId)
      await api.updateProject(backendId, updates)
      const project = projects.value.find(p => p.id === projectId)
      if (project) project.updatedAt = new Date().toISOString()
    } catch (e) {
      console.warn('后端同步项目详情失败', e)
    }
  }

  async function archiveProject(projectId) {
    const project = projects.value.find(p => p.id === projectId)
    if (!project) return
    try {
      const backendId = await syncProjectToBackend(projectId)
      await api.archiveProject(backendId, useAuthStore().currentUser.name)
      project.isArchived = true
      project.updatedAt = new Date().toISOString()
      toast.success('项目已归档')
    } catch (e) {
      console.warn('归档项目失败', e)
      toast.error('归档失败，请重试')
    }
  }

  async function deleteProject(projectId) {
    // Always persist the deletion intent FIRST, so even hardcoded projects
    // without _backendId won't resurrect on page refresh
    // Delete from database (cascade all child records)
    try {
      await api.deleteProjectBackend(projectId)
    } catch (e) {
      console.warn('数据库删除项目失败（已从本地移除）', e)
    }
    // Remove from local store
    const idx = projects.value.findIndex(p => p.id === projectId)
    if (idx !== -1) {
      projects.value.splice(idx, 1)
    }
  }
  async function syncProjectToBackend(projectId) {
    const project = projects.value.find(p => p.id === projectId)
    if (!project) throw new Error('项目未找到')
    try {
      const existing = await api.fetchProject(projectId)
      if (existing) return projectId
    } catch (_) {}
    const created = await api.createProject({
      name: project.name || '',
      customer: project.customer || '',
      budget: project.budget || 0,
      status: project.status || 'Lead',
      description: project.description || '',
      owner: project.owner || '',
      displayId: projectId,
      bidTime: project.bidTime || '',
      customerContact: project.customerContact || '',
      customerPhone: project.customerPhone || '',
    })
    return created.id || projectId
  }
  function syncPlanToBackend(projectId, planId) {
    return syncAndCreateSolution(projectId, planId, '')
  }

  async function savePlanVersion(projectId, planId, data) {
    const p = projects.value.find(p => p.id === projectId)
    const plan = p?.plans?.find(pl => pl.id === planId)
    let sid = plan?._backendSolutionId
    if (!sid) {
      await syncAndCreateSolution(projectId, planId, plan?.name || '方案')
      sid = plan?._backendSolutionId
      if (!sid) return
    }
    const r = await api.createVersion(sid, {
      deviceItems: data.deviceItems || [],
      versionNum: data.versionNum || 'V1.0',
      content: data.content || '',
      tags: data.tags || [],
      amount: data.amount || 0,
      creator: data.operator || ''
    })
    return r
  }

  async function fetchPlanVersions(projectId, planId) {
    const p = projects.value.find(p => p.id === projectId)
    const plan = p?.plans?.find(pl => pl.id === planId)
    if (!plan?._backendSolutionId) return []
    const raw = await api.fetchSolutionVersions(plan._backendSolutionId)
    return Array.isArray(raw) ? raw : []
  }

  return {
    projects,
    projectCount,
    projectsByStatus,
    getProjectById,
    getProjectPlans,
    getProjectDynamics,
    dynamicsMap,
    stats,
    addProject,
    addPlan,
    updatePlan,
    deletePlan,
    savePlanSnapshot,
    updateProjectStatus,
    updateProject,
    updateProjectInList,
    addTimelineEvent,
    fetchProjectDynamics,
    fetchProjectDetail,
    addProjectDynamic,
    deleteDynamic,
    editDynamic,
    deleteProject,
    archiveProject,
    // Shared helpers (also used by ProjectHub)
    loadBackendIdMap,
    saveBackendIdMap,
    // API-driven actions
    syncProjectToBackend,
    syncPlanToBackend,
    savePlanVersion,
    fetchPlanVersions,
  }
})