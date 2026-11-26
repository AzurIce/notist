@!(title: "站点扩展")

# 站点扩展

`notist-ssg` 支持在构建的两个明确边界插入原生或外部命令扩展，扩展始终与单次构建绑定，不污染跨次构建状态：

- `before_compile`：读取或修改内容源文件集合（`CompileInput.documents`），常用于筛选文档、生成虚拟页面或解析站点结构（如 `builtins.book`）。
- `before_template`：接收所有已渲染为 HTML 的只读 `Page` 列表，向 `site.data` 注入模板数据或输出额外生成文件（如搜索索引、RSS、导航树）。

## 虚拟数据文件

`before_compile` 的 `CompileInput.data_files` 提供一份只读覆盖层：键是内容目录相对路径，值是文本内容。评估期的 `read_text(...)` / `read_json(...)` 优先读取这些内存文件，无需先写入源码目录，也不会发布到输出：

```rust
input.data_files.insert(
    ".data/runtime.json".into(),
    r#"{"mode":"shadow"}"#.into(),
);
input.documents.insert(
    "index.md".into(),
    "#import \"../views/lib.not\" as view\n#view.show(\".data/runtime.json\")".into(),
);
```

命令扩展在 `before_compile` 响应中返回同名的 `data_files` 对象即可得到同样效果。

## 外部命令扩展配置

在 `Notist.toml` 中通过 `[site.extensions.<name>]` 声明命令扩展：

```toml
[site.extensions.my_ext]
command = ["python3", "scripts/ext.py"]
watch = ["scripts/ext.py"]
```

多个扩展按 `extension_order`（默认按名称字典序）顺序执行；`notist serve` 会自动监听 `watch` 声明的文件变化并触发增量重建。
