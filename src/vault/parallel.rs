//! Batch evaluation. Only input/cache operations serialize; evaluation
//! and output construction run outside the lock. No input snapshots are copied.
use super::*;
use crate::{EvaluationInputs, InputRead, ParsedDataCache, ResourcePath, ResourceSession};
use notist_core::{
    data::{Data, DataFailure, DataFormat, DataLimits},
    definitions::Type,
    item::Value,
};
use std::sync::Mutex;

struct Inputs<'a, R>(Mutex<&'a mut ResourceSession<R>>);
impl<R: Resources + Send> EvaluationInputs for Inputs<'_, R> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.0.lock().unwrap().file(path)
    }
    fn data_cache(&self) -> Option<&dyn ParsedDataCache> {
        Some(self)
    }
}
impl<R: Resources + Send> ParsedDataCache for Inputs<'_, R> {
    fn parse(
        &self,
        format: DataFormat,
        bytes: Arc<[u8]>,
        limits: DataLimits,
    ) -> Result<Data, DataFailure> {
        self.0
            .lock()
            .unwrap()
            .data_cache()
            .unwrap()
            .parse(format, bytes, limits)
    }
    fn decode(&self, data: &Data, target: &Type, limits: DataLimits) -> Result<Value, DataFailure> {
        self.0
            .lock()
            .unwrap()
            .data_cache()
            .unwrap()
            .decode(data, target, limits)
    }
}

impl<R: Resources + Send> Vault<R> {
    /// Evaluate independent documents using the host's shared executor.
    /// Results retain document order. One job uses the ordinary serial path.
    /// Shared resource/cache limits remain batch-wide, but exhaustion may be
    /// attributed to a different page because input requests are interleaved.
    /// Extensions must tolerate concurrent calls across independent documents.
    pub fn evaluate_documents(
        &mut self,
        documents: &[(&DocumentHandle, &Analysis)],
        executor: &crate::Executor,
    ) -> Result<Vec<Result<Evaluation, VaultError>>, VaultError> {
        for (handle, analysis) in documents {
            self.check_handle(handle)?;
            if analysis.source().id() != handle.source.id()
                || !analysis
                    .source()
                    .text()
                    .zip(handle.source.text())
                    .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
            {
                return Err(ResourceError::Access {
                    path: handle.path.clone(),
                    message: "analysis does not belong to this document handle".into(),
                }
                .into());
            }
        }
        if executor.jobs() == 1 {
            return Ok(documents
                .iter()
                .map(|(handle, analysis)| self.evaluate_document(handle, analysis))
                .collect());
        }
        let inputs = Inputs(Mutex::new(&mut self.resources));
        let pipeline = &self.pipeline;
        let limits = self.evaluation_limits;
        const BATCH: usize = 128;
        let chunks = executor.map_chunks(documents, BATCH, |_, documents| {
            documents
                .iter()
                .map(|(handle, analysis)| {
                    match pipeline.evaluate_with_inputs_and_limits(
                        analysis,
                        handle.environment.context(),
                        &inputs,
                        limits,
                    ) {
                        crate::EvaluationOutcome::Complete(output) => Ok(*output),
                        crate::EvaluationOutcome::NeedResources(paths) => {
                            Err(VaultError::NeedResources(paths))
                        }
                    }
                })
                .collect::<Vec<_>>()
        });
        Ok(chunks.into_iter().flatten().collect())
    }
}
