#import "./lib.not" as demo;
#import "./lib.not": decorate as mark;
#import "./labels.not": *;

# Code functions

本页显式导入 `lib.not`；该模块通过相对路径 `import` `labels.not`。模块绑定沿成员访问公开；本页同时演示成员重命名和通配导入，Code 函数 `tag` 通过重命名的成员调用原 `label` 定义，不需要 `Notist.toml`。

#mark[Inline content]

#demo.panel[Inline input becomes an explicit paragraph.]

#demo.decorate[
# A block forest

Existing paragraphs and sections remain intact.
]

#label("A runtime argument", true)

#demo.tag("A directly imported function")

#demo.labels.label("Hidden content", false)
