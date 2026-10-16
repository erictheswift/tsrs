//! Front-end comparison only; this does not run the checker or resolve project dependencies.
use std::hint::black_box;
use std::hash::Hasher;
use std::time::Instant;
use tsrs_core::{P, arena::Region};
use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};

#[global_allocator]
static ALLOCATOR: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

#[cfg(target_os = "macos")]
fn instructions() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::uninit();
    // SAFETY: the kernel initializes this correctly sized rusage_info_v4 buffer on success.
    unsafe {
        assert_eq!(libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, usage.as_mut_ptr().cast()), 0);
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
    #[cfg(target_os = "macos")]
    let factor = 1;
    #[cfg(not(target_os = "macos"))]
    let factor = 1024;
    u64::try_from(rss).unwrap() * factor
}
#[cfg(not(unix))]
fn peak_rss() -> u64 {
    0
}

fn legacy_checksum(file: P<tsrs_ast::SourceFile>) -> u64 {
    let mut stack = vec![file.as_node()];
    let mut checksum = 0u64;
    while let Some(node) = stack.pop() {
        black_box(node.jsdoc(Some(file.get())));
        checksum += u64::from(node.kind() as u16) + u64::from(node.loc().len().unwrap_or(0));
        node.for_each_child(&mut |n| {
            stack.push(n);
            false
        });
    }
    checksum
}

fn scanner_checksum<'text, S: tsrs_scanner::storage::ScannerStorage<'text>>(
    mut scanner: tsrs_scanner::Scanner<'text, S>,
) -> (u64, usize) {
    use tsrs_scanner::storage::TokenValue as _;
    let mut hash = rustc_hash::FxHasher::default();
    let mut diagnostics = 0;
    scanner.set_on_error(true);
    loop {
        let kind = scanner.scan();
        hash.write_u16(kind as u16);
        hash.write_u32(scanner.token_start());
        hash.write_u32(scanner.token_end());
        hash.write_u32(scanner.token_flags().bits());
        hash.write(scanner.token_value().as_bytes());
        diagnostics += scanner.take_errors().len();
        if kind == tsrs_ast::Kind::EndOfFile { break; }
    }
    for directive in scanner.comment_directives() {
        hash.write_u32(directive.loc.pos().expect_source("directive position"));
        hash.write_u32(directive.loc.end().expect_source("directive end"));
    }
    (hash.finish(), diagnostics)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode: tls, region, retained, direct, published-tls, published, region-drop, published-drop, scan-tls, scan-native");
    if mode == "layout" {
        println!(
            "legacy_node={} native_node={} legacy_link={} native_link={} legacy_list={} native_list={}",
            std::mem::size_of::<tsrs_ast::Node>(),
            std::mem::size_of::<tsrs_frontend_arena::Node<'_>>(),
            std::mem::size_of::<P<tsrs_ast::Node>>(),
            std::mem::size_of::<&tsrs_frontend_arena::Node<'_>>(),
            std::mem::size_of::<tsrs_ast::NodeList>(),
            std::mem::size_of::<tsrs_frontend_arena::NodeList<'_>>()
        );
        return;
    }
    assert!(
        ["tls", "region", "retained", "direct", "published-tls", "published", "region-drop", "published-drop", "scan-tls", "scan-native"].contains(&mode.as_str())
    );
    let manifest = std::fs::read_to_string(args.next().expect("newline-separated absolute file paths")).unwrap();
    let sources: Vec<_> = manifest
        .lines()
        .map(|name| {
            let kind = tsrs_core::get_script_kind_from_file_name(name);
            (name.to_owned(), std::fs::read_to_string(name).unwrap(), kind)
        })
        .collect();
    let arenas: Vec<_> = sources.iter().map(|_| FileArena::new()).collect();
    let mut regions = Vec::new();
    let mut checksum = 0u64;
    let mut used = 0;
    let mut reserved = 0;
    let mut construction = 0;
    let mut diagnostics = 0;
    let begin = instructions();
    let start = Instant::now();
    for ((name, source, kind), arena) in sources.iter().zip(&arenas) {
        if mode.starts_with("scan-") {
            let (hash, errors) = if mode == "scan-native" {
                let result = scanner_checksum(arena.scanner(source));
                used += arena.used_bytes();
                reserved += arena.allocated_bytes();
                result
            } else {
                let mut scanner = tsrs_scanner::Scanner::new();
                scanner.set_text(tsrs_core::alloc_str(source));
                scanner_checksum(scanner)
            };
            checksum = checksum.wrapping_add(hash);
            diagnostics += errors;
        } else if mode.starts_with("published") {
            let temporary = FileArena::new();
            let arena = if mode.ends_with("-drop") { &temporary } else { arena };
            let file = parse_and_bind(
                arena,
                source,
                ParseOptions { file_name: name.clone(), script_kind: *kind, direct_allocation: mode != "published-tls" },
            );
            let mut stack = vec![file.root];
            while let Some(node) = stack.pop() {
                checksum += u64::from(node.kind as u16) + u64::from(node.loc.len().unwrap_or(0));
                node.for_each_child(&mut |n| {
                    stack.push(n);
                    false
                });
            }
            used += arena.used_bytes();
            reserved += arena.allocated_bytes();
            construction += file.construction_bytes;
            diagnostics += file.diagnostics.len();
            black_box(file);
        } else {
            let region = (mode != "tls").then(|| Region::new(source.len().saturating_mul(10).saturating_add(4096).min(64 << 20)));
            let scope = region.as_ref().map(Region::enter);
            let options = tsrs_ast::SourceFileParseOptions { file_name: name.clone(), ..Default::default() };
            let file = if mode == "direct" || mode == "retained" {
                let file =
                    tsrs_parser::parse_source_file_for_publication(region.as_ref().unwrap(), options, source, *kind, mode == "direct");
                tsrs_binder::bind_source_file_for_publication(file);
                file
            } else {
                let file = tsrs_parser::parse_source_file_owned(options, source.clone(), *kind);
                tsrs_binder::bind_source_file(file);
                file
            };
            checksum += legacy_checksum(file);
            diagnostics +=
                file.diagnostics().len() + file.js_diagnostics().len() + file.jsdoc_diagnostics().len() + file.bind_diagnostics().len();
            if let Some(region) = &region {
                used += region.used_bytes();
                reserved += region.allocated_bytes();
            }
            drop(scope);
            if !mode.ends_with("-drop") {
                regions.extend(region);
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let retired = instructions() - begin;
    println!(
        "{{\"mode\":\"{mode}\",\"files\":{},\"seconds\":{elapsed},\"instructions\":{retired},\"peak_rss_bytes\":{},\"used_bytes\":{used},\"reserved_bytes\":{reserved},\"construction_bytes\":{construction},\"checksum\":{checksum},\"diagnostics\":{diagnostics}}}",
        sources.len(),
        peak_rss()
    );
    black_box((&arenas, &regions));
}
