@!(title: "设计概览")

# 设计概览

Notist 的架构以独立、极简的语言内核 `crates/notc` 为中心，遵循三条核心原则：

1. **前端无关 IR，统一文档与代码**：`.md` 文档通过 `DocumentFrontend` 边界降级为与 `.not` 模块相同的 `notc::Module` IR，共享一套三层静态检查器与求值器；未来接入其他文档前端也无需改动编译器核心。
2. **语言内核与渲染/宿主彻底解耦**：`notc` 零文件系统 IO、不感知 HTML / JS / Web Components；文件监听、静态资源、HTML 渲染与 Web Component 加载全部位于宿主与 SSG 层。
3. **面向工具链的分层设计**：解析、接口提取、模块体检查与可达特化明确分层，使 `notc` CLI、SSG 与 LSP 能在不求值且源码不完整的情况下稳定提供类型感知能力。

详细设计见：[编译与构建管线](pipeline.md)、[Crate 架构](crates.md)。
