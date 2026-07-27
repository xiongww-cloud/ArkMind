# TASKS - ArkMind

## 最后更新: 2026-07-26

---

## 桌面精灵窗口 (Sprite Window)

### 已完成
- [x] Tauri配置：精灵窗口透明无边框 (tauri.conf.json)
- [x] 数据库：精灵位置持久化 (sprite_position字段)
- [x] 数据库：精灵开关状态 (sprite_enabled字段)
- [x] Rust后端：精灵窗口管理命令 (create/toggle/close)
- [x] Rust后端：精灵位置读写命令 (save/get_sprite_position)
- [x] Rust后端：精灵开关读写命令 (get/set_sprite_enabled)
- [x] Rust后端：启动时根据开关显示精灵
- [x] Rust后端：托盘菜单切换开关状态
- [x] 前端：SpriteWindow.vue 组件 (独立精灵窗口)
- [x] 前端：拖拽和点击分离 (Pointer Events)
- [x] 前端：位置自动保存 (onMoved事件)
- [x] 前端：设置页面添加精灵开关
- [x] 前端：聊天窗口组件 (ChatWindow.vue)
- [x] 前端：Vite多入口配置 (main/sprite/chat)

### 文件变更清单
```
src-tauri/
├── tauri.conf.json                    # 精灵窗口配置
├── capabilities/desktop.json          # 窗口权限
├── src/
│   ├── lib.rs                         # 注册命令、托盘菜单、启动逻辑
│   ├── commands/
│   │   ├── window_commands.rs         # 窗口管理命令
│   │   └── settings_commands.rs       # 精灵位置/开关命令
│   └── database/
│       ├── db_schema.rs               # 表结构 (sprite_position, sprite_enabled)
│       └── db_migrations.rs           # 迁移

src/renderer/
├── sprite.html                        # 精灵窗口入口
├── chat.html                          # 聊天窗口入口
├── vite.config.ts                     # 多入口配置
├── src/
│   ├── sprite.ts                      # 精灵窗口JS入口
│   ├── chat.ts                        # 聊天窗口JS入口
│   ├── components/
│   │   ├── sprite/SpriteWindow.vue    # 精灵窗口组件
│   │   ├── chat/ChatWindow.vue        # 聊天窗口组件
│   │   └── common/MascotSprite.vue    # 精灵SVG组件
│   ├── api/db-client.ts               # Sprite API函数
│   └── views/Settings.vue             # 精灵开关设置
```

---

## 下一步计划

### Phase 2: 精灵窗口增强
- [ ] 精灵右键菜单
- [ ] 精灵状态显示（思考中、通知等）
- [ ] 精灵动画优化

### Phase 3: 聊天窗口增强
- [ ] 聊天窗口会话管理
- [ ] 聊天窗口历史记录
- [ ] 聊天窗口设置

### Phase 4: 桌面智能助手
- [ ] 语音交互
- [ ] 屏幕感知
- [ ] 主动建议
