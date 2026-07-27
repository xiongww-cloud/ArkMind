# 多窗口聊天功能实现方案

## 一、需求概述

将现有的单窗口聊天系统改造为支持多窗口聊天，每个聊天会话独立成窗口，支持拖拽、置顶、缩放等操作。

---

## 二、技术架构设计

### 2.1 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                      Electron 主进程                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ ChatWindow   │  │ ChatWindow   │  │ ChatWindow   │       │
│  │ Manager      │  │ Instance 1   │  │ Instance 2   │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
└─────────────────────────────────────────────────────────────┘
          │                │                  │
          ▼                ▼                  ▼
┌─────────────────────────────────────────────────────────────┐
│                    IPC 通信层                                │
│  • 创建/销毁窗口    • 消息同步    • 状态同步               │
└─────────────────────────────────────────────────────────────┘
          │                │                  │
          ▼                ▼                  ▼
┌─────────────────────────────────────────────────────────────┐
│                    渲染进程                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ 主窗口       │  │ 聊天窗口1    │  │ 聊天窗口2    │       │
│  │ (会话列表)   │  │ (聊天界面)   │  │ (聊天界面)   │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
└─────────────────────────────────────────────────────────────┘
```

### 2.2 窗口类型

| 窗口类型 | 说明 | 特性 |
|---------|------|------|
| 主窗口 | 显示会话列表、联系人等 | 唯一，不可关闭 |
| 聊天窗口 | 独立的聊天界面 | 多个，可拖拽、置顶、缩放 |

---

## 三、详细实现方案

### 3.1 主进程：ChatWindowManager

**文件**: `src/main/windows/ChatWindowManager.js`

```javascript
class ChatWindowManager {
  constructor() {
    this.windows = new Map() // sessionId -> BrowserWindow
  }
  
  // 创建聊天窗口
  createChatWindow(sessionId, options = {}) {
    if (this.windows.has(sessionId)) {
      this.windows.get(sessionId).focus()
      return
    }
    
    const window = new BrowserWindow({
      width: 400,
      height: 600,
      frame: false,
      transparent: true,
      alwaysOnTop: options.alwaysOnTop || false,
      // ...其他配置
    })
    
    this.windows.set(sessionId, window)
    window.loadURL(`file://${__dirname}/chat.html?sessionId=${sessionId}`)
    
    window.on('closed', () => {
      this.windows.delete(sessionId)
    })
  }
  
  // 销毁聊天窗口
  destroyChatWindow(sessionId) {
    const window = this.windows.get(sessionId)
    if (window) {
      window.destroy()
      this.windows.delete(sessionId)
    }
  }
  
  // 获取窗口
  getWindow(sessionId) {
    return this.windows.get(sessionId)
  }
}
```

### 3.2 IPC 通信协议

**文件**: `src/main/ipc/chatWindowIPC.js`

| 事件名 | 方向 | 说明 |
|--------|------|------|
| `chat-window:create` | 渲染→主 | 创建聊天窗口 |
| `chat-window:close` | 渲染→主 | 关闭聊天窗口 |
| `chat-window:minimize` | 渲染→主 | 最小化窗口 |
| `chat-window:maximize` | 渲染→主 | 最大化窗口 |
| `chat-window:toggle-pin` | 渲染→主 | 切换置顶 |
| `chat-window:message` | 主→渲染 | 新消息通知 |
| `chat-window:sync-state` | 主→渲染 | 状态同步 |

### 3.3 渲染进程：聊天窗口组件

**文件**: `src/renderer/components/ChatWindow.vue`

```vue
<template>
  <div class="chat-window">
    <!-- 标题栏 -->
    <div class="titlebar" @mousedown="startDrag">
      <span class="title">{{ sessionName }}</span>
      <div class="controls">
        <button @click="togglePin">📌</button>
        <button @click="minimize">—</button>
        <button @click="maximize">□</button>
        <button @click="close">×</button>
      </div>
    </div>
    
    <!-- 消息列表 -->
    <div class="messages" ref="messageList">
      <div v-for="msg in messages" :key="msg.id" class="message">
        <!-- 消息内容 -->
      </div>
    </div>
    
    <!-- 输入区域 -->
    <div class="input-area">
      <textarea v-model="inputText" @keyup.enter="send"></textarea>
      <button @click="send">发送</button>
    </div>
  </div>
</template>
```

### 3.4 数据流设计

```
发送消息流程:
1. 用户输入 → 2. 渲染进程发送IPC → 3. 主进程处理 → 4. 存储消息 → 5. 通知相关窗口

接收消息流程:
1. 主进程收到消息 → 2. 查找目标窗口 → 3. 发送IPC通知 → 4. 渲染进程更新UI
```

---

## 四、关键功能实现

### 4.1 窗口拖拽

```javascript
// 渲染进程
const { ipcRenderer } = require('electron')

let isDragging = false
let dragStartPos = { x: 0, y: 0 }

function startDrag(e) {
  isDragging = true
  dragStartPos = { x: e.screenX, y: e.screenY }
  
  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
}

function onDrag(e) {
  if (!isDragging) return
  
  const deltaX = e.screenX - dragStartPos.x
  const deltaY = e.screenY - dragStartPos.y
  
  ipcRenderer.send('chat-window:move', { deltaX, deltaY })
}

function stopDrag() {
  isDragging = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
}
```

### 4.2 窗口置顶

```javascript
// 主进程
ipcMain.on('chat-window:toggle-pin', (event, sessionId) => {
  const window = chatWindowManager.getWindow(sessionId)
  if (window) {
    const isPinned = window.isAlwaysOnTop()
    window.setAlwaysOnTop(!isPinned)
    event.reply('chat-window:pin-changed', !isPinned)
  }
})
```

### 4.3 消息同步

```javascript
// 主进程：广播消息到相关窗口
function broadcastMessage(message, excludeWindowId = null) {
  const windows = BrowserWindow.getAllWindows()
  windows.forEach(window => {
    if (window.id !== excludeWindowId) {
      window.webContents.send('chat-window:message', message)
    }
  })
}
```

### 4.4 窗口布局管理

```javascript
// 窗口布局配置
const windowLayouts = {
  cascade: {
    offsetX: 30,
    offsetY: 30,
    maxWidth: 5,
  },
  grid: {
    columns: 3,
    spacing: 10,
  },
  stack: {
    direction: 'horizontal',
    spacing: 5,
  }
}

function arrangeWindows(layout) {
  const windows = Array.from(chatWindowManager.windows.values())
  const config = windowLayouts[layout]
  
  // 根据布局配置排列窗口
}
```

---

## 五、UI/UX 设计

### 5.1 窗口外观

```
┌─────────────────────────────────────┐
│  ┌─┐                               │
│  │●│ 联系人名称          📌 ─ □ × │
│  └─┘                               │
├─────────────────────────────────────┤
│                                     │
│         消息内容区域                 │
│                                     │
│                                     │
│                                     │
├─────────────────────────────────────┤
│  [输入消息...]              [发送]  │
└─────────────────────────────────────┘
```

### 5.2 交互特性

- **拖拽**: 标题栏可拖拽移动窗口
- **置顶**: 点击📌图标切换窗口置顶状态
- **缩放**: 窗口边缘可拖拽调整大小
- **最小化**: 点击—最小化到任务栏
- **最大化**: 点击□最大化/还原
- **关闭**: 点击×关闭窗口（不退出应用）

---

## 六、代码组织

### 6.1 文件结构

```
src/
├── main/
│   ├── windows/
│   │   ├── ChatWindowManager.js      # 窗口管理器
│   │   └── ChatWindow.js             # 单个窗口类
│   ├── ipc/
│   │   └── chatWindowIPC.js          # IPC通信处理
│   └── main.js                       # 主入口
├── renderer/
│   ├── components/
│   │   ├── ChatWindow/
│   │   │   ├── index.vue             # 主组件
│   │   │   ├── TitleBar.vue          # 标题栏
│   │   │   ├── MessageList.vue       # 消息列表
│   │   │   └── InputArea.vue         # 输入区域
│   │   └── ...
│   ├── pages/
│   │   └── chat/
│   │       └── index.vue             # 聊天页面入口
│   └── App.vue
└── shared/
    └── constants.js                  # 共享常量
```

### 6.2 状态管理

```javascript
// store/modules/chatWindow.js
const state = {
  windows: {},      // sessionId -> windowState
  activeWindow: null // 当前活动窗口
}

const mutations = {
  ADD_WINDOW(state, { sessionId, windowState }) {
    state.windows[sessionId] = windowState
  },
  REMOVE_WINDOW(state, sessionId) {
    delete state.windows[sessionId]
  },
  UPDATE_WINDOW_STATE(state, { sessionId, updates }) {
    if (state.windows[sessionId]) {
      Object.assign(state.windows[sessionId], updates)
    }
  }
}
```

---

## 七、实现步骤

### 阶段1: 基础架构 (2-3天)
1. 创建 ChatWindowManager 类
2. 实现 IPC 通信协议
3. 创建基础聊天窗口组件

### 阶段2: 核心功能 (3-4天)
1. 实现窗口拖拽
2. 实现窗口置顶
3. 实现消息收发
4. 实现窗口状态同步

### 阶段3: UI完善 (2-3天)
1. 优化窗口外观
2. 添加动画效果
3. 实现窗口布局管理

### 阶段4: 测试优化 (2天)
1. 功能测试
2. 性能优化
3. 边界情况处理

---

## 八、注意事项

### 8.1 性能考虑
- 限制同时打开的窗口数量（建议最多10个）
- 使用虚拟滚动优化长消息列表
- 窗口不可见时暂停渲染

### 8.2 内存管理
- 窗口关闭时及时释放资源
- 避免内存泄漏
- 定期清理无用数据

### 8.3 用户体验
- 窗口打开/关闭动画
- 消息提示音
- 未读消息角标
- 窗口位置记忆

### 8.4 兼容性
- Windows/macOS/Linux 平台适配
- 高DPI屏幕支持
- 快捷键支持

---

## 九、扩展功能

### 9.1 窗口分组
- 支持将多个聊天窗口分组
- 分组可以整体移动、最小化

### 9.2 窗口布局预设
- 提供多种布局预设（平铺、层叠、堆叠）
- 支持自定义布局

### 9.3 窗口快捷键
- Ctrl+N: 新建聊天窗口
- Ctrl+W: 关闭当前窗口
- Ctrl+Tab: 切换窗口
- Ctrl+数字: 切换到第N个窗口

### 9.4 消息同步策略
- 离线消息队列
- 消息撤回同步
- 已读状态同步

---

## 十、风险评估

| 风险项 | 影响 | 应对措施 |
|--------|------|---------|
| 内存占用过高 | 高 | 限制窗口数量，优化渲染 |
| IPC通信延迟 | 中 | 使用异步通信，优化数据结构 |
| 窗口状态不一致 | 中 | 完善状态同步机制 |
| 跨平台兼容性 | 低 | 充分测试，平台适配 |

---

## 总结

本方案采用 Electron 多窗口架构，通过 ChatWindowManager 统一管理所有聊天窗口，使用 IPC 通信实现窗口间数据同步。方案具有良好的扩展性和维护性，能够满足多窗口聊天的核心需求。
