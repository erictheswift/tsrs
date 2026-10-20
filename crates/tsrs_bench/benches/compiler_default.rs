use codspeed::codspeed::CodSpeed;
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
    // Match the CLI's stack. The CodSpeed hooks measure exactly one compilation on this thread.
    std::thread::Builder::new()
        .name("tsrs-bench".to_string())
        .stack_size(512 << 20)
        .spawn(move || {
            let (sys, args) = default_invocation(&name);
            let mut codspeed = CodSpeed::new();
            codspeed.start_benchmark(&format!("{}::compiler_default::{name}", file!()));
            let result = __codspeed_root_frame__compile(sys, args);
            codspeed.end_benchmark();
            result.assert_completed();
        })
        .unwrap()
        .join()
        .unwrap();
}
