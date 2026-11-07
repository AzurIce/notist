//! Test-only verification of construction accounting and final-output invariants.
use super::*;

pub fn item_shell(item: &Node) -> Node {
    super::item_shell(item)
}

pub fn assert_summaries<C: EvaluationContext + ?Sized>(
    root: &Node,
    program: &CheckedProgram,
    sources: &notist_core::diag::Sources,
    context: &C,
    limits: Limits,
) {
    fn count(content: &Content) -> usize {
        content
            .pieces
            .iter()
            .map(|piece| match &piece.kind {
                Construction::Error => 0,
                Construction::Ready { nodes } => *nodes,
                Construction::Element(children) | Construction::Splice(children) => {
                    1 + count(children)
                }
                Construction::Call { children, .. } => 1 + count(children),
                Construction::Repeat { .. } => 1,
            })
            .sum()
    }
    let mut evaluator = Evaluator::new(
        context,
        program,
        sources,
        limits,
        None,
        EvaluationMode::Runtime,
    );
    let bound = evaluator.bind_content(
        std::slice::from_ref(root),
        &Scope::new(),
        limits.content_nodes,
    );
    assert_eq!(bound.nodes, count(&bound), "binding: {limits:?}");
    let (nodes, summary) = evaluator.present(&bound, limits.content_nodes, &SourceChain::default());
    assert!(summary.nodes <= limits.content_nodes);
    let actual = nodes
        .iter()
        .flat_map(Node::descendants)
        .filter(|n| !n.is_error())
        .count();
    assert!(
        actual <= summary.nodes,
        "dissolved output exceeds logical work: {limits:?}"
    );
    assert_eq!(
        summary.failed,
        nodes.iter().flat_map(Node::descendants).any(Node::is_error)
    );
    assert!(
        nodes
            .iter()
            .flat_map(Node::descendants)
            .all(|n| n.as_expr().is_none())
    );
}
