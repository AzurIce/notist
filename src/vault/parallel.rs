//! Opt-in batch experiment. Only input/cache operations serialize; evaluation
//! and output construction run outside the lock. No input snapshots are copied.
use super::*;
use crate::{EvaluationInputs, InputRead, ParsedDataCache, ResourcePath, ResourceSession};
use notist_core::{
    data::{Data, DataFailure, DataFormat, DataLimits},
    definitions::Type,
    item::Value,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

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
    /// Experimental concurrent page evaluation. Results retain document order.
    /// Shared resource/cache limits remain batch-wide, but exhaustion may be
    /// attributed to a different page because input requests are interleaved.
    /// Extensions must tolerate concurrent calls across independent documents.
    pub fn evaluate_documents_parallel(
        &mut self,
        documents: &[(&DocumentHandle, &Analysis)],
        workers: usize,
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
        let inputs = Inputs(Mutex::new(&mut self.resources));
        let pipeline = &self.pipeline;
        let limits = self.evaluation_limits;
        let next = AtomicUsize::new(0);
        const BATCH: usize = 128;
        let run = || {
            let mut chunks = Vec::new();
            loop {
                let start = next.fetch_add(BATCH, Ordering::Relaxed);
                if start >= documents.len() {
                    break;
                }
                let output = documents[start..(start + BATCH).min(documents.len())]
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
                    .collect::<Vec<_>>();
                chunks.push((start, output));
            }
            chunks
        };
        let workers = workers
            .clamp(1, 8)
            .min(documents.len().div_ceil(BATCH).max(1));
        let mut chunks = if workers == 1 {
            run()
        } else {
            std::thread::scope(|scope| {
                let tasks: Vec<_> = (0..workers).map(|_| scope.spawn(&run)).collect();
                tasks
                    .into_iter()
                    .flat_map(|task| task.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                    .collect::<Vec<_>>()
            })
        };
        chunks.sort_by_key(|(start, _)| *start);
        Ok(chunks.into_iter().flat_map(|(_, output)| output).collect())
    }
}
