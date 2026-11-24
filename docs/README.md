@!(title: "Notist 文档")

# Notist

Notist 是一个以静态类型内容语言 `notc` 为核心的文档工具链，采用前端无关的内容树 IR，包含：

- **Markdown 文档（`.md` / `.markdown`）**：基于标准 CommonMark / GFM，无缝扩展数学公式、WikiLink、属性注解 `@!(...)`/`@(...)` 与 `#import` / `#call` 模块调用。
- **Code 模块（`.not`）**：面向复用的静态类型函数式语言，支持一等函数、编译期类型泛型（`<T: Type>` + `where` + `match T`）、显式类型关联方法，以及与代码无缝嵌套的不可变 `Inline` / `Block` 内容树。
- **LSP 语言服务**：基于纯内存容错分析引擎 `notc::ide`，提供跨文件实时诊断、大纲符号、Hover 类型签名、定义跳转与类型感知方法补全。
- **SSG 静态站点生成器**：将 `.md` 文档与 `.not` 模块编译为不可变内容树，完成链接校验、HTML 渲染、按约定加载 Web Components 以及 Tera 模板/主题发布。

## 快速导览

| 主题 | 入口 |
| --- | --- |
| 写文档 | [Markup 语法](markup/syntax.md)、[内容模型与内置方法](markup/builtin.md) |
| 写组件与逻辑 | [Code 语法](code/syntax.md)、[类型、泛型与方法](code/types.md)、[模块与外部声明](modules.md) |
| 发布站点 | [静态站点生成 (SSG)](ssg/README.md)、[Web Components 约定](components/README.md)、[CLI 与 LSP](cli.md) |
| 了解内部实现 | [设计概览](designs/README.md)、[编译与构建管线](designs/pipeline.md) |
