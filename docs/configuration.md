@!(title: "配置参考")

# 配置参考（`Notist.toml`）

`Notist.toml` 描述包、工作区与静态站点配置。

## `[site]` 常用字段

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `title` | `"Site"` | 站点标题 |
| `base_url` | `"/"` | 站点的根路径或 `http(s)://` 地址 |
| `content` | `"content"` | 文档源目录 |
| `output` | `"public"` | 构建输出目录 |
| `theme` | 无 | 内置主题（如 `"builtins.book"`）或本地主题目录 |
| `templates` | `"templates"` | 站点自定义 Tera 模板目录（按文件名覆盖主题模板） |
| `static_dir` | `"static"` | 原样复制到输出根目录的静态资源 |
| `data` | `"data"` | 注入到 `site.data` 的 TOML / JSON 数据目录 |
| `sass` | `"sass"` | 自动编译的 `.scss` / `.sass` 样式目录 |
| `directory_urls` | `false` | 将 `README.md` / `index.md` 映射为目录 URL |
| `copy_content_assets` | `true` | 是否全量复制内容目录中的非文档资源；设为 `false` 时仅复制被显式引用的资源 |
| `include` / `exclude` | `["**"]` / `[]` | 内容目录相对路径 glob 过滤 |
| `mounts` | `[]` | 将外部目录挂载到站点内容路径下 |

## `[[extensions]]` 声明替换

包级 `Notist.toml` 支持在顶层声明同契约函数映射（`kind = "replace"`），将内置 `notist.*` 或任意模块的组件函数重绑定到目标实现，同时作用于站点命令（`build` / `serve` / `check`）与单文件命令（`check` / `html` / `core` / `json` / `query`）：

```toml
[[extensions]]
kind = "replace"
imports = { katex = "./modules/katex/lib.not" }
from = "notist.math"
to = "katex.math"
```

- `imports`：局部别名到 `.not` 模块路径的映射（相对当前 `Notist.toml` 所在目录解析）；内置根模块 `notist` 无需显式导入。
- `from` / `to`：形如 `<alias>.<fn>`，要求两端声明的类型参数个数、默认类型参数、值参数类型、返回类型与 `where` 约束相容。
- 多条规则按声明顺序单步折叠；替换目标函数体内部调用原声明时不触发自递归，可直接编写包装/装饰函数。
