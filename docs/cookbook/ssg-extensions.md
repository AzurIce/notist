@!(title: "编写 SSG 扩展")

# 编写 SSG 扩展

通过外部命令扩展可以在不修改 `notist-ssg` 源码的情况下向站点注入页面与模板数据。扩展进程通过标准输入/输出收发 JSON 帧：

1. `before_compile` 事件接收当前 `documents` 映射，返回新增或修改后的 `.md` 源文本。
2. `before_template` 事件接收全部页面的标题、路由、标题大纲与 HTML，返回要写入 `site.data` 的结构化数据与附加输出文件。

在 `Notist.toml` 中注册后，`notist build` 与 `notist serve` 会以完全一致的协议调用扩展，并监听 `watch` 列表中的脚本变化。
