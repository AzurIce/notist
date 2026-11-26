@!(title: "内容模型与内置方法")

# 内容模型与内置方法

`notist-core` 采用**不可变单节点内容模型**，所有内容值底层均由 `Arc` 共享指针支撑：

- `Inline`（`Arc<InlineNode>`）：`Fragment`、`Text`、`Strong`、`Emph`、`Strike`、`Underline`、`Raw`、`Math`、`Link`、`Image`、`Custom`
- `Block`（`Arc<BlockNode>`）：`Fragment`、`Paragraph`、`Heading`、`Callout`、`CodeBlock`、`List`（含 `ordered` 与 `start`）、`Quote`、`Divider`、`Table`（含列对齐 `alignments`）、`Math`、`Div`、`SectionBox`、`Section`、`Custom`

## 核心不变式

1. **严格区分 `Inline` 与 `Block`**：`Inline` 位置静态拒绝 `Block`；代码模式下 `Inline` 不会隐式提升为 `Block`。
2. **规范化 `Fragment`**：无属性的嵌套 `Fragment` 自动展平，单元素无属性 `Fragment` 自动解包；带 `@(...)` 属性的 `Fragment` 保留容器节点。
3. **结构相等与结构共享**：`==` / `!=` 仅比较语义（`kind` 与 `attrs`），忽略源码 `span`；复用绑定或局部重写未改动子树时保持 `Arc` 指针共享。
4. **显式 `sectionize()`**：按标题层级折叠 `Section` 并提升标题属性到外层 `Section` 是 `Block` 上的纯库方法，不硬编码在编译期。

## 内置 `notist` 模块与 Prelude 构造函数

所有内置内容构造函数均注册在内置根模块 `notist` 下，既可以通过 prelude 直接调用（如 `math("x")`），也可以通过 `notist.` 限定调用（如 `notist.math("x")`）。Markup 语法糖（如 `$x$`、`$$x$$`、`**a**`、`# Title`、列表等）在检查期直接绑定到 `notist` 模块对应声明，不受局部同名绑定遮蔽。

| 函数 | 签名 |
| --- | --- |
| `text` | `(String) -> Inline` |
| `strong` / `emph` / `strike` / `underline` | `(Inline) -> Inline` |
| `raw` | `(String) -> Inline` |
| `math` | `<C: Type = Inline>(String) -> C where C == Inline || C == Block` |
| `link` | `(String, Inline) -> Inline` |
| `image` | `(String, String) -> Inline` |
| `inlines` | `(List<Inline>) -> Inline` |
| `para` | `(Inline) -> Block` |
| `heading` | `(Int, Inline) -> Block` |
| `callout` | `(String, Block) -> Block` |
| `code_block` | `(String, String) -> Block` |
| `display_math` | `(String) -> Block`（转发到 `notist.math<Block>`） |
| `quote` / `div` / `section` | `(Block) -> Block` |
| `divider` | `() -> Block` |
| `list` | `(Bool, Int, List<Block>) -> Block` |
| `table` | `(List<Inline>, List<List<Inline>>) -> Block` |
| `blocks` | `(List<Block>) -> Block` |
| `read_text` | `(String) -> String` |
| `read_json` | `(String) -> Json` |
| `panic` | `(String) -> Never` |

## 内置方法

| 接收者 | 方法 | 签名 |
| --- | --- | --- |
| `Int` / `Float` / `Bool` | `to_string` | `(self) -> String` |
| `List<T>` | `len` | `(self) -> Int` |
| `Json` | `get` / `at` | `(self, String) -> Json` / `(self, Int) -> Json` |
| `Json` | `as_string` / `as_int` / `as_bool` | 带默认回退值的类型提取 |
| `Json` | `as_list` / `len` / `is_null` | 转 `List<Json>`、长度与空值判断 |
| `Inline` | `plain_text` | `(self) -> String` |
| `Inline` | `with_attr` / `attr_or` / `has_attr` | 读写节点属性 |
| `Inline` | `rewrite` | `(self, fn(Inline) -> Inline) -> Inline`（自底向上不可变重写） |
| `Block` | `plain_text` | `(self) -> String` |
| `Block` | `sectionize` | `(self) -> Block`（按标题层级折叠为嵌套 `Section` 并提升标题属性） |
| `Block` | `heading_count` / `headings_summary` | 提取文档标题统计与摘要 |
| `Block` | `with_attr` / `attr_or` / `has_attr` | 读写节点属性 |
| `Block` | `rewrite_inlines` | `(self, fn(Inline) -> Inline) -> Block` |
| `Block` | `rewrite_blocks` | `(self, fn(Block) -> Block) -> Block` |

用户自定义同名函数或方法可以自然遮蔽内置 prelude。
