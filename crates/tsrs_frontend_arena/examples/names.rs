//! Isolated-name comparison: one owner retains every result. No source-file parser or binder runs.
use std::hint::black_box;
use std::time::Instant;
use tsrs_ast::Kind;
use tsrs_frontend_arena::{FileArena, parse_isolated_entity_name};

#[global_allocator]
static ALLOCATOR: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

#[cfg(target_os = "macos")]
fn instructions() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::uninit();
    // SAFETY: proc_pid_rusage initializes this correctly sized buffer on success.
    unsafe {
        assert_eq!(
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V4,
                usage.as_mut_ptr().cast()
            ),
            0
        );
        usage.assume_init().ri_instructions
    }
}
#[cfg(not(target_os = "macos"))]
fn instructions() -> u64 {
    0
}

#[cfg(unix)]
fn peak_rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the buffer on success.
    let rss = unsafe {
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()), 0);
        usage.assume_init().ru_maxrss
    };
    u64::try_from(rss).unwrap() * if cfg!(target_os = "macos") { 1 } else { 1024 }
}
#[cfg(not(unix))]
fn peak_rss() -> u64 {
    0
}

const INPUTS: [&str; 16] = [
    "value",
    "Namespace.Type",
    "A.B.C.D",
    "await.yield",
    "default.class",
    "π.名称",
    r"\u0061.\u{62}",
    " a /* comment */ . b ",
    "a.\n b",
    "a.\n b c",
    "a.",
    "#private",
    "a.#private",
    "a.<B>",
    "a b",
    "a/*",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode: legacy or native");
    assert!(mode == "legacy" || mode == "native");
    let rounds: usize = args.next().expect("rounds").parse().unwrap();
    let arena = FileArena::new();
    let mut legacy = Vec::with_capacity(rounds * INPUTS.len());
    let mut native = Vec::with_capacity(rounds * INPUTS.len());
    let mut checksum = 0u64;
    let begin = instructions();
    let start = Instant::now();
    for _ in 0..rounds {
        for text in INPUTS {
            if mode == "legacy" {
                let root = tsrs_parser::parse_isolated_entity_name(text);
                let mut stack = Vec::from_iter(root);
                while let Some(node) = stack.pop() {
                    checksum +=
                        u64::from(node.kind() as u16) + u64::from(node.loc().len().unwrap());
                    if node.kind() == Kind::Identifier {
                        checksum += node
                            .text()
                            .as_bytes()
                            .iter()
                            .map(|&b| u64::from(b))
                            .sum::<u64>();
                    }
                    node.for_each_child(&mut |child| {
                        stack.push(child);
                        false
                    });
                }
                legacy.push(root);
            } else {
                let root = parse_isolated_entity_name(&arena, text);
                let mut stack = Vec::from_iter(root);
                while let Some(node) = stack.pop() {
                    checksum += u64::from(node.kind as u16) + u64::from(node.loc.len().unwrap());
                    if node.kind == Kind::Identifier {
                        checksum += node
                            .text()
                            .as_bytes()
                            .iter()
                            .map(|&b| u64::from(b))
                            .sum::<u64>();
                    }
                    node.for_each_child(&mut |child| {
                        stack.push(child);
                        false
                    });
                }
                native.push(root);
            }
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    let retired = instructions() - begin;
    println!(
        "{{\"mode\":\"{mode}\",\"names\":{},\"seconds\":{seconds},\"instructions\":{retired},\"peak_rss_bytes\":{},\"checksum\":{checksum}}}",
        rounds * INPUTS.len(),
        peak_rss()
    );
    black_box((&arena, &legacy, &native));
}
