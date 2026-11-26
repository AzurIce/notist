@!(title: "类型、泛型与方法")

# 类型、泛型与方法

`notist-core` 的类型系统追求**小而彻底**：没有 Traits、没有联合类型 `A | B`、没有隐式子类型，全部靠一等函数、显式类型关联方法和编译期类型泛型表达。

## 显式类型关联方法（非 UFCS）

方法必须显式绑定在接收者类型上，不会污染普通函数命名空间；非结构体类型不暴露假字段，统一通过方法访问：

```not
fn Block.banner(self) -> Block {
    callout("banner", self)
}

fn build() -> Block {
    [ Hello ].banner().sectionize()
}
```

方法解析按接收者静态类型查找，并支持通过 `import` 跨模块传播（同名冲突静态报错）。

## 编译期类型泛型：`<T: Type>` + `where` + `match T`

`notist-core` 用三个职责正交的机制实现多态内容函数：

1. **类型参数 `<C: Type>` 与默认值 `<C: Type = Default>`**：将类型作为编译期参数传递；调用时可自动推断或回退到尾部默认类型参数（如 `fn math<C: Type = Inline>(text: String) -> C` 默认返回 `Inline`，显式写 `math<Block>(...)` 返回 `Block`），也可显式实例化为一等单态函数值（如 `decorate<Inline>: fn(Inline) -> Inline`）。
2. **`where` 调用点守卫**：只负责在调用点与特化点校验类型前提（支持 `==`、`!=`、`&&`、`||`、`!`），不参与函数体内部的隐式类型推导。
3. **`match C` 显式收窄**：仅在 `match C` 的各分支词法作用域内收窄 `C`；`if` 永不收窄类型。每个分支在定义点都会被静态检查，即使函数尚未被调用。
4. **`compile_error("...")`**：类型为 `Never`，用于在不可达兜底分支提供明确的编译期错误。

```not
fn decorate<C: Type>(content: C) -> C
where C == Inline || C == Block
{
    match C {
        Inline => strong[⚡ #content],
        Block  => callout("highlight")[
            #content
        ],
        _ => compile_error("unreachable: guarded by where"),
    }
}
```

## 定义点全量检查与可达特化

所有模块中未调用的函数、泛型函数和 `match` 分支都会在编译期全部做静态类型检查；执行阶段仅对入口可达的函数与泛型特化实例做单态化与 lowering。
