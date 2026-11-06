# IceXIV 临时对象 / Vault 释放归因

这是上一轮全量 profile 的后续分析。仍固定 Notist `85ab69e` 与同一份完整 IceXIV 输入，未修改生产代码。这里的“临时对象”指一轮构建的中间产物，不专指 Rust 的临时表达式。

## 1. 两个释放边界在哪里

原顺序 coarse 对照：

- `crates/notist-ssg/src/build.rs::build` 返回时，SSG 局部对象销毁 / 尾部约 **1.398s**。
- 随后 `crates/notist-cli/src/ssg.rs::assemble` 返回时，局部 `vault` 销毁约 **1.692s**。
- 合计约 **3.090s**，都发生在 `serve` 的 `Updated` 计时结束前。

需要区分三条 Rust 规则：局部变量一般按声明的逆序释放；结构体字段按声明顺序释放；`Arc` 只有最后一个强引用释放时才销毁其拥有的内容。一个对象在哪个语义阶段建立，不决定最后释放成本落在哪个 Drop。

以下源码位置均指冻结的 `85ab69e`，不是测量期间并行修改的工作区。

## 2. SSG 函数返回：主要清理整站求值树

### 最大的中间产物是 `documents: Vec<Document>`

`crates/notist-ssg/src/build.rs:545` 建立 `documents`，逐页求值后在 573 行把 `evaluation.root` 移入 Document。这个 Vec 一直保留到函数返回，包含 51,739 个文档。

`crates/notist-ssg/src/document.rs:29`：

```rust
struct Document {
    page: Page,
    root: Item,
    source: String,
    sources: Sources,
    ids: BTreeSet<String>,
}
```

最值得关注的是 `root: Item`。`crates/notist-core/src/item.rs:384` 的每个节点包含：

- `children: Vec<Item>`，递归释放整棵子树；
- `fields` / `attrs: Dict`，减少 Arc 引用，最后引用还要清理 DictStorage 的 entries、索引、值；
- `instance` / `target` 的函数身份和参数；
- `origin` 的来源字符串及 expansion 路径；
- 可能存在的 pending 表达式 / 调用信息，实际是否存在取决于节点的求值状态。

这不是释放一个大 HTML 字符串，而是走遍大量节点、容器和引用，产生大量细粒度 deallocation。`present` 的 849 万次调用数不是最终树节点数，不能直接拿来当释放节点数。

主探针显式 Document Drop 约 **1.331s**；原顺序下全部 SSG 尾部约 **1.398s**，两者相符，但前者改变了部分释放顺序，所以不能把 1.331s 当原顺序的精确子项。

### 其余 SSG 局部对象

| 对象 | 位置 / 持有内容 | 已有显式探针量级 |
| --- | --- | ---: |
| `values: TemplateContext` | build.rs:697，Tera 的共享页面/目录/site 数据值，包括序列化后的页面 HTML | 约 65ms |
| `by_source: DocumentIndex` | build.rs:601，source/output 到 `&Document` 的索引；只引用 Document，不拥有 Item 树 | 约 2.4ms |
| `handles: Vec<DocumentHandle>` | build.rs:564，逐文档 path、source Arc、environment Arc、owner Arc | 见下面的归因陷阱，不能采用原探针 1.638s 作自身成本 |
| `dirs` / `components` / `engine` | 目录元数据、组件 URL、Tera 引擎 | 本工作负载中很小 |
| `outputs` / `source_dirs` 等 | 路由集合、路径列表 | 未独立精确分配总尾部中的耗时 |

正常返回时，`handles` 先于 `documents` 释放，但 Environment 仍被 Vault 的 `environments` 表持有，所以释放 handle 不等于销毁全部环境。

### 哪些东西不属于这个返回尾部

- `analysis` 在求值循环每轮结束时已经释放，算在 pipeline 阶段，不是全部留到最后。
- 每页的 `resolved`、`resource_urls`、registry / environment 临时引用，在正文渲染循环中释放，算在 bodies 阶段。
- build.rs:698 的 `drop(data)` 释放的是站点扩展的 `site.data` 元数据，不是 `.data/*.json` 的核心 DataCache；它算在共享模板上下文阶段。
- `assets.finish()` 的文件字节，以及 `pages`、`warnings`，被移入返回的 `BuildOutput`，没有在这里销毁。`Document.page` 被 clone 为输出 Page 后填入渲染 HTML；不能声称原 Document 自己还持有另一份相同的已渲染 HTML。

## 3. Vault 返回：字段级实测

`crates/notist-cli/src/ssg.rs:35` 新建 Vault，43 行完成 Builder，51–52 行函数返回。Vault 没有专门的自定义 Drop；它的所有字段正常递归销毁。

`src/vault.rs:98` 的字段声明顺序：

```text
resources: ResourceSession
pipeline
extensions
environments
environment_cache
identity
config
module_urls
evaluation_limits
```

补充探针在一次性的源码副本中解构 Vault，保持这些字段的原始释放顺序。ResourceSession 的字段和内部 State 也保持原顺序。每个版本 1 次 warmup + 3 次正式全量构建，并逐文件验证完整输出一致。

**计时记录预分配后的三次均值：**

| 路径 | 耗时 | 实际持有内容 |
| --- | ---: | --- |
| **Vault 总销毁** | **1,707.7ms** | 下面各项的父范围，不重复相加 |
| ↳ `resources` | **1,654.3ms** | 本批次 ResourceSession |
| ↳↳ `data_cache` | **1,650.3ms** | parsed JSON 文档、解析字节 Arc、复合投影结果 |
| ↳↳ `state` | 3.90ms | 源码、数据字节/失败结果、依赖、文本缓存 |
| ↳↳ `roots` | 0.053ms | rooted reader 与该授权根的输入读缓存 |
| ↳ `environments` | **53.2ms** | 51,739 个逐文档环境及其引用 |
| ↳ `environment_cache` | 0.252ms | 3 份图缓存、1 份配置缓存 |
| ↳ 其它字段 | 很小 | pipeline、扩展、路径、限制等 |

DataCache 的三次字段耗时是 **1,647.8 / 1,692.8 / 1,610.3ms**；Vault 总耗时是 **1,712.3 / 1,748.1 / 1,662.8ms**。都是诊断时间，不是新的无插桩性能样本。

### ResourceSession 的缓存数量

在本轮销毁边界观察到：

- `state.files`：568 项（固定输入字节 / 失败结果及 reader 引用）。
- `state.sources`：51,739 项（注册的生成文档源码 Arc）。
- `state.texts`：2 项；`state.kinds`：0 项。
- `state.dependencies`：565 项，对应实际读取的分块/目录数据。
- `Vault.environments`：51,739 项。
- `EnvironmentCache.graphs`：3 项；`configs`：1 项。

因此“Vault 有 51,739 个环境，所以这 1.7s 全是环境销毁”不符合本次字段探针：环境表只有约 **53ms**。

`src/resource_session.rs` 的 `state.sources` 子项约 **3.38ms**，`state.files` 约 **0.44ms**。静态图标的主要输出字节已经移入 `BuildOutput.files`，不能把整份 333.7 MB 静态资源都说成在 Vault 中被释放了。

## 4. 最关键：DataCache 是慢路径落点，不等于其自身固有成本

我们做了两组相同字段顺序、相同完整输出的探针：

| 观察 | 临界期首次创建计时记录 | 在构建前预建所有计时记录 |
| --- | ---: | ---: |
| Vault 总销毁 | 1,735.0ms | 1,707.7ms |
| ResourceSession 的字段之外 Self | **1,618.1ms** | **0.005ms** |
| DataCache 直接 Drop 范围 | **24.1ms** | **1,650.3ms** |
| 环境表 Drop | 50.3ms | 53.2ms |

第一组的 `Span::drop` 在读取 elapsed 之后，还会往 BTreeMap 中创建第一次出现的计时记录。那次记账可能分配内存；其耗时不算进刚结束的子范围，而进入父 Self。约 1.6s 因而落在记账边界，而不是任何被测字段内部。

第二组在构建开始前预建全部 metric / edge 记录，并预留计时栈容量，避免临界期第一次记账的分配；相同量级的耗时随后落到 DataCache 的实际 Drop。DataCache 清理操作没有被删减，整轮 Vault 耗时也没有实质消失。

结合上一轮“1.638s 落在 handle Drop、改变顺序后迁移到 Vault”的观察，这说明约 **1.6s 的成本对分配/释放历史高度敏感**。大量 Item 树销毁之后，下一次适当的分配或释放，可以承担之前积累的分配器整理工作。

**已证实：耗时可以跨对象及计时记账边界迁移。**

**尚未证实：具体是哪一个 glibc 内部函数、哪一次 free、多少时间是 bin 合并/扫描/trim。** 当前没有分配器调用栈，不能直接把这些子因当成已采样结论，也不能简单用两组时间相减，宣称 DataCache 在真实运行中的“净析构恰好只有 24ms”。

`crates/notist-core/src/data/cache.rs` 中 DataCache 拥有 parsed / projected 两张表；`data/mod.rs` 中共享 Document 拥有 `Vec<Node>`、字符串、数组索引、对象索引等。最后引用消失时会实际释放这些数据，因而具备触发分配器慢路径的条件。565 次 parse 不是 565 个小对象，更不意味着不存在大量节点；但当前证据也不支持把 1.65s 都归咎于 JSON DOM 自身遍历。

## 5. serve 中还有一处释放，但不在 Updated 数字里

`crates/notist-cli/src/serve.rs:279` 先记录 `started.elapsed()`，280 行再做：

```rust
published.site = Some(Arc::new(result.artifact));
```

这里会减少旧站点 Arc 的引用；最后一个引用消失时释放旧 `BuildOutput.files` / 页面元数据。更准确地说，当前 HTTP handler 并没有 clone 整站 Arc：它在读锁下 clone 所请求文件的 bytes，然后释放读锁再响应。因此当前请求路径不会因持有整站 Arc 而延后旧站点销毁；替换时通常就会销毁旧站点。这不是前述约 3.1s 的中间对象/Vault 销毁，也不包含在当次 `Updated` 显示数字里。

## 6. 对优化方向的修正

1. **先关注 Item 树产生的大量细粒度分配与释放。** compact tree、来源信息共享、减少中间物化、批量生命周期分配，都比根据某个 Drop 标签重写业务缓存更有依据。
2. **不要因 DataCache 字段计时 1.65s 就删掉 DataCache。** 它避免了 51,173 次重复 JSON parse；当前慢路径还明显受之前的堆状态影响。
3. **不要把释放移出 Updated 当优化。** 那会改显示数字，不一定改善总 CPU、延迟、RSS 或下一轮重建。
4. 下一步若要实施优化，应先用分配器/CPU 栈证据细分这段可迁移的 1.6s，再做正常二进制整轮计时及 RSS 对照。可研究分配器替换作为诊断变量，但不在没有证据时承诺特定 allocator 能省多少时间。

## 证据与复现

- 工具：`tools/profile_icexiv_teardown.py`。
- 测试：`tools/test_profile_icexiv_teardown.py`。
- 原结果目录下新增 `teardown.json` / `teardown-prewarm.json`、对应 summary、源码、二进制、patch 与重现脚本；主批次不重写。

```sh
python3 tools/profile_icexiv_teardown.py --out /tmp/opencode/notist-icexiv-profile-20261104
python3 tools/profile_icexiv_teardown.py --out /tmp/opencode/notist-icexiv-profile-20261104 --prewarm
```

上述命令只用于已经完成 ordered 基线、尚未生成这两个补充 cohort 的结果目录；本次已有结果不可覆盖重跑。新增 8 次完整 CLI 构建，逐次输出 SHA-256 核对通过。普通计时探针和预分配探针始终是分开的诊断组，不视为前后优化对比。
