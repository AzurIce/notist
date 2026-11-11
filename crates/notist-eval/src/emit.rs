use super::*;

impl<C: EvaluationContext + ?Sized> Evaluator<'_, C> {
    /// Each piece retains the original forest limit. Aggregate failure discards
    /// only its output suffix, never requests, diagnostics or cumulative work.
    pub(super) fn emit_content(
        &mut self,
        content: &Content,
        limit: usize,
        expansion: &SourceChain,
        out: &mut Vec<Node>,
    ) -> ContentSummary {
        let mut summary = ContentSummary::default();
        for piece in content.pieces.iter() {
            let start = out.len();
            let added = self.emit_piece(piece, limit, expansion, out);
            if summary.nodes.saturating_add(added.nodes) > limit {
                out.truncate(start);
                out.push(self.error(
                    &piece.source,
                    "content size limit exceeded".into(),
                    expansion,
                ));
                summary.failed = true;
            } else {
                summary.append(added);
            }
        }
        summary
    }

    fn emit_piece(
        &mut self,
        piece: &Piece,
        limit: usize,
        expansion: &SourceChain,
        out: &mut Vec<Node>,
    ) -> ContentSummary {
        if matches!(piece.kind, Construction::Error) {
            let Ctor::Error(message) = &piece.source.ctor else {
                unreachable!()
            };
            out.push(self.error(&piece.source, message.clone(), expansion));
            return ContentSummary::ERROR;
        }
        if limit == 0 {
            out.push(self.error(
                &piece.source,
                "content size limit exceeded".into(),
                expansion,
            ));
            return ContentSummary::ERROR;
        }
        match &piece.kind {
            Construction::Ready { item, nodes } => {
                if *nodes > limit {
                    out.push(self.error(
                        &piece.source,
                        "content size limit exceeded".into(),
                        expansion,
                    ));
                    return ContentSummary::ERROR;
                }
                let mut node = piece.source.node(item.clone());
                expand_ready(&mut node, expansion);
                out.push(node);
                ContentSummary {
                    nodes: *nodes,
                    failed: false,
                }
            }
            Construction::Element { spec, children } => {
                self.emit_element(&piece.source, spec.clone(), children, limit, expansion, out)
            }
            Construction::Splice(content) => {
                let start = out.len();
                let mut summary =
                    self.emit_content(content, limit.saturating_sub(1), expansion, out);
                emit_mount(&piece.source, start, expansion, out);
                summary.nodes += 1;
                summary
            }
            Construction::Repeat {
                expression,
                environment,
            } => {
                let mut chain = piece
                    .source
                    .origin
                    .as_ref()
                    .map(|o| o.expansion.clone())
                    .unwrap_or_default();
                if chain.is_empty() {
                    chain = expansion.clone();
                } else {
                    chain.extend_from_slice(expansion);
                }
                let start = out.len();
                match self.repeat(
                    expression,
                    environment,
                    limit.saturating_sub(1),
                    &chain,
                    out,
                ) {
                    Ok(mut summary) => {
                        emit_mount(&piece.source, start, expansion, out);
                        summary.nodes += 1;
                        summary
                    }
                    Err(failure) => {
                        out.truncate(start);
                        out.push(self.error_at(&piece.source, failure, expansion));
                        ContentSummary::ERROR
                    }
                }
            }
            Construction::Call {
                call,
                environment,
                children,
            } => self.emit_call(
                &piece.source,
                call,
                environment,
                children,
                limit,
                expansion,
                out,
            ),
            Construction::Error => unreachable!(),
        }
    }

    pub(super) fn emit_element(
        &mut self,
        source: &NodeMetadata,
        spec: ElementSpec,
        children: &Content,
        limit: usize,
        expansion: &SourceChain,
        out: &mut Vec<Node>,
    ) -> ContentSummary {
        let mut nodes = Vec::with_capacity(children.pieces.len());
        let mut summary =
            self.emit_content(children, limit.saturating_sub(1), expansion, &mut nodes);
        let mut node = source.node(ContentItem {
            ctor: spec.ctor,
            fields: spec.fields,
            children: Shared::from_nodes(nodes),
            instance: spec.instance,
        });
        append_expansion(&mut node, expansion);
        if !valid_structure(&node) {
            out.push(self.error(
                source,
                "evaluated content violated its checked structural role".into(),
                expansion,
            ));
            return ContentSummary::ERROR;
        }
        out.push(node);
        summary.nodes += 1;
        summary
    }
}
