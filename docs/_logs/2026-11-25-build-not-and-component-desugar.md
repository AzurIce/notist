# Markup 组件降糖、`notc` 声明重绑定与 `build.not` 演进规划

2026-11-25 · 状态：总体架构定稿，阶段一已完成 · 范围：`notc` 语言内核、组件模型、`Notist.toml`、SSG 与构建管线

## 1. 背景与现状问题诊断

### 1.1 `math_component` 特判补丁的局限

在 `feat/notc-v2` 迁移重构期间，旧版文档级 `[[extensions]]`（`kind = "replace"`）被移除，`notist-ssg` 临时引入了 `[site].math_component` 配置项直接将 `InlineKind::Math` / `BlockKind::Math` 渲染为硬编码的 `<notist-math>` 标签。这一临时方案偏离了原有的语言设计初衷，造成了多处割裂：

- **丧失通用性**：只能特判内置数学公式，无法替换其他内置语法糖，也无法支持下游项目（如 `Vault`）已经实际使用的跨模块抽象组件替换（例如 `from = "components.diagram"`、`to = "pele.diagram"`）。
- **全局标签写死**：硬编码为单一 `<notist-math>` 自定义元素名，无法在同一页面并存不同模块的数学组件（例如同时直接调用 `#katex.math(...)` 与 `#typst.math(...)`）。
- **工具链不一致**：仅在 `notist-ssg` 整站构建中生效，单文件导出命令 `notist html` 完全不生效；`crates/notist-cli/src/package.rs` 的 `Manifest` 中残留了静默丢弃的 `extensions` 死字段；`modules/katex` 与 `modules/typst` 的签名被迫拆成了 `math` 与 `math_block` 两个单态函数，与其文档和单组件 JS 实现脱节。

### 1.2 下游真实项目的扩展痛点盘点

排查当前十余个使用 Notist 的真实项目（`Vault`、`ranim`、`azurice.github.io`、`icexiv-db`、`icexiv.github.io`、`project-agent-gui-use`、`TradeFactory`、`icy-bot` 等）后，现有扩展的实际用途可归为三类：

1. **组件身份重绑定**：将内置 `notist.math` 映射到 `typst.math` / `katex.math`，或将抽象模块声明 `components.diagram` 映射到具体实现 `pele.diagram`。
2. **弥补早期语言能力缺失的临时胶水**：如 `project-agent-gui-use` 用 Python 读 `registry.json` 拼接 `.md` 源码，`Vault/cosmic` 因早期 `.notc` 无法读文件而在 `before_check` 阶段起 Python 进程往 AST 参数里硬塞序列化 JSON。如今 `notc` 已具备 `read_json`、`read_text`、`for` 与动态内容构造能力，这类胶水本身已可直接用 `.not` 函数内化。
3. **离线重型预处理与构建期站点装配混杂**：如 `icexiv-db`（离线 `prepare` 用 Pillow 转码数万张图标并切分 JSON，但构建期 `extension` 仍需通过 JSONL 子进程吐出 4 万份 `#views.item(...)` Markdown 字符串交由 `notc` 重复解析检查 4 万次）、`azurice.github.io`（混杂 `git log` 时间戳提取、正则剥离 Markdown 提取纯文本摘要、单文件 `uno.css` 路径映射与虚拟页生成）、`ranim/website`（读取 Cargo 示例清单与 `.shadow/refs` 注入虚拟页和 `site.data`）。由于外部进程绕过了 `notist::Resources`，每个项目都不得不手写冗长且易漏的 `watch = [...]` 列表。

---

## 2. 核心架构原则与设计决策

为防止后续开发偏离方向或重走已否决的弯路，在此明确记录本次确立的三条核心架构支柱：

### 2.1 支柱一：全部 Markup 本质上都是 `notist.*` 组件函数的语法糖

- **内置根模块 `notist`（`ModuleId::NOTIST`）**：所有内置内容构造函数（`text`、`strong`、`emph`、`strike`、`underline`、`raw`、`math`、`link`、`image`、`inlines`、`para`、`heading`、`callout`、`code_block`、`display_math`、`quote`、`divider`、`list`、`table`、`div`、`section`、`blocks`）不再只是与模块系统割裂的特殊指令，而是作为内置模块 `"notist"` 下的正式函数声明，拥有标准的 `DeclId` 与 `FunctionSig`。
- **语法糖与显式调用同源且防遮蔽**：
  - 文档与内容字面量中的 Markup 语法糖（如 `$x$`、`$$x$$`、`**a**`、`# Title`、`[label](url)`）在类型检查时直接绑定到 `ModuleId::NOTIST` 下对应的 `DeclId`，**绝不受局部同名变量或局部同名函数的词法遮蔽影响**。
  - 代码/插值中的无限定调用 `math("x")`（经 prelude 作用域）与限定调用 `notist.math("x")`（经 `"notist"` 命名空间）解析到**同一个 `DeclId`**。
- **编译期类型参数默认值统一行内/块级双模签名**：
  - 为 `notc` 补齐编译期类型参数默认值语法 `<C: Type = DefaultType>`。
  - 数学组件恢复统一签名：
    ```not
    fn math<C: Type = Inline>(text: String) -> C
    where C == Inline || C == Block;
    ```
  - `$x$` 与 `#math("x")` 实例化 `C = Inline`；`$$x$$` 与 `#math<Block>("x")` 实例化 `C = Block`。一条指向 `math` 声明的替换规则即可同时覆盖行内与块级公式，且与单个自定义元素基于 `notist-content="inline|block"` 渲染的浏览器端协议天然吻合。
- **调用点源码 `Span` 透传**：外部组件声明（以及内置原语）在求值生成节点时继承调用点的 `Span`，确保断链检查、锚点诊断与源码映射始终精准指向文档中的原始调用或语法糖位置。

### 2.2 支柱二：对标 `build.jai`——在 `TYPECHECKED` 层（`ModuleExpr` / `DeclId`）拦截与重绑定

在讨论中，我们明确评估并**否决了另外两种替代方案**（后续维护者切勿回退到这两种方案）：

- **已否决方案 A：在 `notc` 求值产出 `Block` 内容树之后做后处理替换**
  - **否决原因**：若替换目标是带函数体的普通 `.not` 函数，第一遍求值已白白分配了所有旧节点并构造成完整 `Arc` 树，后处理还需全树二次遍历、拆解属性重新进入求值器并重建祖先链 `Arc`；同时使目标函数的签名校验与 `DataHost` 执行脱离了 `notc` 的一次性静态检查屏障。
- **已否决方案 B：在裸 Parse AST（`ast::Expr`）层改写或由宿主合成虚拟 `notist` 垫片模块**
  - **否决原因**：裸 Parse AST 仅有字符串名称，尚未消解局部绑定与各类 `import` 别名/通配导入，直接改写极易产生词法遮蔽漏洞；而合成 `notist` 垫片模块只能处理顶层内置函数，无法支持 `from = "components.diagram"` → `to = "pele.diagram"` 这类任意具体模块函数的替换，还会篡改 LSP 定义跳转与悬停看到的原始声明身份。

**确立的正统方案：参照 Jai（`build.jai`）的 `TYPECHECKED` 阶段设计，落位在 `notc` 的 Layer 2（`ModuleExpr` / `CheckedDecl`）与 Layer 3（特化降级 `CheckedExpr`）之间**：

```text
源码 (.md / .not)
  │
  ▼  Layer 1 & 2: Parse + Interface + Body Check
Typed Semantic IR (ModuleExpr / CheckedDecl，所有引用已消解为精确的 DeclId + TypeArgs)
  ├──► [LSP / IDE 分支] 停在此层：Hover、跳转定义、查找引用 100% 忠于源码原声明身份
  │
  ▼  Layer 3: Specialization / Lowering（应用 BuildPlan：replacements & 未来的 call_rewriters）
Executable IR (CheckedExpr，指向替换后的目标 FunctionId，局部重检保证类型安全)
  │
  ▼  Single-pass Eval
最终不可变 Block 内容树（无论目标是 Web Component 外部声明还是带函数体的 .not 函数，均严格单遍求值）
```

- **彻底消除遮蔽与内置特例**：由于匹配对象是已消解的 `DeclId`，替换 `notist.math` 与替换任意用户模块函数 `components.diagram` 走完全同一套机制，局部变量、重命名导入、一等函数值传递均精确生效。
- **防自递归包装**：若替换目标函数 `B`（处理 `A -> B`）在自身函数体内显式调用原声明 `A`（例如用 `.not` 函数给默认节点套一层属性或容器），特化器在降低 `B` 自身函数体时保留对原声明 `A` 的引用，天然支持装饰器写法而不会陷入无限自递归。
- **与 `build.jai` 的关键并行化差异**：Jai 的 `compiler_wait_for_message()` 是单线程阻塞事件循环；为保持 Notist 全站数万页的多核并行吞吐，Notist 采用 **“单次执行 `build.not` 组装不可变 `BuildPlan`（声明重绑定表 + 按目标 `DeclId` 订阅的纯函数 AST 改写器） + `Executor` 多核无锁并行特化与求值”** 的模型。

### 2.3 支柱三：`build.not` 终极形态与纯计划值边界（避免语言膨胀与副作用污染）

从长期来看，`build.not` 不仅取代 `Notist.toml` 中的 `[[extensions]]`，还将统一原文档级扩展与站点级扩展，使 SSG / Book 本身成为建立在宿主引擎原语之上的标准 `.not` 管线程序，并支持定义单次 `notist build` 的文件发现、路由、虚拟页生成与多目标输出。

为彻底消除“把构建与 SSG 放进 `.not` 会导致 `.not` 引入复杂 IO 与副作用”的风险，严格划定以下三条铁律：

1. **外部重型预处理与内部确定性装配分离**：
   - 非确定性或重算力二进制工具（`git log` 历史提取、Pillow 图像转码、WASM 编译、外部 JS 账单引擎等）一律归入构建前外部预处理步骤（如 `prepare` 脚本/任务），产出静态文件与 JSON 清单。
   - Notist 构建管线内部只做纯确定性的“数据/源码读取 → 内容编译与改写 → 路由与产物规划”。
2. **`.not` 永不拥有真实写盘或子进程 IO，只构造纯内存 `OutputPlan`**：
   - 普通 `.md` 与业务 `.not` 模块仅能通过只读 `DataHost` 调用受限读取（`read_json` / `read_text`）。
   - 仅顶层 `build.not` 入口接收宿主传入的只读 `BuildContext` 能力句柄（底层直连 `notist::Resources`，任何 `glob` / `exists` / `read_*` 操作均由宿主自动记录文件监听依赖，彻底免除手写 `watch = [...]`），最终返回纯数据的 `OutputPlan`（对应 `notist::publication::Assets`），由 Rust 宿主统一完成冲突校验、原子落盘或内存预览服务。
   - `.not` 语言自身只需演进规划内的名义结构体（Record）、枚举（Enum）与闭包，无需引入可变全局状态、文件系统写 API、子进程执行或 `async` 机制。
3. **控制面在 `.not`，重算力数据面留在 Rust 引擎原语**：
   - 并行文档检查与求值调度、全站链接与 `#anchor` 图校验、Syntect 语法高亮、Sass 编译、Web Component 发现拷贝与 Tera 模板渲染继续作为高性能 Rust 宿主原语暴露高层组合接口；`.not` 负责编排策略、虚拟页面闭包绑定（如 `icexiv-db` 直接将 4 万个路由绑定到已编译的 `views.item` 函数调用，省去生成和重解析 4 万份 Markdown 文本）以及 `site.data` / 附加产物生成。
   - `Notist.toml` 大幅收敛为静态清单（`[workspace]`、`[package]` 与基础 `[site]` 默认项）；无 `build.not` 的常规项目零配置直接运行默认流水线。

---

## 3. 分阶段演进路线图

整套重构拆分为三个可独立交付、逐步收敛的阶段：

### 阶段一：统一组件声明模型与 `notc` 内核级 `replace`（当前实施阶段）

1. **`notc` 编译期类型参数默认值与调用点 `Span`**：
   - 支持 `fn name<T: Type = Default>(...) -> ...` 语法、类型推导回退、部分显式类型实参补齐及 IDE 签名展示。
   - 在求值栈帧中透传调用点 `Span`，使外部组件声明与内置构造函数产出的节点均携带准确的源码调用范围。
2. **内置 `ModuleId::NOTIST` 与 Markup 全面降糖**：
   - 将所有内置内容构造函数注册为内置模块 `"notist"` 的正式声明（含 `DeclId`、`FunctionSig` 与默认 `CheckedDecl` 实现），`notist.math` 恢复为 `fn math<C: Type = Inline>(text: String) -> C where C == Inline || C == Block;`。
   - 所有 Markup 语法糖统一降糖为对 `ModuleId::NOTIST` 声明的调用；同时支持 `notist.<fn>` 限定访问与 prelude 直接访问。
3. **`notc` 内核级 `ReplacementSpec` / 契约校验与特化重绑定**：
   - 在 `CheckedProject` 上提供纯内存替换绑定接口，校验源与目标 `DeclId` 的泛型参数个数、默认类型参数、值参数类型、返回类型及 `where` 约束兼容性。
   - 支持多条规则按序单步折叠（`A -> B, B -> C` 得 `A -> C`，反序得 `B`），并在特化期完成 `DeclId` 重绑定（含目标函数体内部的防自递归豁免）。
4. **宿主收尾与模块对齐**：
   - 移除 `notist-ssg` 中的临时 `[site].math_component` 及 `_notist/components/math` 硬编码分支。
   - 在 `build.not` 落地前，先在 `notist-cli` 与 `notist-ssg` 中接通 `Notist.toml` 顶层 `[[extensions]]`（`kind = "replace"`），同时覆盖站点构建（`notist build` / `serve` / `check`）与单文件 HTML 导出（`notist html`，补齐 Web Component 打包）。
   - 将 `modules/katex/lib.not` 与 `modules/typst/lib.not` 恢复为统一的泛型 `fn math<C: Type = Inline>(text: String) -> C where C == Inline || C == Block;`，同步修正模块说明与配置参考文档。

### 阶段二：引入第一版 `build.not`，退役 TOML 扩展与外部 stdio 协议

- 提供内置构建编排接口（`Compiler.replace` 一等声明引用、基于 `Resources` 的只读目录发现与数据读取、直接绑定 `.not` 闭包的虚拟页面注册、单文件挂载、`site.data` 与额外文本产物输出）。
- `notist-cli` / `notist-ssg` 自动发现包根目录下的 `build.not` 并驱动构建。
- 迁移示例与下游站点扩展，彻底移除 `Notist.toml` 中的 `[[extensions]]` 与 JSON-over-stdio 命令扩展协议。

### 阶段三：开放 Typechecked AST 改写钩子与标准库化 SSG 组合子

- 在 `notc` 落地名义 Record / Enum 后，向 `build.not` 开放按 `DeclId` 订阅的纯函数 `rewrite_call`（读取并改写 `TYPECHECKED` 调用节点，经编译器局部重检后降级执行），支持诸如代码块按语言转图表组件等跨签名变换。
- 将 `builtins.book` 等高层站点编排逻辑沉淀为标准 `.not` 模块。
