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
      try {
        return JSON.parse(raw)
      } catch {
        return []
      }
    }
    return []
  }

  // ─── Canonical ID 迁移（一次性，旧双 ID 体系遗留） ─────────────────────────
  // 旧体系：本地 UUID ↔ 后端 CUID，靠 localStorage 映射维系。
  // 新体系：canonical ID = 前端本地 UUID，直接作为 projects.id。
  // 旧映射在迁移成功后保留一个版本周期（标记 migrated），供旧版本回退使用。
  const LEGACY_STORAGE_KEY = 'pms_backend_project_ids'
  const LEGACY_MIGRATED_KEY = 'pms_backend_project_ids_migrated'
  function loadLegacyIdMap() {
    try {
      const raw = localStorage.getItem(LEGACY_STORAGE_KEY)
      const parsed = raw ? JSON.parse(raw) : {}
      return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {}
    } catch {
      return {}
    }
  }

  let migrationSubmitted = false
  /**
   * 阶段 B：前端启动后提交项目身份映射，触发后端 canonical ID 迁移。
   * 主来源 = 当前项目数据（_backendId），localStorage 旧映射作补充。
   * 幂等：迁移失败允许下次启动重试。
   */
  async function submitProjectIdMigration() {
    if (migrationSubmitted) return
    migrationSubmitted = true
    try {
      if (localStorage.getItem(LEGACY_MIGRATED_KEY)) return
      const { state } = await api.migrationProjectIdState()
      if (state !== 'pending' && state !== 'failed') {
        localStorage.setItem(LEGACY_MIGRATED_KEY, '1')
        return
      }
      const legacy = loadLegacyIdMap()
      const map = []
      const seen = new Set()
      for (const p of projects.value) {
        const backendId = p._backendId || legacy[p.id]
        if (backendId && backendId !== p.id && !seen.has(p.id)) {
          seen.add(p.id)
          map.push({ localId: p.id, backendId })
        }
      }
      for (const [localId, backendId] of Object.entries(legacy)) {
        if (backendId && backendId !== localId && !seen.has(localId)) {
          seen.add(localId)
          map.push({ localId, backendId })
        }
      }
      const result = await api.migrationSubmitProjectIdMap(map)
      localStorage.setItem(LEGACY_MIGRATED_KEY, '1')
      const report = result?.report
      if (report) {
        console.warn(
          `[ID迁移] state=${report.state} migrated=${report.migrated} skipped=${report.skipped} conflicts=${report.conflicts?.length || 0}`
        )
        if (report.conflicts?.length) {
          toast.error(
            `项目数据迁移完成，但有 ${report.conflicts.length} 个项目存在冲突未迁移，请查看运行日志`
          )
        }
      }
      return report
    } catch (e) {
      migrationSubmitted = false
      console.warn('[ID迁移] 提交失败（下次启动重试）', e)
    }
  }

  // ── Project ID generator ──────────────────────────────────────
  function generateProjectId() {
    return crypto.randomUUID()
  }

  // State — starts empty, populated by ProjectHub's syncFromBackend() on mount
  const projects = ref([])

  // Getters
  const projectCount = computed(() => projects.value.length)

  // ── Dynamics store (isolated from project objects — backend fetch never touches project state) ──
  const dynamicsMap = ref({}) // { [projectId]: dynamic[] }

  const getProjectDynamics = computed(() => (projectId) => {
    if (projectId in dynamicsMap.value) return dynamicsMap.value[projectId] || []
    const project = projects.value.find((p) => p.id === projectId)
    const tl = project?.timeline
    return Array.isArray(tl) ? tl : []
  })

  const projectsByStatus = computed(
    () => (status) => projects.value.filter((p) => p.status === status)
  )

  const getProjectById = computed(() => (id) => projects.value.find((p) => p.id === id))

  const getProjectPlans = computed(() => (projectId) => {
    const project = projects.value.find((p) => p.id === projectId)
    const pl = project?.plans
    return Array.isArray(pl) ? pl : []
  })

  const stats = computed(() => ({
    total: projects.value.length,
    lead: projects.value.filter((p) => p.status === 'Lead').length,
    solutionDesign: projects.value.filter((p) => p.status === 'SolutionDesign').length,
    poc: projects.value.filter((p) => p.status === 'PoC').length,
    quotation: projects.value.filter((p) => p.status === 'Quotation').length,
    bidding: projects.value.filter((p) => p.status === 'Bidding').length,
    won: projects.value.filter((p) => p.status === 'Won').length,
    ordered: projects.value.filter((p) => p.status === 'Ordered').length,
    lost: projects.value.filter((p) => p.status === 'Lost').length,
    onHold: projects.value.filter((p) => p.status === 'OnHold').length
  }))

  /**
   * 确保后端存在该项目记录（canonical ID 语义：本地 id 即后端 id）。
   * 仅用于兼容历史遗留的"本地有、后端无"项目；新建项目走 addProject 直接落库。
   */
  async function syncProjectToBackend(projectId) {
    const project = projects.value.find((p) => p.id === projectId)
    if (!project) throw new Error('项目未找到')
    try {
      const existing = await api.fetchProject(projectId)
      if (existing) return projectId
    } catch {
      // 查询失败时尝试创建
    }
    const created = await api.createProject({
      name: project.name || '',
      customer: project.customer || '',
      budget: project.budget || 0,
      status: project.status || 'Lead',
      description: project.description || '',
      owner: project.owner || '',
      // displayId 即 canonical ID 通道：后端将其作为 projects.id
      displayId: projectId,
      bidTime: project.bidTime || '',
      customerContact: project.customerContact || '',
      customerPhone: project.customerPhone || ''
    })
    return created.id || projectId
  }

  // Actions
  // 关键路径：后端写成功才提交本地状态（本地 store 是后端持久化状态的可信投影）
  async function addProject(project) {
    const id = generateProjectId()
    await api.createProject({
      name: project.name || '',
      customer: project.customer || '',
      budget: project.budget || 0,
      status: project.status || 'Lead',
      description: project.description || '',
      owner: project.owner || '',
      userId: project.userId || '',
      displayId: id,
      bidTime: project.bidTime || '',
      customerContact: project.customerContact || '',
      customerPhone: project.customerPhone || ''
    })
    projects.value.push({
      ...project,
      id,
      createdAt: new Date().toISOString().split('T')[0],
      timeline: [],
      plans: [
        {
          id: 'plan-' + Date.now(),
          name: '默认方案',
          items: [],
          history: []
        }
      ]
    })
    return id
  }

  function addPlan(projectId, planName) {
    const project = projects.value.find((p) => p.id === projectId)
    if (project) {
      if (!project.plans) project.plans = []
      const planId = 'plan-' + Date.now()
      let result = planId
      project.plans.push({
        id: planId,
        name: planName || '新方案',
        items: [],
        history: []
      })
      syncAndCreateSolution(projectId, planId, planName || '新方案')
      return result
    }
  }

  async function syncAndCreateSolution(projectId, planId, name) {
    try {
      await syncProjectToBackend(projectId)
      const solution = await api.createSolution(projectId, name)
      const project = projects.value.find((p) => p.id === projectId)
      const plan = project?.plans?.find((pl) => pl.id === planId)
      if (plan) plan._backendSolutionId = solution.id
    } catch (e) {
      console.warn('后端同步方案失败', e)
    }
  }

  function updatePlan(projectId, planId, updates) {
    const project = projects.value.find((p) => p.id === projectId)
    if (Array.isArray(project?.plans)) {
      const plan = project.plans.find((pl) => pl.id === planId)
      if (plan) {
        Object.assign(plan, updates)
        if (updates.name && plan._backendSolutionId) {
          api
            .renameSolution(plan._backendSolutionId, updates.name)
            .catch((e) => console.warn('后端重命名方案失败', e))
        }
      }
    }
  }

  async function deletePlan(projectId, planId) {
    const project = projects.value.find((p) => p.id === projectId)
    if (!Array.isArray(project?.plans)) return
    const idx = project.plans.findIndex((pl) => pl.id === planId)
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
      toast.error('删除方案失败，请重试')
    }
  }

  function savePlanSnapshot(projectId, planId) {
    const project = projects.value.find((p) => p.id === projectId)
    if (Array.isArray(project?.plans)) {
      const plan = project.plans.find((pl) => pl.id === planId)
      if (plan) {
        plan.history.unshift({
          timestamp: new Date().toISOString(),
          items: JSON.parse(JSON.stringify(plan.items))
        })
      }
    }
  }

  /** 关键路径：后端状态写成功才提交本地；失败回滚本地状态。返回是否成功。 */
  async function updateProjectStatus(projectId, newStatus) {
    const project = projects.value.find((p) => p.id === projectId)
    if (!project) return false
    const oldStatus = project.status
    project.status = newStatus
    try {
      const authStore = useAuthStore()
      await syncProjectToBackend(projectId)
      await api.updateProjectStatus(projectId, newStatus, authStore.currentUser.name || '系统')
      project.updatedAt = new Date().toISOString()
      return true
    } catch (e) {
      project.status = oldStatus
      console.warn('后端同步项目状态失败', e)
      toast.error('状态更新失败，请重试')
      return false
    }
  }

  function addTimelineEvent(projectId, event) {
    const newEvent = {
      date: formatDateTime(new Date().toISOString()),
      ...event
    }
    const current = dynamicsMap.value[projectId] || []
    dynamicsMap.value = { ...dynamicsMap.value, [projectId]: [newEvent, ...current] }
    syncAndAddDynamic(projectId, newEvent)
  }

  async function syncAndAddDynamic(projectId, event) {
    try {
      await syncProjectToBackend(projectId)
      const result = await api.createProjectDynamic({ projectId, ...event, author: event.operator })
      const project = projects.value.find((p) => p.id === projectId)
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
      const raw = await api.fetchDynamics(projectId)
      const dynamics = Array.isArray(raw)
        ? raw.map((d) => ({
            date: d.date || d.created_at || '',
            content: d.content,
            operator: d.operator,
            _id: d.id,
            type: d.type || 'system'
          }))
        : []
      dynamicsMap.value = { ...dynamicsMap.value, [projectId]: dynamics }
      return dynamics
    } catch (e) {
      console.warn('获取项目动态失败', e)
      if (projectId in dynamicsMap.value) return dynamicsMap.value[projectId]
      const project = projects.value.find((p) => p.id === projectId)
      const tl = project?.timeline
      return Array.isArray(tl) ? tl : []
    }
  }

  function applyRemoteFields(project, remote) {
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
    project.wonAt = remote.won_at || ''
    project.expectedOrderDate = remote.expected_order_date || ''
    project.customerContact = remote.customer_contact || ''
    project.customerPhone = remote.customer_phone || ''
  }

  /** Re-fetch project details from backend and merge into local store */
  async function fetchProjectDetail(projectId) {
    let project = projects.value.find((p) => p.id === projectId)

    if (!project) {
      try {
        const remote = await api.fetchProject(projectId)
        if (!remote) return null
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
          plans: []
        })
        projects.value.push(project)
      } catch {
        return null
      }
    }

    try {
      const remote = await api.fetchProject(project.id)
      if (remote) applyRemoteFields(project, remote)
      // Also fetch solutions from backend
      try {
        const solutions = await api.fetchProjectSolutions(project.id)
        if (Array.isArray(solutions) && solutions.length) {
          project.plans = solutions.map((s) => {
            const versions = Array.isArray(s.versions) ? s.versions : []
            const mapVersion = (v) => ({
              versionId: v.id,
              versionNum: v.versionNum || v.version,
              tags: v.tags ? String(v.tags).split(',').filter(Boolean) : [],
              content: v.content || '',
              createdAt: v.createdAt || v.created_at,
              amount: v.amount || 0,
              operator: v.creator || '',
              status: v.status || 'final',
              items: parseItems(v.items).map((di) => ({
                uid: di.id,
                productId: di.productId || '',
                rowType: di.rowType || 'device',
                name: di.name || '',
                model: di.rowType === 'subnet' ? '' : di.model || '',
                customName: di.rowType === 'subnet' ? '' : di.deviceName || '',
                unitPrice: di.rowType === 'subnet' ? 0 : di.price || 0,
                quantity: di.rowType === 'subnet' ? 0 : di.count || 1,
                unit: di.rowType === 'subnet' ? '' : di.unit || '台',
                mergedText: di.rowType === 'subnet' ? '' : di.parameters || '',
                params: []
              }))
            })
            // status='draft' 的版本 → 恢复为编辑中草稿；其余 → 历史版本
            const draftRow = versions.find((v) => (v.status || 'final') === 'draft')
            const history = versions
              .filter((v) => (v.status || 'final') !== 'draft')
              .map(mapVersion)
            return {
              id: s.id,
              _backendSolutionId: s.id,
              name: s.name,
              items: [],
              history,
              draft: draftRow
                ? {
                    ...mapVersion(draftRow),
                    _versionId: draftRow.id,
                    startedAt: draftRow.createdAt || draftRow.created_at,
                    operator: draftRow.creator || ''
                  }
                : null
            }
          })
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
      await syncProjectToBackend(projectId)
      const authStore = useAuthStore()
      const result = await api.createProjectDynamic({
        projectId,
        content,
        author: operator || authStore.currentUser.name,
        type
      })
      const newEvent = {
        date: result.date ? formatDateTime(result.date) : formatDateTime(new Date().toISOString()),
        content: result.content,
        operator: result.operator,
        _id: result.id,
        type: result.type || type
      }
      const current = dynamicsMap.value[projectId] || []
      dynamicsMap.value = { ...dynamicsMap.value, [projectId]: [newEvent, ...current] }
      const project = projects.value.find((p) => p.id === projectId)
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
      if (projectId in dynamicsMap.value) {
        dynamicsMap.value = {
          ...dynamicsMap.value,
          [projectId]: dynamicsMap.value[projectId].filter((d) => d._id !== dynamicId)
        }
      }
    } catch (e) {
      console.warn('删除动态失败', e)
    }
  }

  async function editDynamic(dynamicId, projectId, content) {
    try {
      await api.editProjectDynamic(dynamicId, content)
      if (projectId in dynamicsMap.value) {
        dynamicsMap.value = {
          ...dynamicsMap.value,
          [projectId]: dynamicsMap.value[projectId].map((d) =>
            d._id === dynamicId ? { ...d, content } : d
          )
        }
      }
    } catch (e) {
      console.warn('编辑动态失败', e)
    }
  }

  /** Update project-level fields and sync to backend. Returns a promise so callers can await the full round-trip. */
  function updateProject(projectId, updates) {
    const project = projects.value.find((p) => p.id === projectId)
    if (project) {
      Object.assign(project, updates)
      return syncAndUpdateProject(projectId, updates)
    }
  }

  function updateProjectInList(projectId, updates) {
    const p = projects.value.find((p) => p.id === projectId)
    if (p) Object.assign(p, updates)
  }

  async function syncAndUpdateProject(projectId, updates) {
    try {
      await syncProjectToBackend(projectId)
      await api.updateProject(projectId, updates)
      const project = projects.value.find((p) => p.id === projectId)
      if (project) project.updatedAt = new Date().toISOString()
    } catch (e) {
      console.warn('后端同步项目详情失败', e)
      toast.error('项目保存失败，请重试')
    }
  }

  async function archiveProject(projectId) {
    const project = projects.value.find((p) => p.id === projectId)
    if (!project) return
    try {
      await syncProjectToBackend(projectId)
      await api.archiveProject(projectId, useAuthStore().currentUser.name)
      project.isArchived = true
      project.updatedAt = new Date().toISOString()
      toast.success('项目已归档')
    } catch (e) {
      console.warn('归档项目失败', e)
      toast.error('归档失败，请重试')
    }
  }

  /** 关键路径：后端删除成功才移除本地；失败保留本地并提示。返回是否成功。 */
  async function deleteProject(projectId) {
    try {
      await api.deleteProjectBackend(projectId)
    } catch (e) {
      console.warn('数据库删除项目失败', e)
      toast.error('删除失败，请重试')
      return false
    }
    const idx = projects.value.findIndex((p) => p.id === projectId)
    if (idx !== -1) {
      projects.value.splice(idx, 1)
    }
    return true
  }

  function syncPlanToBackend(projectId, planId) {
    return syncAndCreateSolution(projectId, planId, '')
  }

  /** 创建草稿版本（确保后端方案存在，status='draft'），返回完整版本对象 */
  async function createPlanDraft(projectId, planId, data) {
    const p = projects.value.find((p) => p.id === projectId)
    const plan = p?.plans?.find((pl) => pl.id === planId)
    let sid = plan?._backendSolutionId
    if (!sid) {
      await syncAndCreateSolution(projectId, planId, plan?.name || '方案')
      sid = plan?._backendSolutionId
      if (!sid) throw new Error('方案同步失败')
    }
    return api.createVersion(sid, {
      deviceItems: data.deviceItems || [],
      versionNum: data.versionNum || 'V1.0',
      content: data.content || '',
      tags: data.tags || [],
      creator: data.operator || '',
      status: 'draft'
    })
  }

  /** 静默保存草稿到后端（status 保持 'draft'），失败静默 */
  async function savePlanDraft(projectId, planId, data) {
    if (!data._versionId) return
    try {
      await api.saveDraftVersion(data._versionId, {
        deviceItems: data.deviceItems || [],
        versionNum: data.versionNum || 'V1.0',
        content: data.content || '',
        tags: data.tags || []
      })
    } catch (e) {
      console.warn('[Solution] 保存草稿失败:', e)
    }
  }

  async function savePlanVersion(projectId, planId, data) {
    const p = projects.value.find((p) => p.id === projectId)
    const plan = p?.plans?.find((pl) => pl.id === planId)
    let sid = plan?._backendSolutionId
    if (!sid) {
      await syncAndCreateSolution(projectId, planId, plan?.name || '方案')
      sid = plan?._backendSolutionId
      if (!sid) throw new Error('方案同步失败')
    }
    // 草稿版本已在库中（status='draft'）→ 先保存内容再 finalize
    if (data._versionId) {
      try {
        await api.saveDraftVersion(data._versionId, {
          deviceItems: data.deviceItems || [],
          versionNum: data.versionNum || 'V1.0',
          content: data.content || '',
          tags: data.tags || []
        })
        const r = await api.finalizeDraftVersion(data._versionId, {})
        return r
      } catch (e) {
        toast.error('方案版本保存失败，请重试')
        throw e
      }
    }
    try {
      const r = await api.createVersion(sid, {
        deviceItems: data.deviceItems || [],
        versionNum: data.versionNum || 'V1.0',
        content: data.content || '',
        tags: data.tags || [],
        amount: data.amount || 0,
        creator: data.operator || ''
      })
      return r
    } catch (e) {
      toast.error('方案版本保存失败，请重试')
      throw e
    }
  }

  async function fetchPlanVersions(projectId, planId) {
    const p = projects.value.find((p) => p.id === projectId)
    const plan = p?.plans?.find((pl) => pl.id === planId)
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
    // API-driven actions
    syncProjectToBackend,
    syncPlanToBackend,
    createPlanDraft,
    savePlanDraft,
    savePlanVersion,
    fetchPlanVersions,
    // Canonical ID 迁移（一次性）
    submitProjectIdMigration
  }
})
