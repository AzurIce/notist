//! A host-owned concurrency budget shared by all build stages.
use std::{io, num::NonZeroUsize};

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

/// One reusable worker pool, not a separate pool or limit for each stage.
///
/// Hosts may share an executor across builds or preview rebuilds. Ordered chunk
/// collection preserves input order even when tasks complete out of order.
/// External processes and their own worker threads are outside this budget.
#[derive(Debug)]
pub struct Executor {
    jobs: NonZeroUsize,
    #[cfg(not(target_arch = "wasm32"))]
    pool: rayon::ThreadPool,
}

impl Executor {
    pub fn new(jobs: NonZeroUsize) -> io::Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.get())
            .thread_name(|i| format!("notist-worker-{i}"))
            .build()
            .map_err(io::Error::other)?;
        #[cfg(target_arch = "wasm32")]
        if jobs.get() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "parallel build execution is unavailable on wasm32",
            ));
        }
        Ok(Self {
            jobs,
            #[cfg(not(target_arch = "wasm32"))]
            pool,
        })
    }

    /// Use the available CPU count, or one worker when it is unavailable.
    pub fn auto() -> io::Result<Self> {
        #[cfg(not(target_arch = "wasm32"))]
        let jobs = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
        #[cfg(target_arch = "wasm32")]
        let jobs = NonZeroUsize::MIN;
        Self::new(jobs)
    }

    pub fn jobs(&self) -> usize {
        self.jobs.get()
    }

    /// Map consecutive chunks using this executor, returning results in order.
    /// The callback receives the chunk's start offset in the original slice.
    /// Nested calls use the same pool and do not create another worker budget.
    ///
    /// # Panics
    /// Panics if `chunk_size` is zero, or if a callback panics.
    pub fn map_chunks<T: Sync, U: Send>(
        &self,
        input: &[T],
        chunk_size: usize,
        map: impl Fn(usize, &[T]) -> U + Send + Sync,
    ) -> Vec<U> {
        assert!(chunk_size > 0, "chunk size must be positive");
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pool.install(|| {
                input
                    .par_chunks(chunk_size)
                    .enumerate()
                    .map(|(i, chunk)| map(i * chunk_size, chunk))
                    .collect()
            })
        }
        #[cfg(target_arch = "wasm32")]
        input
            .chunks(chunk_size)
            .enumerate()
            .map(|(i, chunk)| map(i * chunk_size, chunk))
            .collect()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    #[test]
    fn concurrent_stages_share_one_budget_and_reuse_workers() {
        let executor = Executor::new(NonZeroUsize::new(2).unwrap()).unwrap();
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let threads = Mutex::new(HashSet::new());
        let input: Vec<_> = (0..32).collect();
        thread::scope(|scope| {
            for _ in 0..4 {
                let executor = &executor;
                let active = &active;
                let maximum = &maximum;
                let threads = &threads;
                let input = &input;
                scope.spawn(move || {
                    let result = executor.map_chunks(input, 1, |start, chunk| {
                        let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum.fetch_max(count, Ordering::SeqCst);
                        threads.lock().unwrap().insert(thread::current().id());
                        thread::sleep(Duration::from_millis(1));
                        active.fetch_sub(1, Ordering::SeqCst);
                        assert_eq!(start, chunk[0]);
                        chunk[0]
                    });
                    assert_eq!(&result, input);
                });
            }
        });
        assert!(maximum.load(Ordering::SeqCst) <= 2);
        assert!(threads.lock().unwrap().len() <= 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn nested_work_is_ordered_and_single_worker_does_not_deadlock() {
        let executor = Executor::new(NonZeroUsize::MIN).unwrap();
        let input: Vec<_> = (0..17).collect();
        let result = executor.map_chunks(&input, 4, |_, outer| {
            executor.map_chunks(outer, 1, |_, inner| inner[0])
        });
        assert_eq!(result.into_iter().flatten().collect::<Vec<_>>(), input);
        assert!(executor.map_chunks::<u8, ()>(&[], 1, |_, _| ()).is_empty());
    }

    #[test]
    fn explicit_budget_is_not_capped_at_eight() {
        let executor = Executor::new(NonZeroUsize::new(9).unwrap()).unwrap();
        assert_eq!(executor.jobs(), 9);
        assert_eq!(executor.pool.current_num_threads(), 9);
    }
}
