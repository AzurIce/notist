use crate::fixtures::{self, Fixture};
use serde_json::{Value, json};
use std::{
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Check,
    Time,
    Alloc,
}

struct Options {
    mode: Mode,
    sizes: Vec<usize>,
    samples: usize,
    sample_ms: u64,
    filter: String,
    seed: u64,
    json: Option<PathBuf>,
}

fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        mode: Mode::Time,
        sizes: vec![1, 16, 128],
        samples: 7,
        sample_ms: 40,
        filter: String::new(),
        seed: 20261107,
        json: None,
    };
    let mut args = arguments.into_iter();
    while let Some(arg) = args.next() {
        let mut next = || {
            args.next()
                .ok_or_else(|| format!("missing value for {arg}"))
        };
        match arg.as_str() {
            "--bench" => {}
            "--test" => options.mode = Mode::Check,
            "--help" | "-h" => {
                println!(
                    "eval scenarios: --mode check|time|alloc --sizes 1,16,128 --samples 7 --sample-ms 40 --filter substring --seed 20261107 --json FILE\nTime includes evaluation and destruction, excludes fixture compilation and validation. Alloc requires --features alloc-metrics and is not a timing measurement."
                );
                return Ok(None);
            }
            "--mode" => {
                options.mode = match next()?.as_str() {
                    "check" => Mode::Check,
                    "time" => Mode::Time,
                    "alloc" => Mode::Alloc,
                    other => return Err(format!("unknown mode {other}")),
                }
            }
            "--sizes" => {
                options.sizes = next()?
                    .split(',')
                    .map(|s| s.parse().map_err(|_| format!("invalid size {s}")))
                    .collect::<Result<_, _>>()?
            }
            "--samples" => options.samples = next()?.parse().map_err(|_| "invalid sample count")?,
            "--sample-ms" => {
                options.sample_ms = next()?.parse().map_err(|_| "invalid sample duration")?
            }
            "--seed" => options.seed = next()?.parse().map_err(|_| "invalid seed")?,
            "--filter" => options.filter = next()?,
            "--json" => options.json = Some(next()?.into()),
            _ => return Err(format!("unknown argument {arg}")),
        }
    }
    if options.samples == 0
        || options.samples > 100
        || options.sample_ms == 0
        || options.sample_ms > 2000
        || options.seed == 0
        || options.sizes.is_empty()
        || options.sizes.iter().any(|n| *n == 0 || *n > 512)
    {
        return Err(
            "samples must be 1..100, sample-ms 1..2000, sizes 1..512, and seed nonzero".into(),
        );
    }
    options.sizes.sort_unstable();
    options.sizes.dedup();
    if options.mode == Mode::Time && cfg!(feature = "alloc-metrics") {
        return Err("timing refuses the allocation-instrumented binary".into());
    }
    if options.mode == Mode::Time && cfg!(debug_assertions) {
        return Err("timing requires a release build".into());
    }
    if options.mode == Mode::Alloc && !cfg!(feature = "alloc-metrics") {
        return Err("allocation mode requires the alloc-metrics feature".into());
    }
    Ok(Some(options))
}

fn elapsed(fixture: &Fixture, iterations: u64) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        let result = black_box(fixture.evaluate());
        drop(result);
    }
    start.elapsed()
}

fn calibrate(fixture: &Fixture, target: Duration) -> u64 {
    let mut iterations = 1;
    loop {
        let duration = elapsed(fixture, iterations);
        if duration >= Duration::from_millis(5) || iterations >= 1_000_000 {
            return ((target.as_secs_f64() / duration.as_secs_f64() * iterations as f64).ceil()
                as u64)
                .clamp(1, 2_000_000);
        }
        iterations *= 2;
    }
}

fn shuffle(order: &mut [usize], state: &mut u64) {
    for i in (1..order.len()).rev() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        order.swap(i, (*state as usize) % (i + 1));
    }
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

pub fn run(arguments: impl IntoIterator<Item = String>) -> Result<(), String> {
    let Some(options) = parse(arguments)? else {
        return Ok(());
    };
    let mut fixtures = Vec::new();
    let started = Instant::now();
    for kind in fixtures::KINDS {
        let sizes = if *kind == "nested_expansion" {
            vec![1, 4, 8, 16]
        } else {
            options.sizes.clone()
        };
        for size in sizes {
            let id = format!("{kind}/{size}");
            if !id.contains(&options.filter) {
                continue;
            }
            fixtures.push(fixtures::prepare(kind, size)?);
        }
    }
    if fixtures.is_empty() {
        return Err("filter selected no scenarios".into());
    }
    let preparation_seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "validated {} scenarios; preparation {:.3}s (not timed)",
        fixtures.len(),
        preparation_seconds
    );
    let mut records = vec![Vec::<Value>::new(); fixtures.len()];
    let mut timings = vec![Vec::<f64>::new(); fixtures.len()];
    let mut iterations = vec![0; fixtures.len()];
    if options.mode == Mode::Time {
        for (index, fixture) in fixtures.iter().enumerate() {
            iterations[index] = calibrate(fixture, Duration::from_millis(options.sample_ms));
            fixture.validate()?;
        }
    }
    let mut order: Vec<_> = (0..fixtures.len()).collect();
    let mut state = options.seed;
    let rounds = if options.mode == Mode::Check {
        1
    } else {
        options.samples
    };
    let mut orders = Vec::new();
    for round in 0..rounds {
        shuffle(&mut order, &mut state);
        orders.push(order.clone());
        for &index in &order {
            let fixture = &fixtures[index];
            match options.mode {
                Mode::Time => {
                    let duration = elapsed(fixture, iterations[index]);
                    let per_eval = duration.as_secs_f64() * 1e9 / iterations[index] as f64;
                    timings[index].push(per_eval);
                    records[index].push(json!({"round": round, "iterations": iterations[index], "elapsed_ns": duration.as_nanos(), "ns_per_eval": per_eval}));
                }
                Mode::Alloc => {
                    #[cfg(feature = "alloc-metrics")]
                    records[index].push(crate::allocation::measure(|| fixture.evaluate()).json());
                    #[cfg(not(feature = "alloc-metrics"))]
                    unreachable!();
                }
                Mode::Check => {}
            }
            // Compare the complete tree, diagnostics, dependencies and sources
            // outside measurement; never normalize away semantic identities.
            fixture.validate()?;
        }
    }
    let rows: Vec<_> = fixtures.iter().enumerate().map(|(index, fixture)| {
        let summary = if timings[index].is_empty() { Value::Null } else {
            let values = &timings[index];
            let ns = median(values);
            let nodes = fixture.facts["output_nodes"].as_u64().unwrap();
            eprintln!("{:<30} {:>10.3} us/eval  nodes={nodes}", fixture.id, ns / 1000.0);
            json!({"median_ns_per_eval": ns, "min_ns_per_eval": values.iter().copied().reduce(f64::min), "max_ns_per_eval": values.iter().copied().reduce(f64::max), "median_ns_per_output_node": if nodes == 0 { None } else { Some(ns / nodes as f64) }})
        };
        json!({"id": fixture.id, "kind": fixture.kind, "size": fixture.size, "facts": fixture.facts, "module_source": fixture.module, "document_source": fixture.document, "timing": summary, "samples": records[index]})
    }).collect();
    let output = json!({
        "schema": "notist-eval-scenarios-v1", "mode": match options.mode { Mode::Check => "check", Mode::Time => "time", Mode::Alloc => "alloc" },
        "allocation_instrumented": cfg!(feature = "alloc-metrics"), "debug_assertions": cfg!(debug_assertions),
        "seed": options.seed, "sample_ms": options.sample_ms, "preparation_seconds": preparation_seconds,
        "scope": "production notist_eval::run plus output destruction; compiler/validation/HTML/IO excluded",
        "cache_policy": "json_cached is warmed in preparation and retained across evaluations; json_uncached supplies no parsed-data cache; on_demand_instances starts with an empty per-run program but a warm Context checked-plan cache",
        "orders": orders, "rows": rows,
    });
    let bytes = serde_json::to_vec_pretty(&output).map_err(|e| e.to_string())?;
    if let Some(path) = options.json {
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    } else {
        use std::io::Write;
        std::io::stdout()
            .lock()
            .write_all(&bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orders_are_repeatable_and_preserve_every_case() {
        let mut a: Vec<_> = (0..52).collect();
        let mut b = a.clone();
        shuffle(&mut a, &mut 42);
        shuffle(&mut b, &mut 42);
        assert_eq!(a, b);
        assert_ne!(a, (0..52).collect::<Vec<_>>());
        a.sort_unstable();
        assert_eq!(a, (0..52).collect::<Vec<_>>());
    }
    #[test]
    fn invalid_parameters_fail_before_measurement() {
        for arguments in [
            vec!["--sizes", "0"],
            vec!["--samples", "0"],
            vec!["--seed", "0"],
            vec!["--sample-ms", "0"],
            vec!["--mode", "unknown"],
        ] {
            assert!(parse(arguments.into_iter().map(String::from)).is_err());
        }
    }
    #[test]
    fn median_includes_all_samples() {
        assert_eq!(median(&[9., 1., 3.]), 3.);
        assert_eq!(median(&[9., 1., 3., 5.]), 4.);
    }
}
