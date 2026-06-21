import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export interface FileVersion {
  version: string
  uploadDate: string
  uploadedBy: string
  note: string
  fileUrl: string
}

export interface FileItem {
  id: string
  projectId: string
  fileName: string
  fileType: string
  currentVersion: string
  history: FileVersion[]
}

export const useFileStore = defineStore('file', () => {
  const files = ref<FileItem[]>([])

  const getFilesByProject = computed(() => (projectId: string) =>
    files.value.filter(f => f.projectId === projectId)
  )

  const getFileById = computed(() => (fileId: string) =>
    files.value.find(f => f.id === fileId)
  )

  const getVersionHistory = computed(() => (fileId: string) => {
    const file = files.value.find(f => f.id === fileId)
    return file ? file.history : []
  })

  function addFile(projectId: string, fileName: string, fileType: string, note: string, uploadedBy = 'Current User'): string {
    const id = `FILE-${String(files.value.length + 1).padStart(3, '0')}`
    files.value.push({
      id,
      projectId,
      fileName,
      fileType,
      currentVersion: 'V1.0',
      history: [
        {
          version: 'V1.0',
          uploadDate: new Date().toISOString().split('T')[0],
          uploadedBy,
          note,
          fileUrl: `/mocks/files/${fileName}`
        }
      ]
    })
    return id
  }

  function addVersion(fileId: string, newVersion: string, note: string, uploadedBy = 'Current User') {
    const file = files.value.find(f => f.id === fileId)
    if (file) {
      file.currentVersion = newVersion
      file.history.push({
        version: newVersion,
        uploadDate: new Date().toISOString().split('T')[0],
        uploadedBy,
        note,
        fileUrl: `/mocks/files/${file.fileName.replace(/v\d/i, newVersion.toLowerCase())}`
      })
    }
  }

  function deleteFile(fileId: string) {
    const idx = files.value.findIndex(f => f.id === fileId)
    if (idx !== -1) {
      files.value.splice(idx, 1)
    }
  }

  return {
    files,
    getFilesByProject,
    getFileById,
    getVersionHistory,
    addFile,
    addVersion,
    deleteFile
  }
})
