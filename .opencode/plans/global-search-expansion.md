# 全局搜索功能扩展计划

## 目标
扩展 Header 栏全局搜索功能，覆盖联系人、渠道商、供应商、客户、邮件五种业务对象。

## 用户需求
1. 合作伙伴类型分开显示（渠道商、供应商、客户分别显示）
2. 邮件需要搜索正文
3. 联系人只搜索姓名

---

## 修改清单

### 1. 后端：`src-tauri/src/commands/kb/kb_memory_commands.rs`

#### 1.1 `search_business()` 函数（第96-153行）

在 todos 查询之后添加：

```rust
// partners (渠道商/供应商/客户)
if let Ok(rows) = database::all("SELECT id, name, type FROM partners WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
    for r in &rows {
        let id = r["id"].as_str().unwrap_or("").to_string();
        let name = r["name"].as_str().unwrap_or("").to_string();
        let ptype = r["type"].as_str().unwrap_or("").to_string();
        let source_type = match ptype.as_str() {
            "CHANNEL" => "CHANNEL",
            "VENDOR" => "VENDOR",
            "CUSTOMER" => "CUSTOMER",
            _ => continue,
        };
        if seen.insert(format!("{}-{}", source_type, id)) {
            objects.push(json!({"source_type":source_type,"source_id":id,"title":name}));
        }
    }
}
// partner_contacts (联系人)
if let Ok(rows) = database::all("SELECT id, name FROM partner_contacts WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
    for r in &rows {
        let id = r["id"].as_str().unwrap_or("").to_string();
        let name = r["name"].as_str().unwrap_or("").to_string();
        if seen.insert(format!("CONTACT-{}", id)) {
            objects.push(json!({"source_type":"CONTACT","source_id":id,"title":name}));
        }
    }
}
// emails (邮件，搜索主题和正文)
if let Ok(rows) = database::all("SELECT id, subject FROM emails WHERE (subject LIKE ?1 OR body_text LIKE ?1) AND is_deleted=0 LIMIT 10", &[json!(&like)]) {
    for r in &rows {
        let id = r["id"].as_str().unwrap_or("").to_string();
        let name = r["subject"].as_str().unwrap_or("").to_string();
        if seen.insert(format!("EMAIL-{}", id)) {
            objects.push(json!({"source_type":"EMAIL","source_id":id,"title":name}));
        }
    }
}
```

#### 1.2 `lookup_object_names()` 函数（第182-211行）

在 match 块中添加新类型：

```rust
"CHANNEL" => database::get("SELECT name FROM partners WHERE id=?1 AND type='CHANNEL'", &[json!(sid)])
    .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
"VENDOR" => database::get("SELECT name FROM partners WHERE id=?1 AND type='VENDOR'", &[json!(sid)])
    .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
"CUSTOMER" => database::get("SELECT name FROM partners WHERE id=?1 AND type='CUSTOMER'", &[json!(sid)])
    .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
"CONTACT" => database::get("SELECT name FROM partner_contacts WHERE id=?1", &[json!(sid)])
    .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
"EMAIL" => database::get("SELECT subject FROM emails WHERE id=?1", &[json!(sid)])
    .ok().flatten().and_then(|r| r["subject"].as_str().map(String::from)).unwrap_or_default(),
```

---

### 2. 前端：`src/renderer/src/components/layout/Header.vue`

#### 2.1 `typeLabelMap`（第148行）

```typescript
const typeLabelMap: Record<string,string> = {
  FILE:'文件', NOTE:'笔记', PRODUCT:'产品', PROJECT:'项目', TODO:'待办',
  CONTACT:'联系人', CHANNEL:'渠道商', VENDOR:'供应商', CUSTOMER:'客户', EMAIL:'邮件'
}
```

#### 2.2 `typeBadgeMap`（第149行之后）

需要同步扩展 badge 颜色映射。

#### 2.3 `selectSearchResult`（第127-146行）

添加新类型的导航逻辑：

```typescript
function selectSearchResult(obj) {
  searchOpen.value = false
  searchInputRef.value?.blur()
  if (obj.project_id) {
    // ... 现有逻辑
  }
  const map: Record<string, import('@/stores/selection').SelectionType> = {
    PROJECT: 'project', PRODUCT: 'product', NOTE: 'note', TODO: 'todo',
    CONTACT: 'contact', CHANNEL: 'partner', VENDOR: 'partner', CUSTOMER: 'partner',
  }
  const t = map[obj.source_type]
  if (t === 'partner' || t === 'contact') {
    navigateTo(t, obj.source_id)
    router.push('/contacts')
  } else if (t === 'email' || obj.source_type === 'EMAIL') {
    selectionStore.jumpTo('email', obj.source_id)
    router.push('/email')
  } else if (t) {
    navigateTo(t, obj.source_id)
  } else if (obj.source_type === 'FILE') {
    invoke('file_open', { versionId: obj.source_id }).catch(() => {})
  }
}
```

---

### 3. 前端：`src/renderer/src/composables/useNavigation.ts`

扩展 `typeRouteMap`：

```typescript
const typeRouteMap: Record<SelectionType, string> = {
  product: '/products',
  note: '/notes',
  todo: '/todos',
  project: '/projects',
  contact: '/contacts',
  partner: '/contacts',
  email: '/email',
}
```

---

### 4. 前端：`src/renderer/src/stores/selection.ts`

扩展 `SelectionType`：

```typescript
export type SelectionType = 'product' | 'note' | 'todo' | 'project' | 'contact' | 'partner' | 'email'
```

---

## 执行顺序

1. ✅ 修改 `kb_memory_commands.rs` - search_business()
2. ✅ 修改 `kb_memory_commands.rs` - lookup_object_names()
3. ✅ 修改 `Header.vue` - typeLabelMap + typeBadgeMap
4. ✅ 修改 `Header.vue` - selectSearchResult
5. ✅ 修改 `useNavigation.ts` - typeRouteMap
6. ✅ 修改 `selection.ts` - SelectionType

---

## 验证方式

1. 在 Header 搜索框输入关键词，验证以下类型是否出现：
   - 渠道商名称
   - 供应商名称
   - 客户名称
   - 联系人姓名
   - 邮件主题或正文关键词
2. 点击搜索结果，验证是否正确跳转到对应页面
3. 验证类型筛选器是否显示新增的类型标签
