//! Factory-only comparison: retain identical synthetic `value + 1;` trees. No parser or binder runs.
use std::hint::black_box;
use std::time::Instant;
use tsrs_ast::{Kind, TokenFlags};
use tsrs_core::{P, TextRange};
use tsrs_frontend_arena::{FileArena, Node, Text, construction};

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

fn finish_legacy(node: P<tsrs_ast::Node>, loc: TextRange) -> P<tsrs_ast::Node> {
    node.set_loc(loc);
    node.for_each_child(&mut |child| {
        child.set_parent(Some(node));
        false
    });
    node
}
fn finish_native<'build, 'ast>(
    node: &'build construction::Node<'build, 'ast>,
    loc: TextRange,
) -> &'build construction::Node<'build, 'ast> {
    node.set_loc(loc);
    node.for_each_child(&mut |child| {
        child.set_parent(Some(node));
        false
    });
    node
}
fn legacy_tree(statements: usize) -> P<tsrs_ast::Node> {
    let factory = tsrs_ast::NodeFactory::default();
    let name = tsrs_core::alloc_str("value");
    let mut nodes = Vec::with_capacity(statements);
    for i in 0..statements {
        let start = u32::try_from(i * 11).unwrap();
        let left = finish_legacy(
            factory.new_identifier(name),
            TextRange::new(start, start + 5),
        );
        let operator = finish_legacy(
            factory.new_token(Kind::PlusToken),
            TextRange::new(start + 6, start + 7),
        );
        let right = finish_legacy(
            factory.new_numeric_literal("1", TokenFlags::None),
            TextRange::new(start + 8, start + 9),
        );
        let binary = finish_legacy(
            factory.new_binary_expression(None, left, None, operator, right),
            TextRange::new(start, start + 9),
        );
        nodes.push(finish_legacy(
            factory.new_expression_statement(binary),
            TextRange::new(start, start + 10),
        ));
    }
    finish_legacy(
        factory.new_block(factory.new_node_list(nodes), true),
        TextRange::new(0, u32::try_from(statements * 11).unwrap()),
    )
}
fn native_tree(arena: &FileArena, statements: usize) -> &Node<'_> {
    arena.build(|factory| {
        let name = factory.text(b"value");
        let mut nodes = Vec::with_capacity(statements);
        for i in 0..statements {
            let start = u32::try_from(i * 11).unwrap();
            let left = finish_native(
                factory.new_identifier(name),
                TextRange::new(start, start + 5),
            );
            let operator = finish_native(
                factory.new_token(Kind::PlusToken),
                TextRange::new(start + 6, start + 7),
            );
            let right = finish_native(
                factory.new_numeric_literal(Text::from_bytes(b"1"), TokenFlags::None),
                TextRange::new(start + 8, start + 9),
            );
            let binary = finish_native(
                factory.new_binary_expression(None, left, None, operator, right),
                TextRange::new(start, start + 9),
            );
            nodes.push(finish_native(
                factory.new_expression_statement(binary),
                TextRange::new(start, start + 10),
            ));
        }
        finish_native(
            factory.new_block(factory.new_node_list(&nodes), true),
            TextRange::new(0, u32::try_from(statements * 11).unwrap()),
        )
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode: legacy or native");
    assert!(mode == "legacy" || mode == "native");
    let files: usize = args.next().expect("file count").parse().unwrap();
    let statements: usize = args.next().expect("statements per file").parse().unwrap();
    let arenas: Vec<_> = std::iter::repeat_with(FileArena::new).take(files).collect();
    let mut legacy = Vec::with_capacity(files);
    let mut native = Vec::with_capacity(files);
    let mut checksum = 0u64;
    let begin = instructions();
    let start = Instant::now();
    for arena in &arenas {
        if mode == "legacy" {
            let root = legacy_tree(statements);
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                checksum +=
                    u64::from(node.kind() as u16) + u64::from(node.loc().len().unwrap_or(0));
                node.for_each_child(&mut |child| {
                    assert_eq!(child.parent(), Some(node));
                    stack.push(child);
                    false
                });
            }
            legacy.push(root);
        } else {
            let root = native_tree(arena, statements);
            let mut stack = vec![root];
            while let Some(node) = stack.pop() {
                checksum += u64::from(node.kind as u16) + u64::from(node.loc.len().unwrap_or(0));
                node.for_each_child(&mut |child| {
                    assert!(std::ptr::eq(child.parent.unwrap(), node));
                    stack.push(child);
                    false
                });
            }
            native.push(root);
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    let retired = instructions() - begin;
    println!(
        "{{\"mode\":\"{mode}\",\"files\":{files},\"statements\":{statements},\"seconds\":{seconds},\"instructions\":{retired},\"peak_rss_bytes\":{},\"checksum\":{checksum}}}",
        peak_rss()
    );
    black_box((&arenas, &legacy, &native));
}
