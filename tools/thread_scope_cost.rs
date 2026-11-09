//! Standalone contextual measurement, not a predictor of a busy evaluator's cost.
//! rustc -O tools/thread_scope_cost.rs -o <external-path>
fn main() {
    for workers in [2, 4, 8] {
        let mut samples = Vec::new();
        for iteration in 0..520 {
            let start = std::time::Instant::now();
            std::thread::scope(|scope| {
                let tasks: Vec<_> = (0..workers)
                    .map(|i| scope.spawn(move || std::hint::black_box(i)))
                    .collect();
                for task in tasks {
                    std::hint::black_box(task.join().unwrap());
                }
            });
            if iteration >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1e6);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{{\"workers\":{workers},\"samples\":{},\"median_us\":{:.3},\"p95_us\":{:.3}}}",
            samples.len(), samples[samples.len() / 2], samples[samples.len() * 95 / 100],
        );
    }
}
