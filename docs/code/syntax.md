@!(title: "Code 语法")

# Code 语法（`.not`）

`.not` 是用于组织文档逻辑、封装复用组件和编写内容变换的静态类型函数式语言。顶层仅允许三类声明：`import`、函数 `fn` 与类型关联方法 `fn Type.method`。

## 基础类型与字面量

- `Unit`：`()`（无列表上下文时表示 `Unit`；处于 `List<T>` 期望上下文时表示空列表）
- `Bool`：`true`、`false`
- `Int`：64 位有符号整数（如 `42`、`1_000`），算术运算带溢出与除零检查
- `Float`：64 位浮点数（如 `3.14`）
- `String`：支持普通双引号 `"..."`、多行 `"""..."""` 与原始字符串 `r#"..."#` / `r#"""..."""#`；提供 `.starts_with(prefix)` / `.ends_with(suffix)` / `.contains(needle)` / `.replace(from, to)` / `.trim()` 方法
- `List<T>`：Typst 风格圆括号列表——空列表 `()`（按上下文推断元素类型）、单元素 `(x,)`（带尾逗号以区分分组 `(x)`）、多元素 `(a, b, c)`；支持 `+` 拼接、`==`/`!=`、`.len()` 与 `for x in list { body }`
- `Json`：宿主只读 JSON 数据，通过 `read_json(path)` 读取，提供 `.get(key)` / `.at(idx)` / `.as_string(fb)` / `.as_int(fb)` / `.as_bool(fb)` / `.as_list()` / `.len()` / `.is_null()`
- `Inline` / `Block`：通过 flank 规则 `[inline]` 与 `[ block ]` 直接嵌入 Markdown 内容字面量（内部支持 `#name`、`#(expr)`、`#fn(...)`、`#fn[...]` 插值）

## 表达式与运算符

- 局部变量绑定：`let x = expr; body`（词法作用域，支持遮蔽）
- 顺序求值：`expr1; expr2`
- 条件分支：`if cond { a } else if other { b } else { c }`（各分支类型须一致；`else if` 可连续串联）
- 列表推导：`for item in list { body }`（返回 `List<Body>`）
- 运算符：`!`、`-`、`*` `/` `%`、`+` `-`、`<` `<=` `>` `>=`、`==` `!=`、短路 `&&` / `||`
- 内容拼接与比较：`Inline + Inline`、`Block + Block`、`==`、`!=`

## 函数与尾随内容语法糖

内容参数就是普通位置参数；调用处紧贴参数列表或函数名的 `[...]` 会自动作为下一个位置参数传入：

```not
fn note(kind: String, body: Block) -> Block {
    callout(kind)[
        #body
    ]
}

fn demo() -> Block {
    note("tip")[
        等价于 `note("tip", [ ... ])`。
    ]
}
```

## 参数默认值

参数可以带默认值；带默认值的参数必须位于参数列表末尾，调用时可省略任意一段尾部参数：

```not
fn note(kind: String, title: String = "", body: Block = [ ]) -> Block {
    callout(kind)[
        #strong(text(title))
        #body
    ]
}

fn demo() -> Block {
    let _full = note("tip", "Title", [ Body ]); // 显式传入全部参数
    note("tip")                                 // title 与 body 都使用默认值
}
```

默认值在**被调函数的帧内**求值，因此可以引用声明在它之前的参数（如 `right: String = left + "]"`），但不能引用自身或之后的参数。默认值参数与类型参数默认值相互独立：前者在调用值层面补齐，后者在编译期特化时补齐。

