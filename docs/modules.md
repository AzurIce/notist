@!(title: "模块与外部声明")

# 模块与外部声明

每个 `.not` 文件是一个模块，模块间通过相对路径静态构成有向无环图（DAG），循环依赖在编译期报错。

## 六种 `import` 形式

```not
import "./math.not";                 // 默认以文件 stem `math` 作为命名空间
import "./math.not" as m;            // 显式命名空间别名
import "./math.not": inc;            // 导入单个函数
import "./math.not": inc, add as s;  // 导入多个并重命名
import "./math.not": *;              // 通配导入所有公开绑定
```

在 `.md` 文档中使用 `#import`（分号可选），规则完全相同。模块中定义的类型关联方法会自动随 `import` 传播到导入方作用域。

## 无函数体的外部内容声明

当一个内容节点需要交由下游后端（例如 SSG Web Component、PDF 渲染器等）按约定处理时，可以在 `.not` 中只写类型签名，以分号结尾：

```not
fn badge(label: String, count: Int) -> Inline;
fn panel(title: String, body: Block) -> Block;
fn diagram(source: String) -> Block;
```

- 外部声明必须返回 `Inline` 或 `Block`（或受限的类型参数），参数不能为函数类型。
- `notc` 内核对调用做完整的静态类型检查，并在求值时产出纯数据节点 `CustomNode { module, name, props }`。
- `notc` 本身完全不感知 HTML、JS 或 Web Components；具体呈现完全交由渲染后端决定。
