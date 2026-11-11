use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Counting;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
fn record() {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations(run: impl FnOnce()) -> usize {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.with(|flag| flag.set(false));
        }
    }
    ALLOCATIONS.with(|n| n.set(0));
    ACTIVE.with(|flag| assert!(!flag.replace(true)));
    let guard = Guard;
    run();
    drop(guard);
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn empty_content_construction_clone_and_drop_do_not_allocate() {
    assert_eq!(
        allocations(|| {
            let content = Content::default();
            drop(content.clone());
            drop(content);
        }),
        0
    );
}

#[test]
fn ready_leaf_expansion_does_not_allocate_child_storage() {
    let owner = notist_core::source::Source::metadata("leaf.not").unwrap();
    let suffix: SourceChain = vec![SourceLocation {
        source: owner.id(),
        start: 1,
        end: 3,
    }]
    .into();
    let mut leaf = Node::new(Ctor::Divider, Default::default());
    leaf.origin = Some(Origin {
        source: owner.id(),
        start: 0,
        end: 1,
        expansion: Default::default(),
    });
    let retained = leaf.clone();
    assert_eq!(allocations(|| expand_ready(&mut leaf, &suffix)), 0);
    assert!(retained.origin.as_ref().unwrap().expansion.is_empty());
    assert_eq!(leaf.origin.as_ref().unwrap().expansion, suffix);
}

#[test]
fn single_content_has_one_allocation_and_replays_shared_storage() {
    let node = Node::text("one".into(), Default::default());
    let piece = Piece {
        source: NodeMetadata::from(&node),
        kind: Construction::Ready {
            item: node.as_item().unwrap().clone(),
            nodes: 1,
        },
    };
    assert_eq!(
        allocations(|| {
            let content = Content::one(piece, 1);
            let copy = content.clone();
            assert!(std::ptr::eq(&content.pieces[0], &copy.pieces[0]));
            drop(copy);
            drop(content);
        }),
        1
    );
}

struct Context;
impl EvaluationContext for Context {
    fn has_mappings(&self) -> bool {
        false
    }
    fn registry(&self) -> &notist_core::registry::Registry {
        notist_core::builtins::registry()
    }
    fn resource_root(&self) -> &str {
        "/"
    }
    fn target(
        &self,
        id: &notist_core::definitions::FunctionId,
    ) -> notist_core::definitions::FunctionId {
        id.clone()
    }
    fn specialize(
        &self,
        _: &notist_core::definitions::FunctionInstance,
        p: CheckedProgram,
    ) -> (CheckedProgram, Vec<Diagnostic>) {
        (p, Vec::new())
    }
}

#[test]
fn binding_single_ready_piece_does_not_allocate_a_temporary_vector() {
    let context = Context;
    let node = Node::text("ready".into(), Default::default());
    let program = CheckedProgram::default();
    let sources = Default::default();
    let scope = Scope::new();
    let mut e = Evaluator::new(
        &context,
        &program,
        &sources,
        Limits::default(),
        None,
        EvaluationMode::Runtime,
    );
    assert_eq!(
        allocations(|| drop(e.bind_content(&[node], &scope, 100))),
        1
    );
}

#[test]
fn fixed_scalar_arguments_need_no_dictionary_or_heap_frame() {
    let context = Context;
    let mut definition = context
        .registry()
        .function("notist", "str")
        .unwrap()
        .clone();
    definition.parameters[0].ty = Type::Int;
    let instance = FunctionInstance {
        function: definition.id.def_id(),
        arguments: vec![ComptimeValue::Type(Type::Int)],
    };
    let expression = TypedExpr {
        ty: Some(Type::Int),
        known: Some(ComptimeValue::Data(Value::Int(7))),
        kind: ExprKind::Constant(ComptimeValue::Data(Value::Int(7))),
        span: Default::default(),
        source: Default::default(),
        shape: Default::default(),
    };
    let call = Invocation::new(instance, definition, vec![("value".into(), expression)]);
    let program = CheckedProgram::default();
    let sources = Default::default();
    let scope = Scope::new();
    let mut e = Evaluator::new(
        &context,
        &program,
        &sources,
        Limits::default(),
        None,
        EvaluationMode::Runtime,
    );
    assert_eq!(
        allocations(|| drop(ArgumentValues::evaluate(&mut e, &call, &scope).unwrap())),
        0
    );
}

#[test]
fn report_private_execution_layout() {
    println!(
        "execution-layout Content={} Piece={} Runtime={} Construction={} ArgumentValues={}",
        std::mem::size_of::<Content>(),
        std::mem::size_of::<Piece>(),
        std::mem::size_of::<Runtime>(),
        std::mem::size_of::<Construction>(),
        std::mem::size_of::<ArgumentValues<'_>>()
    );
    assert!(std::mem::size_of::<Content>() <= 2 * std::mem::size_of::<usize>());
}
