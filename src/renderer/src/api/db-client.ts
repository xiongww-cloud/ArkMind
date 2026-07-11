/**
 * Tauri IPC database client — v10.0.1
 * All data access via @tauri-apps/api invoke() calling Rust Tauri commands
 */
import { invoke } from '@tauri-apps/api/core'

// ─── Types ──────────────────────────────────────────────────────
export interface NoteRow { note_id: string; title: string; content: string; is_daily: number; notebook_id: string|null; project_ids: string|null; created_at: string; updated_at: string; [key:string]: unknown }
export interface NotebookRow { id: string; name: string; color: string; sort_order: number; parent_id?: string|null; [key:string]: unknown }
export interface SuccessResponse<T = void> { success: boolean; error?: string }
export interface FetchNotesResponse extends SuccessResponse { notes: NoteRow[] }
export interface FetchNotebooksResponse extends SuccessResponse { notebooks: NotebookRow[] }
export interface ProjectRow { id: string; name: string; customer: string; budget: string|number; description: string; status: string; owner: string; userId: string; displayId: string; created_at: string; updated_at: string; isArchived: boolean; bid_time: string; customer_contact: string; customer_phone: string; [key:string]: unknown }
export interface FileRow { id: string; file_ext: string; file_size: number; parse_status: string; global_name: string; custom_name: string; created_at: string; updated_at: string; [key:string]: unknown }
export interface ProductRow { id: string; name: string; series_id: string|null; description: string; standard_specs: string; full_specs: string; created_at: string; updated_at: string; [key:string]: unknown }
export interface ProductSeriesRow { id: string; name: string; parent_id: string|null; category: string; sort_order: number; [key:string]: unknown }
export interface TodoRow { id: string; title: string; content: string; done: number|boolean; priority: string; due_date: string; repeat: string; project_id: string; project_name: string; created_at: string; [key:string]: unknown }
export interface LicenseStatus { activated: boolean; status: string; product: string; expiresAt: string; activatedAt: string; machineCode: string }
export interface DynamicRow { id: string; content: string; operator: string; date: string; type: string; created_at: string; [key:string]: unknown }
export interface SolutionRow { id: string; name: string; project_id: string; [key:string]: unknown }
export interface VersionRow { id: string; versionNum: string; tags: string; content: string; amount: number; creator: string; items: string; createdAt: string; [key:string]: unknown }
export interface AiConfig { providers?: AiProvider[]; activeProviderId?: string; visionProviders?: AiProvider[]; activeVisionId?: string; embedding?: { baseUrl: string; apiKey: string; modelName: string }; [key:string]: unknown }
export interface AiProvider { id: string; name: string; apiKey: string; baseUrl: string; modelName: string; [key:string]: unknown }
export interface ChatSession { id: string; name: string; messages: ChatMessage[]; createdAt: string }
export interface ChatMessage { role: 'user'|'assistant'; content: string; [key:string]: unknown }

// ─── User ───────────────────────────────────────────────────────
export async function fetchUserProfile(): Promise<UserProfile> { return invoke('user_profile') }
export async function updateUserProfile(data: Record<string,any>): Promise<void> { await invoke('user_profile_update', { data }) }

// ─── License ────────────────────────────────────────────────────
export async function fetchLicenseStatus(): Promise<LicenseStatus> { return invoke('license_status') }
export async function activateLicense(key: string): Promise<{message:string;expiresAt:string}> { return invoke('license_activate', { key }) }
export async function getMachineCode(): Promise<string> { return invoke('machine_code') }

// ─── Projects ──────────────────────────────────────────────────
export function fetchProject(id: string): Promise<ProjectRow|null> { return invoke('project_get', { id }) }
export async function fetchProjects(): Promise<ProjectRow[]> { return invoke('project_list') }
export async function createProject(data: Record<string,any>): Promise<ProjectRow> { return invoke('project_create', { data }) }
export async function updateProject(id: string, data: Record<string,any>): Promise<void> { await invoke('project_update', { id, data }) }
export async function deleteProject(id: string): Promise<void> { await invoke('project_delete', { id }) }
export const deleteProjectBackend = deleteProject

export async function createProjectDynamic(data: Record<string,any>): Promise<DynamicRow> { return invoke('project_dynamics_add', { projectId: data.projectId, event: { type: data.type||'note', content: data.content||'', operator: data.author||'' } }) }
export async function fetchDynamics(projectId: string): Promise<DynamicRow[]> { return invoke('project_dynamics_list', { projectId }) }
export async function fetchAllDynamics(): Promise<DynamicRow[]> { return invoke('project_dynamics_list_all') }
export async function fetchProjectDynamics(projectId: string): Promise<DynamicRow[]> { return fetchDynamics(projectId) }
export async function deleteProjectDynamic(dynamicId: string): Promise<void> { await invoke('project_dynamics_delete', { dynamicId }) }
export async function editProjectDynamic(dynamicId: string, content: string): Promise<void> { await invoke('project_dynamics_edit', { dynamicId, content }) }
export async function updateProjectStatus(projectId: string, newStatus: string, operator: string): Promise<void> { await invoke('project_update_status', { projectId, newStatus, operator }) }
export async function archiveProject(projectId: string, operator: string): Promise<void> { await invoke('project_archive', { projectId, operator }) }
export async function exportProject(projectId: string, savePath?: string): Promise<{success:boolean;exportPath?:string;error?:string}> { return invoke('project_export', { projectId, savePath }) }
export async function importProject(data: any): Promise<{success:boolean;error?:string}> { return invoke('project_import', { data }) }
export async function projectImportFile(filePath: string): Promise<{success:boolean;error?:string}> { return invoke('project_import_file', { filePath }) }
export async function projectExportFiles(projectId: string): Promise<{success:boolean;error?:string}> { return invoke('project_export_files', { projectId }) }
export async function projectsBatchExport(projectIds: string[]): Promise<{success:boolean;exportPath?:string;error?:string}> { return invoke('projects_batch_export', { projectIds }) }

// ─── Products ──────────────────────────────────────────────────
export async function fetchProductSeries(): Promise<ProductSeriesRow[]> { return invoke('product_series_list') }
export async function createProductSeries(data: Record<string,any>): Promise<ProductSeriesRow> { return invoke('product_series_create', { data }) }
export async function updateProductSeries(id: string, data: Record<string,any>): Promise<void> { return invoke('product_series_update', { id, data }) }
export async function deleteProductSeries(id: string): Promise<void> { return invoke('product_series_delete', { id }) }
export async function updateProductSeriesSortOrders(orders: {id:string;sortOrder:number}[]): Promise<void> { return invoke('product_series_update_sort_order', { data: orders }) }
export async function fetchProducts(params?: Record<string,any>): Promise<ProductRow[]> { return invoke('product_list', { seriesId: params?.seriesId||null }) }
export async function createProduct(data: Record<string,any>): Promise<ProductRow> { return invoke('product_create', { data }) }
export async function updateProduct(id: string, data: Record<string,any>): Promise<void> { return invoke('product_update', { id, data }) }
export async function updateProductSortOrders(orders: {id:string;sortOrder:number}[]): Promise<void> { return invoke('product_update_sort_orders', { data: orders }) }
export async function deleteProduct(id: string): Promise<void> { return invoke('product_delete', { id }) }

// ─── Files ─────────────────────────────────────────────────────
export async function fetchFiles(projectId: string): Promise<FileRow[]> { return invoke('file_list', { projectId }) }
/// 统一文件上传：entityType 区分域（project / library / product / note / todo）
export async function uploadFile(data: any): Promise<{fileId?:string;id?:string;[key:string]:any}> { return invoke('file_upload', { data }) }
export async function deleteFile(fileId: string): Promise<void> { return invoke('file_delete', { fileId }) }
export async function updateFileMeta(fileId: string, data: Record<string,any>): Promise<void> { return invoke('file_update_meta', { fileId, data }) }
export async function openFile(fileId: string): Promise<void> { return invoke('file_open', { versionId: fileId }) }
export async function downloadFile(versionId: string, suggestedName?: string): Promise<{success:boolean;error?:string}> { const { save } = await import('@tauri-apps/plugin-dialog'); const name = suggestedName || 'download'; const p = await save({ defaultPath: name }); if(!p) return {success:false,error:'cancelled'}; return invoke('file_download', { data: { versionId, savePath: p } }) }
export async function getDownloadUrl(versionId: string): Promise<string> { return invoke('file_get_path', { versionId }) }
export async function fileAddTagMapping(fileId: string, tagName: string): Promise<void> { return invoke('file_add_tag_mapping', { fileId, tagName }) }
export async function fileRemoveTagMapping(fileId: string, tagId: string): Promise<void> { return invoke('file_remove_tag_mapping', { fileId, tagId }) }


// ─── 通用文件预览 & 导出（适用于任意 file_assets ID） ──
export async function previewFile(fileId: string): Promise<any> { return invoke('file_preview', { fileId }) }
export async function exportFile(fileId: string): Promise<any> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const r: any = await invoke('file_export', { fileId });
  if (!r?.success) return r;
  const p = await save({ defaultPath: r.fileName || 'download' });
  if (!p) return { success: false, error: '已取消' };
  await invoke('file_export_to_path', { filePath: r.filePath, savePath: p });
  return { success: true };
}
export async function readFileBytes(fileId: string): Promise<{success:boolean;data?:string;error?:string}> { return invoke('file_read_bytes', { fileId }) }

// ─── 资料库：孤文件 + 虚拟树 + 移动 + 文件夹 ──
export async function listGlobalFiles(): Promise<any> { return invoke('file_list_global') }
export async function listAllFiles(): Promise<any> { return invoke('file_list_all') }
export async function fetchFileTree(projectId?: string): Promise<any> { return invoke('file_tree', { projectId: projectId || null }) }
export async function moveFile(data: { fileId: string; fromDomain: string; toDomain: string; toId?: string; toFolderId?: string }): Promise<any> { return invoke('file_move', { data }) }
export async function addFileNoteMapping(fileId: string, noteId: string, fileName?: string): Promise<any> { return invoke('file_add_note_mapping', { fileId, noteId, fileName: fileName || null }) }
export async function removeFileNoteMapping(fileId: string, noteId: string): Promise<any> { return invoke('file_remove_note_mapping', { fileId, noteId }) }
export async function fetchFilesByNote(noteId: string): Promise<any> { return invoke('file_list_by_note', { noteId }) }
export async function addFileTodoMapping(fileId: string, todoId: string, fileName?: string): Promise<any> { return invoke('file_add_todo_mapping', { fileId, todoId, fileName: fileName || null }) }
export async function removeFileTodoMapping(fileId: string, todoId: string): Promise<any> { return invoke('file_remove_todo_mapping', { fileId, todoId }) }
export async function deleteFileFromDomain(fileId: string, entityType: string, entityId: string): Promise<any> { return invoke('file_delete_from_domain', { fileId, entityType, entityId }) }
export async function fetchFilesByTodo(todoId: string): Promise<any> { return invoke('file_list_by_todo', { todoId }) }
export async function moveFileToFolder(fileId: string, folderId?: string|null): Promise<any> { return invoke('file_move_to_folder', { fileId, folderId: folderId || null }) }
export async function createFolder(name: string, parentId?: string|null, projectId?: string|null): Promise<any> { return invoke('file_folder_create', { name, parentId, projectId }) }
export async function renameFolder(id: string, name: string): Promise<any> { return invoke('file_folder_rename', { id, name }) }
export async function moveFolder(id: string, parentId?: string|null): Promise<any> { return invoke('file_folder_move', { id, parentId: parentId || null }) }
export async function reorderFolder(id: string, sortOrder: number): Promise<any> { return invoke('file_folder_reorder', { id, sortOrder }) }
export async function deleteFolder(id: string): Promise<any> { return invoke('file_folder_delete', { id }) }

// Material/product files — 统一走 uploadFile({ entityType: 'product', entityIds: [...] })
export async function listMaterials(productId: string): Promise<any> { return invoke('file_list_by_product', { productId }) }
export async function openMaterial(materialId: string): Promise<any> { return previewFile(materialId) }
export async function downloadMaterial(data: { materialId: string }): Promise<any> { return exportFile(data.materialId) }
export async function deleteMaterial(materialId: string, productId?: string): Promise<any> { return invoke('file_unlink_from_product', { fileId: materialId, productId: productId || '' }) }
export async function deleteAllMaterial(materialId: string): Promise<any> { return invoke('file_delete', { fileId: materialId }) }
export async function getMaterialAssociations(materialId: string): Promise<any> { return invoke('file_get_product_associations', { fileId: materialId }) }
export async function updateMaterialAssociations(materialId: string, productIds: string[], matType?: string): Promise<any> { return invoke('file_update_product_associations', { fileId: materialId, data: { productIds, type: matType||'OTHER' } }) }

// ─── Settings ──────────────────────────────────────────────────
export interface UserProfile { id: string; name: string; username: string; role: string; phone: string; email: string; avatar: string; aiConfig: string }
export async function fetchSettings(): Promise<Record<string, any>> { return invoke('settings_get') }
export async function updateSettings(data: Record<string,any>): Promise<{success: boolean}> { return invoke('settings_update', { data }) }
export async function fetchAiConfig(): Promise<AiConfig> { return invoke('get_ai_config') }
export async function updateAiConfig(data: Record<string,any>): Promise<{success: boolean}> { return invoke('update_ai_config', { data: { ai_config: JSON.stringify(data) } }) }

// ─── User Policies (用户策略/宪法) ───────────────────────────────
export interface UserPolicy { id: number; content: string; created_at: string; is_active: number }
export async function getUserPolicies(): Promise<{success:boolean;policies:UserPolicy[]}> { return invoke('get_user_policies') }
export async function setUserPolicies(policies: string): Promise<{success:boolean}> { return invoke('set_user_policies', { policies }) }
export async function managePolicy(id?: number, content?: string): Promise<{success:boolean;action?:string;message?:string}> { return invoke('manage_policy', { id: id ?? null, content: content ?? null }) }

// ─── Notes ─────────────────────────────────────────────────────
export async function fetchNotes(): Promise<FetchNotesResponse> { return invoke('notes_list_all') }
export async function fetchDailyNotes(): Promise<FetchNotesResponse> { return invoke('notes_list_daily') }
export async function fetchProjectNotes(projectId: string): Promise<FetchNotesResponse> { return invoke('notes_list_by_project', { projectId }) }
export async function fetchOrphanNotes(): Promise<FetchNotesResponse> { return invoke('notes_list_orphan') }
export async function createNote(data: Record<string,any>): Promise<{noteId:string}> { return invoke('notes_create', { data }) }
export async function updateNote(data: Record<string,any>): Promise<void> { return invoke('notes_update', { data }) }
export async function deleteNote(noteId: string): Promise<void> { return invoke('notes_delete', { noteId }) }
export async function rebuildNoteFTS(): Promise<{success:boolean}> { return invoke('notes_rebuild_fts') }
export async function upgradeNoteFTSTriggers(): Promise<{success:boolean}> { return invoke('notes_upgrade_fts_triggers') }
export async function getProjectKnowledgeGraph(projectId: string): Promise<any> { return invoke('notes_get_project_knowledge_graph', { projectId }) }
export async function semanticRecall(data: any): Promise<{results?:any[]}> { return invoke('notes_semantic_recall', { data }) }
export async function countNotesByNotebook(notebookId: string): Promise<{count:number}> { return invoke('notes_count_by_notebook', { notebookId }) }

// Notes v2 aliases
export const notesV2ListAll = fetchNotes
export const notesV2ListDaily = fetchDailyNotes
export const notesV2ListByProject = fetchProjectNotes
export const notesV2ListOrphan = fetchOrphanNotes
export async function notesV2Create(data: any): Promise<{noteId:string}> { return invoke('notes_create', { data }) }
export async function notesV2Update(data: any): Promise<void> { return invoke('notes_update', { data }) }
export async function notesV2Delete(noteId: string): Promise<void> { return invoke('notes_delete', { noteId }) }
export async function notesV2ListTrash(): Promise<FetchNotesResponse> { return invoke('notes_list_trash') }
export async function notesV2ListArchive(): Promise<FetchNotesResponse> { return invoke('notes_list_archive') }
export async function notesV2Restore(noteId: string, notebookId: string): Promise<void> { return invoke('notes_restore', { noteId, notebookId }) }
export async function notesV2PermanentDelete(noteId: string): Promise<void> { return invoke('notes_permanent_delete', { noteId }) }

// ─── Notebooks ─────────────────────────────────────────────────
export async function fetchNotebooks(): Promise<FetchNotebooksResponse> { return invoke('notebook_list') }
export async function createNotebook(data: Record<string,any>): Promise<{notebook?:NotebookRow;id?:string}> { return invoke('notebook_create', { data }) }
export async function updateNotebook(data: Record<string,any>): Promise<void> { return invoke('notebook_update', { data }) }
export async function deleteNotebook(notebookId: string): Promise<void> { return invoke('notebook_delete', { notebookId }) }

// ─── Partners (Unified) ─────────────────────────────────────
export async function fetchPartners(projectId: string, partnerType?: string): Promise<any[]> { return invoke('partner_list', { projectId, partnerType: partnerType || null }) }
export async function createPartner(data: Record<string,any>): Promise<any> { return invoke('partner_create', { data }) }
export async function updatePartner(data: Record<string,any>): Promise<any> { return invoke('partner_update', { data }) }
export async function deletePartner(id: string): Promise<any> { return invoke('partner_delete', { partnerId: id }) }
export async function partnerLinkProject(partnerId: string, projectId: string, contactId?: string, remark?: string): Promise<any> { return invoke('partner_link_project', { partnerId, projectId, contactId: contactId || null, remark: remark || null }) }
export async function partnerUnlinkProject(partnerId: string, projectId: string): Promise<any> { return invoke('partner_unlink_project', { partnerId, projectId }) }
export async function createPartnerContact(data: Record<string,any>): Promise<any> { return invoke('partner_contact_create', { data }) }
export async function updatePartnerContact(data: Record<string,any>): Promise<any> { return invoke('partner_contact_update', { data }) }
export async function deletePartnerContact(id: string): Promise<any> { return invoke('partner_contact_delete', { contactId: id }) }
export async function contactLinkProject(contactId: string, projectId: string): Promise<any> { return invoke('contact_link_project', { contactId, projectId }) }
export async function contactUnlinkProject(contactId: string, projectId: string): Promise<any> { return invoke('contact_unlink_project', { contactId, projectId }) }
export async function partnerUpdateRemark(partnerId: string, projectId: string, remark?: string): Promise<any> { return invoke('partner_update_remark', { partnerId, projectId, remark: remark || null }) }
export async function contactsListAll(partnerType?: string): Promise<any[]> { return invoke('contacts_list_all', { partnerType: partnerType || null }) }

// ─── Partners (Legacy aliases) ──────────────────────────────
export async function fetchChannelPartners(projectId: string): Promise<any[]> { return invoke('channel_list', { projectId }) }
export async function createChannelPartner(data: Record<string,any>): Promise<any> { return invoke('channel_create', { data }) }
export async function updateChannelPartner(data: Record<string,any>): Promise<any> { return invoke('channel_update', { data }) }
export async function deleteChannelPartner(id: string): Promise<any> { return invoke('channel_delete', { channelId: id }) }
export const fetchChannels = fetchChannelPartners; export const createChannel = createChannelPartner; export const updateChannel = updateChannelPartner; export const deleteChannel = deleteChannelPartner
export async function createChannelContact(data: Record<string,any>): Promise<any> { return invoke('channel_contact_create', { data }) }
export async function updateChannelContact(data: Record<string,any>): Promise<any> { return invoke('channel_contact_update', { data }) }
export async function deleteChannelContact(id: string): Promise<any> { return invoke('channel_contact_delete', { contactId: id }) }
export async function channelLinkProject(channelId: string, projectId: string, contactId?: string, remark?: string): Promise<any> { return invoke('channel_link_project', { channelId, projectId, contactId: contactId || null, remark: remark || null }) }
export async function channelUnlinkProject(channelId: string, projectId: string): Promise<any> { return invoke('channel_unlink_project', { channelId, projectId }) }
export async function fetchVendorPartners(projectId: string): Promise<any[]> { return invoke('vendor_list', { projectId }) }
export async function createVendorPartner(data: Record<string,any>): Promise<any> { return invoke('vendor_create', { data }) }
export async function updateVendorPartner(data: Record<string,any>): Promise<any> { return invoke('vendor_update', { data }) }
export async function deleteVendorPartner(id: string): Promise<any> { return invoke('vendor_delete', { vendorId: id }) }
export const fetchVendors = fetchVendorPartners; export const createVendor = createVendorPartner; export const updateVendor = updateVendorPartner; export const deleteVendor = deleteVendorPartner
export async function createVendorContact(data: Record<string,any>): Promise<any> { return invoke('vendor_contact_create', { data }) }
export async function updateVendorContact(data: Record<string,any>): Promise<any> { return invoke('vendor_contact_update', { data }) }
export async function deleteVendorContact(id: string): Promise<any> { return invoke('vendor_contact_delete', { contactId: id }) }
export async function vendorLinkProject(vendorId: string, projectId: string, contactId?: string, remark?: string): Promise<any> { return invoke('vendor_link_project', { vendorId, projectId, contactId: contactId || null, remark: remark || null }) }
export async function vendorUnlinkProject(vendorId: string, projectId: string): Promise<any> { return invoke('vendor_unlink_project', { vendorId, projectId }) }

// ─── Solutions ─────────────────────────────────────────────────
export async function fetchProjectSolutions(projectId: string): Promise<SolutionRow[]> { return invoke('solution_list', { projectId }) }
export async function createSolution(projectId: string, name: string): Promise<SolutionRow> { return invoke('solution_create', { projectId, name }) }
export async function renameSolution(solutionId: string, name: string): Promise<void> { await invoke('solution_rename', { solutionId, name }) }
export async function deleteSolution(solutionId: string): Promise<void> { await invoke('solution_delete', { solutionId }) }
export async function createVersion(solutionId: string, data: Record<string,any>): Promise<VersionRow> { return invoke('solution_version_create', { solutionId, data }) }
export async function finalizeDraftVersion(versionId: string, data: Record<string,any>): Promise<VersionRow> { return invoke('solution_version_finalize', { versionId, data }) }
export async function fetchVersions(solutionId: string): Promise<VersionRow[]> { return invoke('solution_version_list', { solutionId }) }
export const fetchSolutionVersions = fetchVersions
export async function deleteVersion(versionId: string): Promise<void> { await invoke('solution_version_delete', { versionId }) }
export async function updateSolutionVersionMeta(versionId: string, data: Record<string,any>): Promise<void> { await invoke('solution_version_update_meta', { versionId, data }) }
export async function saveDraftVersion(versionId: string, data: Record<string,any>): Promise<VersionRow> { return invoke('solution_version_save_draft', { versionId, data }) }
export async function solutionDownloadExcel(data: {buffer:number[];fileName:string}): Promise<{success:boolean;savedPath?:string;error?:string}> { const { save } = await import('@tauri-apps/plugin-dialog'); const sp = await save({ defaultPath: data.fileName, filters: [{name:'Excel',extensions:['xlsx']}] }); if(!sp) return {success:false,error:'cancelled'}; return invoke('solution_download_excel', { data: { buffer: data.buffer, savePath: sp } }) }

// ─── KB ────────────────────────────────────────────────────────
export interface KbConfig { embeddingApiUrl?: string; embeddingApiKey?: string; embeddingModelName?: string; embeddingActive?: boolean; chatApiUrl?: string; chatApiKey?: string; chatModelName?: string }
export interface KbSearchResult { source_type: string; source_id: string; title: string; chunk_text: string; score: number }
export interface KbDiagnoseResult { success: boolean; embedding_configured: boolean; chat_configured: boolean; lancedb_count: number; fts5_count: number }
export async function kbGetConfig(): Promise<KbConfig> { return invoke('kb_get_config') }
export async function kbUpdateConfig(data: Record<string, any>): Promise<{success:boolean}> { return invoke('kb_update_config', { data }) }
export async function kbImportFile(data: Record<string, any>): Promise<{success:boolean;fileId?:string;error?:string}> { return invoke('kb_import_file', { data }) }
export async function kbSaveNote(data: Record<string, any>): Promise<{success:boolean;noteId?:string}> { return invoke('kb_save_note', { data }) }
export async function kbSearch(data: Record<string, any>): Promise<{success:boolean;results?:KbSearchResult[]}> { return invoke('kb_search', { data }) }
export async function kbDelete(docId: string): Promise<{success:boolean}> { return invoke('kb_delete', { docId }) }
export async function kbListDocuments(projectId?: string): Promise<{success:boolean;documents?:FileRow[]}> { return invoke('kb_list_documents', { projectId: projectId||null }) }
export async function kbChat(data: Record<string, any>): Promise<{success:boolean;answer?:string;error?:string}> { return invoke('kb_chat', { data }) }
export async function kbAgentChat(data: Record<string, any>): Promise<{success:boolean;answer?:string;tool_log?:Array<{tool:string;args:unknown;result:string}>;context_docs?:Array<{title:string;chunk_text:string}>;workspace?:string;error?:string}> { return invoke('kb_agent_chat', { data }) }
export async function agentCancel(): Promise<{success:boolean}> { return invoke('agent_cancel') }
export async function kbDiagnose(): Promise<KbDiagnoseResult> { return invoke('kb_diagnose') }
export async function kbGetTableChunks(fileId: string): Promise<{success:boolean;chunks?:Array<{id:number;chunk_text:string;chunk_index:number}>}> { return invoke('kb_get_table_chunks', { fileId }) }
export async function kbTestEmbedding(data: {url:string;key:string;model:string}): Promise<{success:boolean;message?:string;error?:string}> { return invoke('kb_test_embedding', { data }) }
export async function kbTestChat(data: {url:string;key:string;model:string}): Promise<{success:boolean;message?:string;error?:string}> { return invoke('kb_test_chat', { data }) }

// ─── AI Sync ───────────────────────────────────────────────────
export async function aiSyncStatus(): Promise<any> { return invoke('ai_sync_status') }
export async function aiSyncTrigger(): Promise<{success:boolean}> { return invoke('ai_sync_trigger') }
export async function aiTestProvider(data: Record<string,any>): Promise<{success:boolean;message?:string;error?:string}> { return invoke('ai_test_provider', { data }) }
export async function aiTestEmbedding(data: Record<string,any>): Promise<{success:boolean;message?:string;error?:string}> { return invoke('ai_test_embedding', { data }) }
export async function aiSyncDiagnose(): Promise<any> { return invoke('ai_sync_diagnose') }

// ─── KB Debug & Agent Tools ─────────────────────────────────────
export async function kbSearchDebug(query: string): Promise<any> { return invoke('kb_search_debug', { query }) }
export async function kbDataBrowse(query?: string, page?: number): Promise<any> { return invoke('kb_data_browse', { query: query || null, page: page || null }) }
export async function kbInspectFts5(query?: string, page?: number): Promise<any> { return invoke('kb_inspect_fts5', { query: query || null, page: page || 1 }) }
export async function kbFts5Inspector(): Promise<any> { return invoke('kb_fts5_inspector') }
export async function executeProductionRag(data: any): Promise<any> { return invoke('execute_production_rag', { data }) }
export async function extToolSearchKnowledgeBase(keywords: string[], projectId?: string, filterTags?: string[]): Promise<string> { return invoke('ext_tool_search_knowledge_base', { keywords, projectId: projectId || null, filterTags: filterTags || null }) }
export async function extToolGetDocumentPage(docId: string, page: number): Promise<any> { return invoke('ext_tool_get_document_page', { docId, page }) }
export async function extToolFindRelatedKnowledge(docId: string): Promise<any> { return invoke('ext_tool_find_related_knowledge', { docId }) }
export async function extToolGetProjectContext(projectId: string): Promise<any> { return invoke('ext_tool_get_project_context', { projectId }) }
export async function cleanupRetryFailed(): Promise<any> { return invoke('cleanup_retry_failed') }
export async function cleanupOrphanData(): Promise<{success:boolean;message?:string}> { return invoke('cleanup_orphan_data') }
export async function knowledgePipelineStatus(): Promise<any> { return invoke('knowledge_pipeline_status') }
export async function pipelineDebugStatus(): Promise<any> { return invoke('pipeline_debug_status') }
export async function pipelineClearKnowledge(): Promise<any> { return invoke('pipeline_clear_knowledge') }
export async function pipelineClearVectors(): Promise<any> { return invoke('pipeline_clear_vectors') }
export async function pipelineClearAll(): Promise<any> { return invoke('pipeline_clear_all') }
export async function pipelineResync(): Promise<any> { return invoke('pipeline_resync') }
export async function pipelineBrowseSource(sourceType?: string, limit?: number): Promise<any> { return invoke('pipeline_browse_source', { sourceType: sourceType || null, limit: limit || 50 }) }
export async function pipelineGetSourceContent(id: number): Promise<any> { return invoke('pipeline_get_source_content', { id }) }
export async function kbSaveSessions(sessions: any): Promise<any> { return invoke('kb_save_sessions', { data: { sessions } }) }
export async function kbLoadSessions(): Promise<any> { return invoke('kb_load_sessions') }
export async function kbDeleteSession(sessionId: string): Promise<any> { return invoke('kb_delete_session', { sessionId }) }

// ── Memory Explorer API ──
export async function kbMemoryObjects(query: string): Promise<{success:boolean;objects:Array<{source_type:string;source_id:string;title:string}>}> { return invoke('kb_memory_objects', { query }) }
export async function searchBusiness(query: string): Promise<{success:boolean;objects:Array<{source_type:string;source_id:string;title:string}>}> { return invoke('search_business', { query }) }
export async function searchSourceData(query: string): Promise<{success:boolean;objects:Array<{source_type:string;source_id:string}>}> { return invoke('search_source_data', { query }) }
export async function lookupObjectNames(ids: Array<{source_type:string;source_id:string}>): Promise<{success:boolean;names:Record<string,string>}> { return invoke('lookup_object_names', { ids }) }
export async function kbMemoryParents(sourceType: string, sourceId: string): Promise<{success:boolean;parents:Array<any>}> { return invoke('kb_memory_parents', { sourceType, sourceId }) }
export async function kbMemoryEntityRelations(entityName?: string, sourceType?: string, sourceId?: string): Promise<{success:boolean;entities:Array<any>;related_objects:Array<any>}> { return invoke('kb_memory_entity_relations', { entityName: entityName || '', sourceType: sourceType || '', sourceId: sourceId || '' }) }
export async function kbMemoryHeatmap(): Promise<{success:boolean;total_chunks:number;hot:Array<any>;cold:Array<any>}> { return invoke('kb_memory_heatmap') }
export async function kbDashboardStats(): Promise<any> { return invoke('kb_memory_dashboard_stats') }
export async function kbProjectNoteIds(): Promise<{ids:string[]}> { return invoke('kb_project_note_ids') }
export async function kbBrowseParents(query?: string, page?: number): Promise<any> { return invoke('kb_browse_parents', { query: query || null, page: page || null }) }
export async function kbBrowseVectors(limit?: number): Promise<any> { return invoke('kb_browse_vectors', { limit: limit || 50 }) }

// ─── Assets ───────────────────────────────────────────────────
export async function assetsSearch(data: any): Promise<any> { return invoke('assets_search', { data }) }
export async function assetsSearchFts5(data: any): Promise<any> { return invoke('assets_search_fts5', { data }) }
export async function assetsList(): Promise<any> { return invoke('assets_list') }
export async function assetSyncStatus(): Promise<any> { return invoke('asset_sync_status') }
export async function assetSyncTrigger(fileId: string): Promise<any> { return invoke('asset_sync_trigger', { fileId }) }
export async function assetDiagnose(): Promise<any> { return invoke('asset_diagnose') }
export async function assetGetLatestVersion(fileId: string): Promise<any> { return invoke('asset_get_latest_version', { fileId }) }
export async function assetGetParsedText(fileId: string): Promise<any> { return invoke('asset_get_parsed_text', { fileId }) }
export async function assetRerunMigration(): Promise<any> { return invoke('asset_rerun_migration') }

// ─── Storage ───────────────────────────────────────────────────
export async function getStorageConfig(): Promise<any> { return invoke('storage_get_config') }

// ─── Contacts (通讯录) ────────────────────────────────────────
export async function contactsListChannels(): Promise<any[]> { return invoke('contacts_list_channels') }
export async function contactsListVendors(): Promise<any[]> { return invoke('contacts_list_vendors') }
export async function channelCreate(data: Record<string,any>): Promise<any> { return invoke('channel_create', { data }) }
export async function channelUpdate(data: Record<string,any>): Promise<any> { return invoke('channel_update', { data }) }
export async function channelDelete(channelId: string): Promise<any> { return invoke('channel_delete', { channelId }) }
export async function channelContactCreate(data: Record<string,any>): Promise<any> { return invoke('channel_contact_create', { data }) }
export async function channelContactUpdate(data: Record<string,any>): Promise<any> { return invoke('channel_contact_update', { data }) }
export async function channelContactDelete(contactId: string): Promise<any> { return invoke('channel_contact_delete', { contactId }) }
export async function vendorCreate(data: Record<string,any>): Promise<any> { return invoke('vendor_create', { data }) }
export async function vendorUpdate(data: Record<string,any>): Promise<any> { return invoke('vendor_update', { data }) }
export async function vendorDelete(vendorId: string): Promise<any> { return invoke('vendor_delete', { vendorId }) }
export async function vendorContactCreate(data: Record<string,any>): Promise<any> { return invoke('vendor_contact_create', { data }) }
export async function vendorContactUpdate(data: Record<string,any>): Promise<any> { return invoke('vendor_contact_update', { data }) }
export async function vendorContactDelete(contactId: string): Promise<any> { return invoke('vendor_contact_delete', { contactId }) }
export async function customerList(): Promise<any[]> { return invoke('customer_list') }
export async function customerCreate(data: Record<string,any>): Promise<any> { return invoke('customer_create', { data }) }
export async function customerUpdate(data: Record<string,any>): Promise<any> { return invoke('customer_update', { data }) }
export async function customerDelete(customerId: string): Promise<any> { return invoke('customer_delete', { customerId }) }
export async function customerContactCreate(data: Record<string,any>): Promise<any> { return invoke('customer_contact_create', { data }) }
export async function customerContactUpdate(data: Record<string,any>): Promise<any> { return invoke('customer_contact_update', { data }) }
export async function customerContactDelete(contactId: string): Promise<any> { return invoke('customer_contact_delete', { contactId }) }
export async function customerLinkProject(customerId: string, projectId: string): Promise<any> { return invoke('customer_link_project', { customerId, projectId }) }
export async function customerUnlinkProject(customerId: string, projectId: string): Promise<any> { return invoke('customer_unlink_project', { customerId, projectId }) }
export async function updateStorageConfig(data: any): Promise<any> { return invoke('storage_update_config', { data }) }
export async function testOssConnection(): Promise<any> { return invoke('oss_test_connection') }
export async function getDefaultStoragePath(): Promise<string> { return invoke('storage_default_path') }
export async function migrateFiles(oldPath: string, newPath: string): Promise<any> { return invoke('file_migrate_files', { oldPath, newPath }) }

export async function fetchMigrationProgress(): Promise<any> { return invoke('storage_migrate_progress') }

// ─── Export ────────────────────────────────────────────────────
export async function exportAllData(domains: string[], savePath: string): Promise<{success:boolean;error?:string}> { return invoke('export_data', { domains, savePath }) }
export async function importData(filePath: string): Promise<any> { return invoke('import_data', { filePath }) }
export async function productsExport(): Promise<any> { return invoke('products_export') }
export async function productsImport(data: any): Promise<any> { return invoke('products_import', { data }) }
export async function productsExportFile(): Promise<any> { return invoke('products_export_file') }
export async function productsImportFile(filePath: string): Promise<any> { return invoke('products_import_file', { filePath }) }

// ─── Shell & Dialog ────────────────────────────────────────────
export async function revealFile(filePath: string): Promise<{success:boolean}> { try { const { open } = await import('@tauri-apps/plugin-shell'); await open(`file:///${filePath.replace(/\\/g,'/')}`); return {success:true} } catch { return {success:false} } }
export async function openFileDialog(filters?: {name:string;extensions:string[]}[]): Promise<{canceled:boolean;path:string}> { try { const { open } = await import('@tauri-apps/plugin-dialog'); const r = await open({ filters: filters?.map(f=>({name:f.name,extensions:f.extensions})), multiple: false }); if(!r||typeof r!=='string') return {canceled:true,path:''}; return {canceled:false,path:r} } catch { return {canceled:true,path:''} } }
export async function browseFolder(): Promise<{canceled:boolean;path:string}> { try { const { open } = await import('@tauri-apps/plugin-dialog'); const r = await open({ directory: true, multiple: false }); if(!r||typeof r!=='string') return {canceled:true,path:''}; return {canceled:false,path:r} } catch { return {canceled:true,path:''} } }

// ─── Todos ──────────────────────────────────────────────────────
export async function fetchTodos(): Promise<{todos?:TodoRow[]}> { return invoke('todos_list') }
export async function createTodo(data: Record<string, any>): Promise<{todo?:TodoRow}> { return invoke('todos_create', { data }) }
export async function updateTodo(id: string, data: Record<string, any>): Promise<{todo?:TodoRow}> { return invoke('todos_update', { id, data }) }
export async function deleteTodo(id: string): Promise<void> { return invoke('todos_delete', { id }) }
export async function toggleTodo(id: string): Promise<{todo?:TodoRow;repeat_advanced?:boolean}> { return invoke('todos_toggle', { id }) }
export async function getTodo(id: string): Promise<{todo?:TodoRow}> { return invoke('todos_get', { id }) }

// OSS
export async function ossTestConnection(): Promise<{success:boolean;error?:string}> { return invoke('oss_test_connection') }
export async function ossRawDiagnose(): Promise<{success:boolean;detail:string}> { return invoke('oss_raw_diagnose') }

// Project import/export
export async function projectExport(projectId: string): Promise<any> { return invoke('project_export', { projectId }) }
export async function batchExportProjects(projectIds: string[], filePath: string): Promise<any> { return invoke('projects_batch_export', { projectIds, filePath }) }

// Pipeline Debug
export async function kbPipelineDebug(): Promise<{success:boolean;stats:any;sync_queue:any[];cleanup_queue:any[]}> { return invoke('kb_pipeline_debug') }
