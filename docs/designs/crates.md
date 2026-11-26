@!(title: "Crate 架构")

# Crate 架构

Notist 工作区采用“前端无关语言内核 + 薄宿主与站点层”的清晰边界：

| Crate | 职责 |
| --- | --- |
| `crates/notist-core` | 独立纯库语言内核：`.not` 词法/语法、前端无关 `Module` IR 与 `DocumentFrontend` 边界、内置 `markdown` 前端（基于轻量纯 Rust `rushdown` 封装在独立子模块内）、多文件三层类型检查、共享函数预降级、不可变 `Inline`/`Block` 求值与容错 `ide` 层 |
| `crates/notist-ssg` | 静态站点生成器：基于 `notist-core` 并行分析/求值/渲染站点，提供路由、链接与锚点校验、缓存 JSON `DataHost`、按约定加载 Web Components、`builtins.book` 导航、Tera 模板/主题与 Sass |
| `crates/notist-cli` | 统一命令行入口 `notist`：`build`、`serve`、`check`、`cst`、`core`、`json`、`query`、`html`、`lsp`（全量基于 `notist-core`） |
| `crates/notist-html` | HTML 转义、安全 URL 校验与基于 `syntect` 的构建期代码高亮（`highlight::highlight_str` + `CSS`） |
| `crates/notist-observe` | 构建阶段耗时采集与进度观测 |
| 根 `notist`（`src/`） | 宿主通用基础设施：`Resources` 虚拟文件系统（磁盘/内存/监听观察）、`Executor` 并行执行器、`Assets` 资产与 URL 工具、`Diagnostic` 与 `Vault` |
