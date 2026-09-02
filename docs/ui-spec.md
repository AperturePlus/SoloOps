# SoloOps WebUI 设计规范（ui-spec）

- **目的：** 把"设计品味"翻译成"机械可执行的规则"。写 UI 的人（你或 AI）**不需要审美判断，只需要合规**。
- **适用范围：** `apps/web/src` 下所有 `.svelte` / `.css`。
- **强制手段：** `bun run lint:ui`（违禁模式扫描）+ `bun e2e/ui-audit.mjs`（截图审查）。

> 原则：**不发明新设计。** 所有取值都从本规范的 token 表里选；表里没有的值不允许出现。
> 判断标准只有一条：**和相邻元素是否同构**（同样性质的元素，必须有同样的尺寸、圆角、边框、灰阶）。

---

## 1. Token 表（唯一的取值来源）

### 1.1 字号（Type Scale）

| Token       | px  | 用途                                |
| ----------- | --- | ----------------------------------- |
| `text-2xs`  | 11  | 微标签、时间戳、mono 徽标、辅助计数 |
| `text-xs`   | 12  | 次要说明、帮助文字                  |
| `text-sm`   | 13  | **默认正文字号**                    |
| `text-base` | 14  | 强调正文                            |
| `text-lg`   | 16  | 卡片小标题（可选）                  |
| `text-xl`   | 20  | 页面标题                            |
| `text-2xl`  | 24  | 大数字（统计值）                    |

- **禁止** `text-[13px]` 等任意值；禁止 `text-[0.8rem]`。
- 正文默认 `text-sm`；不要在正文里用 `text-xs` 凑密度，改用布局解决。

### 1.2 字重与层级（仅 3 档灰阶）

| 语义                         | Token                       | 替代掉的旧写法           |
| ---------------------------- | --------------------------- | ------------------------ |
| 主要文字                     | `text-foreground`           | `text-slate-100/200`     |
| 次要文字                     | `text-muted-foreground`     | `text-slate-300/400`     |
| 弱化文字（时间戳、ID、占位） | `text-muted-foreground/65`  | `text-slate-500/600/700` |
| 强调/品牌                    | `text-mint-400`（或语义色） | —                        |

- **禁止**直接写 `text-slate-*`、`text-white`、`text-zinc-*` 等原始灰阶。
- 悬停变亮 = `hover:text-foreground`（基于 muted-foreground）；不要引入新灰度。

### 1.3 颜色（语义色）

| 语义      | Token                                           |
| --------- | ----------------------------------------------- |
| 背景      | `bg-background` / 面板 `bg-card`（或 `.panel`） |
| 边框-常规 | `border-edge`（= `--border` #1c2230）           |
| 边框-强调 | `border-edge-strong`（= `--input` #2a3245）     |
| 品牌/成功 | `mint-400` / `mint-500`                         |
| 运行/信息 | `sky-300/400`                                   |
| 审批/警告 | `amber-300/400`                                 |
| 失败/危险 | `red-300/400` / `destructive`                   |

- **边框只有两个值**：`edge`（静止）与 `edge-strong`（hover / 激活 / 强调）。
- **禁止** `border-white/10`、`border-white/5`、`divide-white/[0.04]` 等白色透明度边框。
- 状态色 tint 一律 `/10`（底）+ `/30`（边框）+ 命名色文字，如 `bg-sky-400/10 border-sky-400/30 text-sky-300`。

### 1.4 圆角（Radius）

| 层级                             | 值                                  | 用于                       |
| -------------------------------- | ----------------------------------- | -------------------------- |
| 面板/卡片/横幅                   | `rounded-lg`（10px，`.panel` 同值） | 所有卡片、banner、大输入区 |
| 按钮/输入框/小卡                 | `rounded-lg`（10px，shadcn 默认）   | Button、Input、Textarea    |
| 内嵌小块（图标槽、chip、代码块） | `rounded-md`（8px）                 | 图标容器、内嵌 well、tag   |
| 药丸/圆点                        | `rounded-full`                      | StatusBadge、状态点        |

- **禁止** `rounded-xl`（14px）用于卡片；`rounded-lg` 与 `.panel` 必须同值共存。

### 1.5 图标尺寸（仅 3 种）

| 场景                | 尺寸        | 用法                 |
| ------------------- | ----------- | -------------------- |
| 行内小图标          | `size={12}` | 徽标旁、次级行       |
| 常规图标            | `size={14}` | 列表、面板头、按钮内 |
| 独立图标按钮/页面头 | `size={16}` | 导航、页面标题旁     |

- **禁止** `size={10/11/13/15/18/22}`。`Icon` 的 `size` prop 只允许 12/14/16。
- 需要更大的视觉重量时用图标槽（见 §3.2）包住 14px 图标，而不是放大图标本身。

### 1.6 控件高度

| 控件                  | 高度                          |
| --------------------- | ----------------------------- |
| 默认按钮 / 输入框     | `h-8`（32px，shadcn default） |
| 页面主 CTA / 登录提交 | `h-9`（36px，`size="lg"`）    |
| 图标按钮              | `size-8`                      |

### 1.7 间距（4px 网格）

| 场景                | 值                                           |
| ------------------- | -------------------------------------------- |
| 卡片内边距          | `p-4`（16px）；大区 `p-5` 仅用于整版 section |
| 页面水平留白        | `px-6`（24px）                               |
| 面板间距 / 区块间距 | `gap-4` / `mt-4`（16px）                     |
| 列表行内            | `gap-2`、`gap-2.5`                           |
| 微间距              | `gap-1` / `gap-1.5`                          |

- **禁止** `p-[18px]`、`gap-[13px]` 等任意值；`gap-5`(20px) 不再新增。

### 1.8 阴影

- 面板：`--shadow-panel`（`.panel` 内置）。
- 浮层：`--shadow-pop`。
- 不新增阴影值；卡片不堆叠多层阴影。

---

## 2. 页面骨架（所有页面必须同构）

```text
┌──────────────────────────────────────────────┐
│ PageHeader（面包屑 · 标题 · 副标题 · 右侧动作） │  h-14 顶栏或页头，px-6
├──────────────────────────────────────────────┤
│                                              │
│   内容区：唯一 max-w 容器 + px-6 + gap-4      │
│                                              │
└──────────────────────────────────────────────┘
```

1. **每个路由都必须用 `PageHeader` 组件**（`$lib/components/PageHeader.svelte`）开头：
   面包屑（上一级页名 › 当前页名）+ 标题（`text-xl`，`text-foreground`）+ 副标题（`text-sm`，`text-muted-foreground`）+ 右侧动作按钮。
2. 面包屑分隔符统一 `›`；上一级链接用 `text-muted-foreground/65`，当前页 `text-foreground`。
3. 顶栏：`h-14`、`border-b border-edge`、`bg-background/80 backdrop-blur`。
4. 左侧导航 rail（w-14）保持现状：rail 内图标使用 16px。
5. **例外**：运行详情页（`/runs/:id`）使用紧凑 sticky 监控工具栏（`h-14`）替代 PageHeader，取值同样来自本规范 token 表。
6. 页面内容容器统一 `max-w-7xl`（1280px）；禁止各页自定 `max-w-[Npx]`。

---

## 3. 组件模式（复用优先，禁止手搓）

### 3.1 状态展示（唯一入口：`StatusBadge`）

- 列表、详情、卡片一律使用 `StatusBadge`；紧凑场景（列表行内）用 `<StatusBadge status={...} compact />`。
- **禁止**手写截断徽章（如 `"succeeded".slice(0, 3)` → "SUC"）、禁止自制药丸。
- 纯色点仅用于"极小面积"场景（列表项左侧 1.5px 点），色值与 StatusBadge 同源。

### 3.2 图标槽（Icon tile）

```svelte
<span
  class="grid size-9 place-items-center rounded-md border border-edge bg-ink-800/70 text-<语义色>"
>
  <Icon name="..." size={14} />
</span>
```

- 尺寸仅两档：`size-9`（页面头/统计卡）、`size-7`（列表行内）；底色带语义 tint 时去掉 border。
- **禁止**为图标槽发明第三种尺寸或新底色。

### 3.3 面板（Panel）

- 一律 `.panel`（含边框、圆角、阴影）；交互卡片加 `.panel-hover`。
- 面板标题：`text-sm font-semibold text-foreground` + 左侧 14px 图标（可选）+ 右侧辅助元素（计数用 `font-mono text-2xs text-muted-foreground/65`）。
- 区块微标题（如 "BUDGET"）：`text-2xs font-semibold uppercase tracking-[0.12em] text-muted-foreground/65`。

### 3.4 统计卡（Stat card）

- 结构固定：图标槽（size-9，语义 tint）+ `text-2xs` 大写标签 + `font-mono text-2xl tabular-nums` 数值。
- 可点击统计卡：**必须** `panel-hover` + 语义色 hover；不可点击统计卡用普通 `.panel`。两种不要混排在同一行。

### 3.5 键值行（KeyValue）

- 标签：`text-2xs uppercase tracking-[0.12em] text-muted-foreground/65`；值：`font-mono text-sm text-foreground`。
- 分隔线：`divide-edge`（禁止白色透明度分割线）。

### 3.6 空状态

- 统一：虚线框（`rounded-lg border border-dashed border-edge`）+ 标题（`text-sm font-semibold text-foreground`）+ 一句说明（`text-sm text-muted-foreground`）+ 主 CTA（Button）。禁止在空状态里塞第二层级营销文案。

### 3.7 表单

- Label：`text-sm text-foreground`；帮助文字：`text-xs text-muted-foreground`；错误：`text-destructive`。
- 输入框用 shadcn `Input` / `Textarea`（`rounded-lg`），不要手写 input 类名组合。

---

## 4. Do / Don't 速查

| ✅ Do                              | ❌ Don't                                              |
| ---------------------------------- | ----------------------------------------------------- |
| 灰阶只用 3 档语义 token            | `text-slate-300`、`text-white/70`                     |
| 边框只用 `edge` / `edge-strong`    | `border-white/10`、`divide-white/[0.04]`              |
| 图标只 12/14/16                    | `size={13}`                                           |
| 卡片 `rounded-lg` / `.panel`       | 卡片 `rounded-xl`                                     |
| 状态用 `StatusBadge`               | 手搓截断徽章 / 自制药丸                               |
| 每页一个 `PageHeader`              | 每页发明自己的页头/面包屑                             |
| 面板内边距 `p-4`                   | `px-3 py-2.5` 随手调                                  |
| 次要文字 ≥ `text-muted-foreground` | slate-600/700 低对比灰字                              |
| 背景透明度走 token                 | `bg-white/[0.03]`（用 `bg-ink-800/60` 或 `bg-muted`） |

> `bg-white/[0.03]` 类微弱白 tint 已收敛为 `bg-ink-800/60`（token 化）。如需新的背景层级，**先改本规范加 token，再写代码**。

---

## 5. 审查闭环（怎么发现"不好看"）

1. **改前**：跑 `bun run dev:mock`，用 `bun e2e/ui-audit.mjs` 出全页截图（desktop + narrow）。
2. **自查三问**：
   - 同性质的相邻元素，尺寸/圆角/边框/灰阶是否完全一致？（错配检查）
   - 眯眼截图：最先看到的是不是页面想强调的东西？（层级检查）
   - 次要文字在 100% 缩放下还读得清吗？（对比度检查）
3. **改后**：重跑 `ui-audit`，新旧截图并排对比；`bun run lint:ui` 必须 0 违规。
4. 每次新增页面/组件，提交说明必须附 `ui-audit` 截图。

---

## 6. 违禁模式（`lint:ui` 强制）

- `text-slate-`、`bg-slate-`、`border-slate-`、`text-white`、`text-zinc`（原始灰阶）
- `border-white/`、`divide-white/`、`bg-white/`（白色透明度当边框/背景）
- `text-[0-9]`px、`gap-[0-9]`px、`p[xytlr]?-[0-9]+px`（任意值）
- `size={10}` `size={11}` `size={13}` `size={15}` `size={18}` `size={22}`（非标图标）
- `rounded-xl`（卡片 14px 圆角）
- `slice(0, 3)`（截断徽章）

白名单例外：`apps/web/src/lib/components/ui/**`（shadcn 官方生成组件不改）。Icon 的 `size` 另有 TypeScript 编译期约束（`12 | 14 | 16`），svelte-check 会直接报错。
