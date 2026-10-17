# 图标来源与许可证

本目录三套图标体系并存（设计依据 `SYSTEM.md`）：

## 1. 自绘双色调体系（主体）

`src/graph/iconArt*.ts` 内联 SVG（data URI，无独立文件），**项目自有、许可随本仓库**。
设计系统：32px 网格 / 2px 圆角描边 / 固定双色调令牌（详见 `SYSTEM.md`）。
新增图标走 contact sheet 流程（`/contact.html`）审阅后登记 `icons.ts`。

## 2. Tabler Icons 补缺（`tabler/`，20 个）

- 来源：[tabler-icons](https://github.com/tabler/tabler-icons)，**MIT**
- 仅取角色与通用符号类（user/users/star/folder/clock/search…），已将 `currentColor` 替换为设计系统描边色 `#3F4C63`

## 3. 经典（draw.io 写实集，`net/` `pc/` `allied/` `ad/`）

提取自开源项目 [draw.io](https://github.com/jgraph/drawio)（Apache-2.0，仓库 `src/main/webapp/img/`），仅作 demo 用途：

- `net/` — `img/networking/`（源自 openclipart 公有领域素材 gwf 系列）
- `pc/` — `img/computers/`
- `allied/` — `img/lib/allied_telesis/`
- `ad/` — `img/lib/active_directory/`

**商用提示**：draw.io `img/LICENSE` 含「不得作为 Atlassian 产品资产分发」限制；厂商品牌图形涉商标。
自绘体系与 Tabler（MIT）可安全商用；经典集若随 ArkMind 商业发布需复核或下架。
