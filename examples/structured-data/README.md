# 构建期读取单条记录

在仓库根运行 `cargo run --example structured_data`。示例将相同的 notc 函数用于两个页面，在同一 Vault 输入批次中共享 JSON／TOML 的解析结果，打印静态 HTML。

- `page.not` 只传递字面量 Path 和记录 ID。
- `views.notc` 选择字段、显式解码，再使用正常内容函数构造正文。
- `private/` 是读取输入，不是发布清单；示例不会复制或自动发布其中的文件。
- 缺失的 description 使用空字符串；显式 null 或错误类型会失败，不能冒充缺席。
- 将 `data_expect` 换为显式匹配 Result，可为可恢复的数据错误选择回退；未准备输入和硬预算仍不可吞掉。

这不是通用遍历、整站页面枚举或 `site.data` 迁移示例。
