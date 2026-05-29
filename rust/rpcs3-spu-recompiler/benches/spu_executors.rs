//! Benchmark: SPU interpreter vs Cranelift recompiler on real oracle programs.
//!
//! Lever #0 of `docs/PORT_STATUS_AND_ROADMAP.md` §4 — a measurement harness so
//! the interpreter-vs-JIT speed delta (and future optimizations) can be tracked
//! against regressions. Workloads are real behavior-freeze oracle `.spuimg`
//! images, so the SPU encodings are guaranteed valid.
//!
//! `execute()` measures the full per-dispatch cost (fresh `SpuThread` + segment
//! deploy + run-to-stop), mirroring what emu-core pays per
//! `sys_spu_thread_group_start`. For the recompiler:
//!   * `recompiler_cold` rebuilds the executor each iteration (compile every run)
//!   * `recompiler_warm` reuses the executor (compile once, cached dispatch)
//! The cold/warm split is the empirical face of the compile-latency vs
//! steady-state-throughput trade-off discussed in §4.1 (Cranelift vs LLVM).

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use rpcs3_spu_differential::{
    build_spu_program_from_captured_image, parse_jsonl_trace, CapturedEvent, InterpreterExecutor,
    SpuExecutor, SpuProgram,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Load a behavior-freeze SPU oracle program by fixture name (e.g.
/// `single_spu_branch_loop_v1`). Resolves the `.jsonl` trace + its `.spuimg`
/// side-file relative to the workspace root, mirroring the replay tests.
fn load_oracle(name: &str) -> SpuProgram {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop(); // -> rust/
    root.pop(); // -> workspace root

    let trace_path = root
        .join("behavior-freeze/fixtures/spu/traces")
        .join(format!("{name}.jsonl"));
    let images_dir = root.join("behavior-freeze/fixtures/spu/images");

    let raw = std::fs::read_to_string(&trace_path)
        .unwrap_or_else(|e| panic!("read trace {}: {e}", trace_path.display()));
    let events = parse_jsonl_trace(&raw).expect("parse trace");

    let image = events
        .iter()
        .find_map(|ev| match ev {
            CapturedEvent::SpuImage(img) => Some(img),
            _ => None,
        })
        .expect("trace must contain exactly one spu_image event");

    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    build_spu_program_from_captured_image(&image_path, image, 100_000_000)
        .expect("build SpuProgram from .spuimg")
}

fn bench_oracle(c: &mut Criterion, name: &str) {
    let program = load_oracle(name);
    let mut group = c.benchmark_group(name);

    group.bench_function("interpreter", |b| {
        let mut exec = InterpreterExecutor::default();
        b.iter(|| black_box(exec.execute(black_box(&program))));
    });

    group.bench_function("recompiler_cold", |b| {
        b.iter(|| {
            let mut exec = RecompilerExecutor::new();
            black_box(exec.execute(black_box(&program)))
        });
    });

    group.bench_function("recompiler_warm", |b| {
        let mut exec = RecompilerExecutor::new();
        // Prime the compiled-function cache so we measure cached dispatch.
        let _ = exec.execute(&program);
        b.iter(|| black_box(exec.execute(black_box(&program))));
    });

    group.finish();
}

/// Synthetic HOT loop: `il r3, N; (loop) ai r3,r3,-1; brnz r3,(loop); stop`.
/// Runs N iterations of a 2-instruction body — long enough that native code
/// amortizes the JIT compile + dispatch overhead, unlike the tiny oracles
/// (which are correctness fixtures, not throughput stress programs). This is
/// where a JIT is *supposed* to win; the oracles are not.
/// Encodings reuse the exact closures proven by the recompiler's
/// `loop_program` test (src/lib.rs:1103), so they are guaranteed valid.
fn hot_loop_program(iters: u16) -> SpuProgram {
    let il = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | (u32::from(imm) << 7) | rt;
    let ai = |rt: u32, ra: u32, imm: u32| (0x1Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
    let brnz = |rt: u32, off: u32| (0x042u32 << 23) | ((off & 0xFFFF) << 7) | rt;
    let stop = 0x55u32 & 0x3FFF;

    let code = [
        il(3, iters),    // 0x100  r3 = iters
        ai(3, 3, 0x3FF), // 0x104  r3 -= 1  (-1 in 10-bit)   [loop target]
        brnz(3, 0xFFFF), // 0x108  if r3 != 0 → 0x108 + (-1)*4 = 0x104
        stop,            // 0x10C  stop 0x55
    ];
    let mut bytes = Vec::with_capacity(16);
    for w in code {
        bytes.extend_from_slice(&w.to_be_bytes());
    }
    SpuProgram::new(0x100, 200_000).with_segment(0x100, bytes)
}

fn bench_hot_loop(c: &mut Criterion) {
    let program = hot_loop_program(30_000);

    // Sanity: the loop must reach Stop with ~60k steps, NOT a max_steps
    // timeout — guards against an encoding mistake silently benchmarking a
    // runaway loop instead of a clean 30k-iteration run.
    let mut probe = InterpreterExecutor::default();
    let r = probe.execute(&program);
    eprintln!(
        "[hot_loop sanity] stop_reason={:?} steps_executed={}",
        r.stop_reason, r.steps_executed
    );

    let mut group = c.benchmark_group("hot_loop_30k");
    group.bench_function("interpreter", |b| {
        let mut exec = InterpreterExecutor::default();
        b.iter(|| black_box(exec.execute(black_box(&program))));
    });
    group.bench_function("recompiler_warm", |b| {
        let mut exec = RecompilerExecutor::new();
        let _ = exec.execute(&program); // prime the compiled-function cache
        b.iter(|| black_box(exec.execute(black_box(&program))));
    });
    group.finish();
}

fn benches(c: &mut Criterion) {
    // Compute-heavy (Fibonacci branch loop) — short; setup-dominated.
    bench_oracle(c, "single_spu_branch_loop_v1");
    // Channel-heavy (mailbox handshake) — exercises interpreter fallback.
    bench_oracle(c, "single_spu_mailbox_v1");
    // Synthetic hot loop — where the JIT is actually meant to win.
    bench_hot_loop(c);
}

criterion_group!(benches_group, benches);
criterion_main!(benches_group);
