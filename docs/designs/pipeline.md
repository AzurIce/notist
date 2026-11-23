@!(title: "编译与构建管线")

# 编译与构建管线

一份 `.md` 文档与一组 `.not` 模块如何变成最终的静态站点 HTML 或编辑器 LSP 语义反馈？整个过程在解析之后迅速汇合为同一条前端无关的 `notc` 管线。

## 一图总览

```text
  .md 文档 (MarkdownFrontend / rushdown)     其他前端 (DocumentFrontend)     .not 模块源码
                     │                                   │                         │
                     ▼                                   ▼                         ▼
         单向降级到 BlockPiece/InlinePiece    实现 DocumentFrontend trait   parse (lexer + parser +
         (@!(...) + #import + #call + 统一属性)                              flank [inline] / [ block ])
                     │                                   │                         │
                     └──────────────────► 统一的 notc::Module IR ◄─────────────────┘
                                                   │
                          ┌────────────────────────┴────────────────────────┐
                          │   site.analyze（严格静态屏障，零求值、零数据 IO） │
                          │  1. 共享 .not 模块三层检查 + 预降级 SharedLowered│
                          │  2. 并行解析 & 静态检查全部文档 → CheckedDocument │
                          │     （直接复用共享 FunctionId，不重复降级共享函数）│
                          └────────┬────────────────────────────────┬───────┘
                                   │                                │
                   ┌───────────────┘                                └───────────────┐
                   ▼ (LSP 路径：不求值)                                              ▼ (SSG 路径：静态无错后进入)
            WorkspaceAnalysis                                            site.evaluate（并行求值）
            容错 AST 语义索引                                              CheckedProject::eval_checked_document
            → Diagnostics / Symbols                                      + SsgDataHost（共享 JSON 分片缓存）
            → Hover / GotoDef                                            → 不可变单节点树 notc::Block
            → 类型感知 Completions                                        → Document::from_block (sectionize)
                                                                         → 预渲染 HtmlPlan 并即时释放树
                                                                                │
                                                                                ▼
                                                                         site.render（并行回填链接与组件）
                                                                         • DocumentIndex 校验跨页链接与 #anchor
                                                                         • 按约定加载 Custom 节点对应 JS 组件
                                                                         • 代码高亮（剥离围栏属性）
                                                                                │
                                                                                ▼
                                                                         site.templates (Tera) → BuildOutput
```

## 关键设计决策

- **前端无关的 IR 边界**：`notc::ast`（`BlockPiece`/`InlinePiece`/`Module`）、`notc::content`（`Block`/`Inline`）、类型检查器与求值器完全不依赖任何 Markdown 类型；`rushdown` 被严格封装在 `crates/notc/src/markdown/` 内部，内置 `MarkdownFrontend` 与任意自定义前端都通过同一个 `DocumentFrontend` trait 降级到 `notc::Module`。
- **AST 级统一属性挂载**：前端仅产出 `Attr` 占位节点，由 `build_document_module` 内的统一 pass 挂载到紧随其后的第一个 AST 节点。
- **严格静态屏障 + 共享函数预降级**：`CheckedProject` 预降级一次单态共享函数（`SharedLowered`）；`site.analyze` 先完成全站文档静态检查，存在任何语法/类型错误则立即终止、绝不触发求值或 `DataHost` IO。
- **`sectionize()` 移出编译器**：标题折节是 `Block` 上的纯库方法，只在下游显式调用，并将标题属性提升到外层 `Section`。
- **Web Components 归属 SSG 层**：`notc` 中的外部声明只生成 `CustomNode` 数据；JS 组件发现、脚本打包与 `<script type="module">` 注册全部在 SSG 阶段按目录约定完成。
