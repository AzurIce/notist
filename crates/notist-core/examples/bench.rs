//! Stage 1 standalone benchmark: measures parse, check, and eval separately.
//!
//! Run with:
//!   cargo run --release -p notc --example bench

use notist_core::{Value, check, parse};
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::{Duration, Instant};

fn generate_module(function_count: usize) -> String {
    let mut out = String::with_capacity(function_count * 120);
    out.push_str(
        "fn inc(x: Int) -> Int { x + 1 }\n\
         fn identity(f: fn(Int) -> Int) -> fn(Int) -> Int { f }\n\
         fn Int.add_to(self, delta: Int) -> Int { self + delta }\n\
         fn apply2(f: fn(Int) -> Int, x: Int) -> Int { f(f(x)) }\n\n",
    );
    for i in 0..function_count {
        let _ = writeln!(
            out,
            "fn f_{i}(x: Int, flag: Bool) -> Int {{\n\
                 let step = identity(inc);\n\
                 let a = step(x).add_to({i});\n\
                 if flag {{ apply2(step, a) }} else {{ a + 1 }}\n\
             }}"
        );
    }
    let _ = writeln!(
        out,
        "\nfn bench_chain(x: Int, depth: Int) -> Int {{\n\
             if false {{ x }} else {{ apply2(inc, x).add_to(depth) }}\n\
         }}"
    );
    // Build a deep straight-line call chain to measure steady-state evaluation without
    // hitting MAX_CALL_DEPTH (64): 40 functions each calling the previous twice via
    // local bindings / higher-order calls.
    out.push_str("fn eval_leaf(x: Int) -> Int { identity(inc)(x).add_to(1) }\n");
    for level in 0..14 {
        let prev = if level == 0 {
            "eval_leaf".to_owned()
        } else {
            format!("eval_l{}", level - 1)
        };
        let _ = writeln!(
            out,
            "fn eval_l{level}(x: Int) -> Int {{\n\
                 let f = identity({prev});\n\
                 let a = f(x);\n\
                 f(a)\n\
             }}"
        );
    }
    out.push_str("fn eval_entry() -> Int { eval_l13(0) }\n");
    out
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn main() {
    let decl_count = 2_000usize;
    let source = generate_module(decl_count);
    let source_bytes = source.len();

    // Verify correctness outside measured loops.
    let parsed_once = parse(&source);
    assert!(
        parsed_once.diagnostics.is_empty(),
        "unexpected parse diagnostics: {:?}",
        parsed_once.diagnostics
    );
    let total_decls = parsed_once.program.root.declarations.len();
    let checked_once = check(&parsed_once.program).expect("benchmark module must check");
    let expected = 1i64 << 15; // 2^14 leaf calls * (+2 per leaf) = 32768
    let actual = checked_once
        .run("eval_entry")
        .expect("benchmark entry must execute");
    assert!(
        matches!(actual, Value::Int(v) if v == expected),
        "expected Int({expected}), got {actual:?}"
    );

    // 1. Cold parse benchmark
    let parse_iters = 25;
    let mut parse_samples = Vec::with_capacity(parse_iters);
    for _ in 0..parse_iters {
        let t0 = Instant::now();
        let res = parse(black_box(&source));
        let dt = t0.elapsed();
        black_box(res);
        parse_samples.push(dt);
    }
    let parse_med = median(parse_samples);

    // 2. Cold check benchmark (reuses already-parsed AST)
    let check_iters = 25;
    let mut check_samples = Vec::with_capacity(check_iters);
    for _ in 0..check_iters {
        let t0 = Instant::now();
        let res = check(black_box(&parsed_once.program));
        let dt = t0.elapsed();
        let _ = black_box(res);
        check_samples.push(dt);
    }
    let check_med = median(check_samples);

    // 3. Eval benchmark (32,767+ calls per run across 14 levels + HOF + methods)
    let eval_iters = 25;
    let mut eval_samples = Vec::with_capacity(eval_iters);
    for _ in 0..eval_iters {
        let t0 = Instant::now();
        let res = checked_once.run(black_box("eval_entry"));
        let dt = t0.elapsed();
        let _ = black_box(res);
        eval_samples.push(dt);
    }
    let eval_med = median(eval_samples);

    let mb = source_bytes as f64 / (1024.0 * 1024.0);
    let parse_mb_s = mb / parse_med.as_secs_f64();
    let check_decls_s = total_decls as f64 / check_med.as_secs_f64();
    // Leaf calls: 2^14 = 16,384; each leaf calls identity + inc + add_to (4 calls);
    // internal nodes: 2^14 - 1 = 16,383 (each calls identity + 2 recursive calls).
    let approx_calls = (16_384 * 4 + 16_383 * 3) as f64;
    let ns_per_call = eval_med.as_secs_f64() * 1e9 / approx_calls;

    println!("notc Stage 1 benchmark ({total_decls} decls, {source_bytes} bytes):");
    println!(
        "  parse : {:8.3} ms median  ({:6.1} MiB/s)",
        parse_med.as_secs_f64() * 1e3,
        parse_mb_s
    );
    println!(
        "  check : {:8.3} ms median  ({:9.0} decls/s)",
        check_med.as_secs_f64() * 1e3,
        check_decls_s
    );
    println!(
        "  eval  : {:8.3} ms median  ({:6.1} ns/call across ~{:.0} calls, result={expected})",
        eval_med.as_secs_f64() * 1e3,
        ns_per_call,
        approx_calls
    );
    if let Some(kib) = peak_rss_kib() {
        println!("  VmHWM : {:.2} MiB", kib as f64 / 1024.0);
    }
}
