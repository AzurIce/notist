@!(title: "CLI 与 LSP")

# CLI 与 LSP

`notist` 是 Notist 的统一命令行入口，覆盖构建、本地热重载预览、静态检查、单页 HTML 导出和 LSP 语言服务：

| 命令 | 说明 |
| --- | --- |
| `notist build` | 构建当前包或工作区静态站点（支持 `--out-dir`、`-j <N>`、`--timings`） |
| `notist serve` | 启动本地 HTTP 预览服务，监听 `.md` / `.not` / 模板 / 资源变化并热重载 |
| `notist check <file\|dir>` | 使用 `notc` 静态检查单个 `.md`/`.not` 文件或整个目录并输出源码级诊断 |
| `notist cst <file>` | 打印由 `notc` 解析出的 `Module` AST |
| `notist core <file>` | 输出静态分析诊断与声明；加 `--eval` 对文档或 `.not` 的 `main` 求值 |
| `notist json <file>` | 输出包含诊断、符号与求值内容树的机器可读 JSON |
| `notist query <file> <selector>` | 按 `id:` / `tag:` / `ctor:` / `function:` / `level:` 查询求值后的 `notc` 内容节点 |
| `notist html <file>` | 将单个 `.md` 文档求值导出为独立 HTML 并复制引用的安全本地图片 |
| `notist lsp` | 通过 stdio 启动基于 `notc::ide` 的 Language Server（支持 `.md` / `.markdown` / `.not`） |

## 编辑器 LSP 支持

`notist lsp` 同时支持 `.md` 文档与 `.not` 模块，提供：

- **实时诊断**：未保存修改即时生效，语法/类型错误随编辑自动发布与清除。
- **文档大纲**：`.not` 展示函数与类型方法，`.md` 展示嵌套标题结构。
- **Hover**：展示表达式类型、局部变量类型及完整泛型 `where` 签名。
- **跳转定义**：支持 `.md` ↔ `.not` 跨文件函数/方法跳转、局部变量跳转以及 `.md` 间 `#anchor` 链接跳转。
- **类型感知补全**：在 `.md` 中由 `#` / `.` 触发；在 `.not` 中 `expr.` 会根据接收者静态类型列出全部用户方法与内置方法。
