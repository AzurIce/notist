# Notist

Notist 是基于前端无关内容树 IR 的文档工具链，内置 Markdown（`.md`）文档前端与 `.not` 编程模块，提供独立编译器 `notc` 与完整站点/LSP 工具链 `notist`。

- [Documentation](docs/README.md)：语法、内置函数、IR、模块与工具使用。
- [Notist CLI](docs/cli.md)：`notc` 编译器与 `notist` 包/workspace 构建、站点预览、单文件检查与诊断。
- [Notist.toml 配置](docs/configuration.md)：workspace / package / site / extensions 的字段、路径基准与输出规则。
- [Notist Book](docs/book.md)：书籍阅读结构、README 路由、`SUMMARY.md`、主题与本地预览。
- [Notist SSG](docs/ssg/README.md)：目录映射、站点模板、主题资源与网站构建。
- [Package 与站点组织](docs/packages.md)：单包单站点、workspace 成员与共享输出根。
- [本地模块](docs/modules.md)：相对路径 import、词法命名空间与组件资源。

```sh
cargo run -p notc -- check docs/
cargo run -p notist-cli -- build
cargo run -p notist-cli -- serve
```

默认文档产物位于 `notist-output/notist/`。使用 `cargo run -p notist-cli -- build --workspace` 同时构建文档与站点示例；每个包拥有独立的发布子目录。
