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
