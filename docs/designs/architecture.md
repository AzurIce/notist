#import "../components/lib.notc" as components;

# Vault 在构建管线中的位置

`Vault` 是宿主的一次输入会话。一次站点构建共用一个 `Vault`：各页共享已读取的模块，但有各自的源码和 import 作用域。`Vault` 准备这些输入，`Pipeline` 用它们检查和求值。

#components.diagram(r#"""
flowchart TD
    entry["SSG：发现页面，运行入口扩展"] -->|路径和源码| lower
    subgraph vault["Vault：一次构建的输入会话"]
        lower["Pipeline：解析源码，提取 imports"] --> env["Vault：加载配置和模块，绑定本页的名字"]
        env --> check["Pipeline：检查文档"]
        check --> ready["Analysis + DocumentHandle"]
        ready --> eval["Pipeline：从 Analysis 继续求值"]
        inputs["ResourceSession：按权限读取资源"] --> eval
    end
    eval -->|求值树| html["HTML 后端与 SSG：生成页面、套用模板、发布"]
"""#)

`Analysis` 是检查结果；`DocumentHandle` 记住它所属的 `Vault`、源码和环境。编辑器可以停在分析阶段；构建则用这对结果继续求值，不必重新解析或检查入口。

**并发边界：** SSG 逐页分析，因为准备环境需要可变访问 `Vault`。所有入口源码登记完毕后，各页可以并行求值；即使一页读取另一页的源码，也能读到本轮扩展给出的文本。

## Vault 持有哪些状态？

以 A、B 都导入 C 为例：同一批次可以复用 C 的模块图，但 A、B 各有自己的环境。图中箭头表示持有或引用；`DocumentHandle` 由调用方持有。

#components.diagram(r#"""
flowchart LR
    vault["Vault：本轮构建"] --> inputs["resources: ResourceSession<R>"]
    vault --> shared["environment_cache：配置与模块图"]
    vault --> index["environments：按路径和源码查环境"]
    index --> a["A 的 Environment"]
    index --> b["B 的 Environment"]
    handle["A 的 DocumentHandle"] --> a
"""#)

| 状态 | 存什么、何时使用 |
| --- | --- |
| `pipeline: Pipeline`、`extensions: Vec<Arc<dyn DocumentExtension>>` | 选择文档前端，注册文档扩展。 |
| `config: Option<PathBuf>`、`evaluation_limits: Limits`、`module_urls: BTreeMap<PathBuf, String>` | 指定配置文件（未指定时就近查找）、单次求值限额，以及 HTML 绑定使用的模块 URL。 |
| `resources: ResourceSession<R>` | 保存本轮的输入、读取权限和计账，见下节。 |
| `environment_cache: EnvironmentCache` | 按配置路径缓存解析结果；按入口目录、导入路径列表和配置路径缓存成功装配的模块图。各页的 import 别名仍单独绑定。 |
| `environments: BTreeMap<PathBuf, BTreeMap<Arc<str>, Arc<Environment>>>` | 两层键分别是文档路径和确切源码。值包含选中的配置路径、模块来源和 `Context`。`DocumentHandle` 引用同一份环境。 |
| `identity: Arc<()>` | 用指针身份标记句柄所属的 `Vault`。替换管线、指定配置或追加文档扩展时，`Vault` 清空两层环境缓存并更换标记。 |

**生命周期：** 新一轮构建或编辑创建新的 `Vault`，重新观察输入。`Analysis` 和求值树作为结果返回给调用方；局部变量和调用栈属于单次求值，都不是 `Vault` 的跨页状态。

### ResourceSession：一次构建的输入记录

`Vault<R = FsResources>` 持有一个 `ResourceSession<R>`。`R: Resources` 是宿主提供的资源后端，负责实际读取；`ResourceSession<R>` 包装这个后端，记住本轮已经读到了什么。它按需读取，不在构建开始时扫描并复制整个文件系统。

```rust
pub struct ResourceSession<R> {
    access: InputAccess<R>,                           // 后端、读取根、文件缓存和预算
    documents: DocumentInputs,                        // 宿主提交的文档源码
    answers: RefCell<HashMap<ResourceId, InputRead>>,  // 已返回的求值读取结果
    dependencies: RefCell<BTreeSet<ResourcePath>>,    // 逻辑数据读取记录
    data_cache: DataCache,                            // JSON/TOML 解析和解码结果
}
```

`InputAccess` 的文件缓存只保存后端读到的结果：字节或错误、文件类型、UTF-8 解码结果。`DocumentInputs` 是独立的 `HashMap<PathBuf, Arc<str>>`，可以保存扩展改写或没有落盘的文档。第一次求值读取优先使用登记的源码；结果一旦返回，后续读取不再受文档登记影响。

同一会话向管线提供两个接口：

- **加载模块**用 `Resources::source_shared(&self, path: &Path) -> Result<Arc<str>, ResourceError>` 取得 UTF-8 源码。读取模块不会给求值器增加数据读取权限。
- **求值读数据**用 `EvaluationInputs::file(&self, path: &ResourcePath) -> InputRead`。结果是 `Ready(Arc<[u8]>)`、`Failed(InputError)` 或 `Pending`（需要宿主准备输入）。例如两页都执行 `read_json` 读取同一文件，每次请求都计数、检查读取根；命中缓存时可共享同一份字节和解析结果。
