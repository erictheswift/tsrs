use codspeed::codspeed::CodSpeed;
use codspeed::walltime_results::WalltimeBenchmark;
use std::time::Instant;
use tsrs_bench::{BenchmarkSystem, Compilation, default_invocation};

#[global_allocator]
static ALLOCATOR: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

#[inline(never)]
#[expect(non_snake_case, reason = "CodSpeed recognizes this prefix as the profiling root")]
fn __codspeed_root_frame__compile(sys: &'static BenchmarkSystem, args: Vec<String>) -> Compilation {
    sys.compile(args)
}

fn main() {
    #[cfg(all(target_os = "linux", not(miri)))]
    tsrs_core::allocator::init_mimalloc();
    for (name, _) in std::env::vars_os() {
        assert!(
            !name.as_encoded_bytes().starts_with(b"TSRS_") || name == "TSRS_BENCH_PROJECT",
            "unset compiler environment override {} before benchmarking",
            name.display()
        );
    }
    assert!(
        std::env::var_os("RAYON_NUM_THREADS").is_none(),
        "unset RAYON_NUM_THREADS to use the CLI's default parallelism"
    );
    let name = std::env::var("TSRS_BENCH_PROJECT")
        .expect("set TSRS_BENCH_PROJECT to the project to benchmark");
    // cargo-codspeed passes --bench only in walltime mode. That mode needs a raw duration sample
    // in addition to the hooks' profile, or the action has no walltime result to upload.
    let walltime = std::env::args_os().any(|arg| arg == "--bench");
    // Match the CLI's stack. The CodSpeed hooks measure exactly one compilation on this thread.
    std::thread::Builder::new()
        .name("tsrs-bench".to_string())
        .stack_size(512 << 20)
        .spawn(move || {
            let (sys, args) = default_invocation(&name);
            let mut codspeed = CodSpeed::new();
            let uri = format!("{}::compiler_default::{name}", file!());
            let start = Instant::now();
            codspeed.start_benchmark(&uri);
            let result = __codspeed_root_frame__compile(sys, args);
            codspeed.end_benchmark();
            let elapsed = start.elapsed();
            result.assert_completed();
            if walltime {
                WalltimeBenchmark::collect_raw_walltime_results(
                    "tsrs",
                    format!("compiler_default::{name}"),
                    uri,
                    vec![1],
                    vec![elapsed.as_nanos()],
                    None,
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
