@!(title: "静态站点生成 (SSG)")

# 静态站点生成（SSG）

`notist-ssg` 将内容目录中的 `.md` / `.markdown` 文档连同它们导入的 `.not` 模块整体编译，并结合路由、链接校验、静态资源、Sass 与 Tera 模板发布为静态站点。

## 构建流程

1. **发现源文件**：按 `include` / `exclude` 和 `mounts` 收集文档与静态资源。
2. **站点前置扩展**：运行配置的 `before_compile` 扩展（如 `builtins.book` 解析 `SUMMARY.md` 并筛选页面）。
3. **全站 `notist-core` 编译**：解析全部文档与递归导入的 `.not` 模块，执行一次性静态类型检查，逐页求值并调用 `sectionize()`。
4. **全站链接与资源校验**：汇总全站页面路由与元素 `id`，解析相对链接、校验 `#anchor`、按需拷贝引用资源（缺失本地图片直接报错，断链发出告警）。
5. **HTML 渲染与 Web Components**：单遍将 `notist_core::Block` 渲染为 HTML，代码块走构建期语法高亮；对存在同目录 JS 实现的 `Custom` 节点按约定生成自定义元素标签并打包注册脚本。
6. **模板渲染**：运行 `before_template` 扩展，通过 Tera 模板（支持内置或自定义主题覆盖）并行渲染全部页面并输出。

## 模板上下文

每个页面渲染时提供：

- `page`：当前页面（`route`、`title`、`attrs`、`headings`、`html`、`directory`、`ancestors`、`root`）。`page.root` 是当前输出文件到输出根目录的相对前缀（根页面 `./`，`items/a.html` 为 `../`）。
- `site`：`title`、`base_url`、`pages`、`directories`、`params`、`data`。`site.params` 来自 `[site.params]` 与 `-D KEY=VALUE`。
- `url(path=..., relative_to=...)`：生成站点 URL。省略 `relative_to` 时按 `base_url` 拼接；`base_url = "relative"` 时按当前页面生成相对 URL。
- `to_json` / `json_encode`：把值编码为 JSON，并把 `</` 转义为 `<\/`，可直接嵌入 `<script type="application/json">`。

相关文档：[Book 导航与主题](../book.md)、[站点扩展](extensions.md)、[HTML 渲染](../html.md)、[Web Components 约定](../components/README.md)。
