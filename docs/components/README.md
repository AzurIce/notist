@!(title: "Web Components 约定")

# Web Components 约定

Notist 将**内容抽象定义**与 **Web Component 运行时实现**彻底解耦：

1. **`notist-core` 只负责类型签名与数据树**：模块作者在 `.not` 中写无函数体的外部声明（例如 `docs/components/lib.not`），`notist-core` 对调用进行完整静态类型检查，并产出纯数据节点 `CustomNode { module, name, props }`。
2. **SSG 按目录约定加载 Web Components**：仅在 SSG 渲染 HTML 时，`notist-ssg` 检查声明模块所在目录是否存在 `<name>.js` 或 `<name>/index.js`。

## 示例

在 `components/lib.not` 中声明：

```not
fn badge(label: String, count: Int) -> Inline;
fn panel(title: String, body: Block) -> Block;
```

在同目录提供 `components/badge.js` 或 `components/panel/index.js`（导出继承 `HTMLElement` 的默认类）。在 `.md` 文档中调用：

```markdown
#import "./components/lib.not" as ui

状态：#ui.badge("beta", 2)

#ui.panel("说明")[
  子内容仍然是完整的 `notist-core` 内容树，链接与标题照常校验。
]
```

SSG 构建时会自动完成：

- 将 `<name>.js` 或整个 `<name>/` 目录复制到 `_notist/components/...`。
- 为使用了该组件的页面生成 `_notist/pages/<page>.js` 注册脚本，并通过 `<script type="module">` 注入模板。
- 输出 `<notist-<module>-<name> data-props="...">...</notist-<module>-<name>>`；若未提供 JS 实现，则回退为普通 `<div class="notist-component">` / `<span>`，不阻塞构建。
