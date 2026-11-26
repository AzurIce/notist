@!(title: "Book 导航与主题")

# Book 导航与主题

`builtins.book` 是 `notist-ssg` 内置的书籍型文档扩展与配套主题，通过内容根目录的 `SUMMARY.md` 声明章节层次、分区标题与分隔线。

## 启用方式

```toml
[site]
title = "My Docs"
content = "docs"
theme = "builtins.book"
directory_urls = true

[site.extensions.builtins.book]
select = true
```

- `select = true` 时，仅发布 `SUMMARY.md` 中声明的页面与目录子树；否则所有文档照常发布，`SUMMARY.md` 仅提供侧边栏导航、面包屑与上下页链接。
- `SUMMARY.md` 本身作为声明式数据由 `notist-core` 解析，不作为页面发布。

## `SUMMARY.md` 写法

```markdown
# Summary

- [概述](README.md)

# 章节

- [入门](guide/README.md)
  - [安装](guide/install.md)
  - [配置](guide/config.md)
```

支持无序列表缩进嵌套、一级分部标题（`# 分部名`）、分隔线（`---`）、草稿项（`[待写]()`）与目录自动发现（`[目录](dir/)`）。
