import { Node, mergeAttributes } from '@tiptap/core'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import ProjectNode from './ProjectNode.vue'
import FileNode from './FileNode.vue'

export const ProjectExtension = Node.create({
  name: 'projectMention',
  group: 'inline',
  inline: true,
  selectable: true,
  atom: true,
  addAttributes() {
    return {
      projectId: { default: null },
      name: { default: '未知项目' },
    }
  },
  parseHTML() {
    return [{ tag: 'span[data-type="project-link"]' }]
  },
  renderHTML({ HTMLAttributes }) {
    // Fallback: server-side or non-Vue render
    return ['span', mergeAttributes(
      { class: 'project-link', 'data-type': 'project-link' },
      HTMLAttributes
    ), ['span', { class: 'link-text' }, HTMLAttributes.name || '']]
  },
  addNodeView() {
    return VueNodeViewRenderer(ProjectNode)
  },
})

export const FileExtension = Node.create({
  name: 'fileMention',
  group: 'inline',
  inline: true,
  selectable: true,
  atom: true,
  addAttributes() {
    return {
      fileId: { default: null },
      name: { default: '未知文件' },
    }
  },
  parseHTML() {
    return [{ tag: 'span[data-type="file-link"]' }]
  },
  renderHTML({ HTMLAttributes }) {
    return ['span', mergeAttributes(
      { class: 'file-link', 'data-type': 'file-link' },
      HTMLAttributes
    ), ['span', { class: 'link-text' }, HTMLAttributes.name || '']]
  },
  addNodeView() {
    return VueNodeViewRenderer(FileNode)
  },
})
