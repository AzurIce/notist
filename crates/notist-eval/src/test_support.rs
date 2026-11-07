//! Test-only access to the existing presentation oracle and invariant checks.
//! Not enabled by normal consumers; this is not an alternate evaluator API.
use super::*;

pub fn reset_checkpoint_restores() {
    CHECKPOINT_RESTORES.set(0);
}

pub fn checkpoint_restores() -> usize {
    CHECKPOINT_RESTORES.get()
}

pub fn reference_present<T>(operation: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REFERENCE_PRESENT.set(false);
        }
    }
    assert!(!REFERENCE_PRESENT.replace(true));
    let _reset = Reset;
    operation()
}

pub fn item_shell(item: &Item) -> Item {
    super::item_shell(item)
}

pub fn assert_summaries<C: EvaluationContext + ?Sized>(
    root: &Item,
    program: &CheckedProgram,
    context: &C,
    limits: Limits,
) {
    let mut evaluator = Evaluator {
        context,
        program: program.clone(),
        implementations: InstanceCache::default(),
        diagnostics: Vec::new(),
        limits,
        steps: 0,
        iterations: 0,
        loop_work: 0,
        active_loops: 0,
        depth: 0,
        mode: EvaluationMode::Runtime,
        inputs: None,
        requests: 0,
        dependencies: Default::default(),
        needed: Default::default(),
        sources: Default::default(),
        data_retention: program.data_retention.clone(),
    };
    let (bound, count) = evaluator.bind_content(
        std::slice::from_ref(root),
        &Scope::new(),
        limits.content_nodes,
    );
    assert_eq!(count, content_count(&bound), "binding: {limits:?}");
    let (presented, summary) = evaluator.present(bound, limits.content_nodes);
    assert_eq!(presented.len(), 1);
    assert_eq!(
        summary.nodes,
        content_count(&presented),
        "presentation: {limits:?}"
    );
    assert!(summary.nodes <= limits.content_nodes);
    assert_eq!(
        summary.failed,
        presented
            .iter()
            .flat_map(Item::descendants)
            .any(|item| matches!(item.evaluation, Some(EvaluationState::Error(_))))
    );
    assert!(
        presented
            .iter()
            .flat_map(Item::descendants)
            .all(|item| item.pending.is_none())
    );
}
