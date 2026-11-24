@!(title: "包与工作区")

# 包与工作区

单个站点目录放置一个包含 `[package]` 与 `[site]` 的 `Notist.toml`；多个站点可通过根目录的 `[workspace]` 统一管理：

```toml
[workspace]
members = ["examples/site", "examples/ssg-extensions"]

[package]
name = "notist"
```

- `notist build` 默认构建当前所在包；`notist build --workspace` 构建所有工作区成员；`notist build -p <name>` 构建指定成员。
- `notist serve -p <name>` 启动指定成员的本地预览服务器。
