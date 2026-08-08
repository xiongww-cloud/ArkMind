import { ref, watch } from 'vue'
import type { Ref } from 'vue'

export function useTiptapToolbar(editor: Ref<any>) {
  const headingLabel = ref('正文')
  const headingOpen = ref(false)
  const listOpen = ref(false)
  const alignOpen = ref(false)
  const alignLabel = ref('左对齐')
  const tableMenuOpen = ref(false)
  const tableRows = ref(3)
  const tableCols = ref(3)
  const atOpen = ref(false)

  // Watch editor selection changes to update labels
  watch(() => editor.value, (ed) => {
    if (!ed) return
    ed.on('selectionUpdate', ({ editor: e }: any) => {
      if (e.isActive('heading', { level: 1 })) headingLabel.value = '标题 1'
      else if (e.isActive('heading', { level: 2 })) headingLabel.value = '标题 2'
      else if (e.isActive('heading', { level: 3 })) headingLabel.value = '标题 3'
      else headingLabel.value = '正文'

      if (e.isActive({ textAlign: 'right' })) alignLabel.value = '右对齐'
      else if (e.isActive({ textAlign: 'center' })) alignLabel.value = '居中'
      else alignLabel.value = '左对齐'
    })
  }, { immediate: true })

  function execFormat(cmd: string, val?: string) {
    if (!editor.value) return
    const ed = editor.value.chain().focus()
    switch (cmd) {
      case 'bold': ed.toggleBold().run(); break
      case 'italic': ed.toggleItalic().run(); break
      case 'underline': ed.toggleUnderline().run(); break
      case 'strike': case 'strikeThrough': ed.toggleStrike().run(); break
      case 'justifyLeft': case 'alignLeft': ed.setTextAlign('left').run(); break
      case 'justifyCenter': case 'alignCenter': ed.setTextAlign('center').run(); break
      case 'justifyRight': case 'alignRight': ed.setTextAlign('right').run(); break
      case 'insertHorizontalRule': case 'horizontalRule': ed.setHorizontalRule().run(); break
      case 'formatBlock': if (val === 'p') ed.setParagraph().run(); break
      case 'bulletList': ed.toggleBulletList().run(); break
      case 'orderedList': ed.toggleOrderedList().run(); break
      case 'taskList': ed.toggleTaskList().run(); break
      case 'codeBlock': ed.toggleCodeBlock().run(); break
      case 'blockquote': ed.toggleBlockquote().run(); break
      case 'heading': if (val) ed.toggleHeading({ level: Number(val) as 1 | 2 | 3 }).run(); break
      case 'align': if (val) ed.setTextAlign(val as any).run(); break
      case 'insertTable': ed.insertTable({ rows: tableRows.value, cols: tableCols.value, withHeaderRow: true }).run(); tableMenuOpen.value = false; break
      case 'addColumnBefore': ed.addColumnBefore().run(); break
      case 'addColumnAfter': ed.addColumnAfter().run(); break
      case 'deleteColumn': ed.deleteColumn().run(); break
      case 'addRowBefore': ed.addRowBefore().run(); break
      case 'addRowAfter': ed.addRowAfter().run(); break
      case 'deleteRow': ed.deleteRow().run(); break
      case 'deleteTable': ed.deleteTable().run(); tableMenuOpen.value = false; break
    }
  }

  function insertHeading(level: number) {
    editor.value?.chain().focus().toggleHeading({ level: level as 1 | 2 | 3 }).run()
    headingOpen.value = false
  }

  function insertList(type: 'ul' | 'ol') {
    if (!editor.value) return
    type === 'ul'
      ? editor.value.chain().focus().toggleBulletList().run()
      : editor.value.chain().focus().toggleOrderedList().run()
  }

  function toggleTaskList() {
    editor.value?.chain().focus().toggleTaskList().run()
    listOpen.value = false
  }

  function insertBlock(tag: string) {
    if (!editor.value) return
    if (tag === 'pre') editor.value.chain().focus().toggleCodeBlock().run()
    else if (tag === 'blockquote') editor.value.chain().focus().toggleBlockquote().run()
  }

  function execAlign(dir: 'left' | 'center' | 'right') {
    editor.value?.chain().focus().setTextAlign(dir).run()
    alignLabel.value = dir === 'right' ? '右对齐' : dir === 'center' ? '居中' : '左对齐'
    alignOpen.value = false
  }

  function insertTableCustom() {
    const r = Math.max(1, Math.min(10, tableRows.value))
    const c = Math.max(1, Math.min(8, tableCols.value))
    editor.value?.chain().focus().insertTable({ rows: r, cols: c, withHeaderRow: true }).run()
    tableMenuOpen.value = false
  }

  function insertTime() {
    if (!editor.value) return
    const now = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
    editor.value.chain().focus().insertContent({
      type: 'timeTag',
      content: [{ type: 'text', text: now }],
    }).insertContent(' ').run()
  }

  function closePopups() {
    headingOpen.value = false
    listOpen.value = false
    alignOpen.value = false
    tableMenuOpen.value = false
    atOpen.value = false
  }

  function togglePopup(name: string) {
    const wasOpen = name === 'heading' ? headingOpen.value
      : name === 'list' ? listOpen.value
      : name === 'align' ? alignOpen.value
      : name === 'table' ? tableMenuOpen.value
      : name === 'at' ? atOpen.value
      : false
    closePopups()
    if (!wasOpen) {
      if (name === 'heading') headingOpen.value = true
      else if (name === 'list') listOpen.value = true
      else if (name === 'align') alignOpen.value = true
      else if (name === 'table') tableMenuOpen.value = true
      else if (name === 'at') atOpen.value = true
    }
  }

  return {
    headingLabel,
    headingOpen,
    listOpen,
    alignOpen,
    alignLabel,
    tableMenuOpen,
    tableRows,
    tableCols,
    atOpen,
    execFormat,
    insertHeading,
    insertList,
    toggleTaskList,
    insertBlock,
    execAlign,
    insertTableCustom,
    insertTime,
    closePopups,
    togglePopup
  }
}
