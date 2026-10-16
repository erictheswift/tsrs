# Native name parsing

Continuation of [native node construction](native-node-factory.md), starting at `a6c270b8` on
`codex/file-arena-lifetimes`. The existing isolated entity-name entry point now has a native implementation:

```rust
use tsrs_frontend_arena::{FileArena, parse_isolated_entity_name};
let arena = FileArena::new();
let name = parse_isolated_entity_name(&arena, "Namespace.Type").unwrap();
assert!(name.parent.is_none());
assert!(parse_isolated_entity_name(&arena, "Namespace.").is_none());
```

The scanner, parser state, factory and every graph edge carry real lifetimes. The arena copies the input
once, and nodes borrow scanner values directly, including cooked escaped identifiers. The caller may drop
its input string immediately. No legacy node, source-text registry slot, thread-local allocation, publication
walk or graph copy is involved in this entry point. Immutable results can be read on scoped worker threads.

## Shared grammar

`tsrs_parser::name_parser::NameParser` contains the existing identifier, binding-identifier, private-name,
qualified-name and right-side-of-dot routines. Their names, branch ordering, keyword/context rules and error
recovery are preserved. The production `Parser` forwards to these routines, retaining its current factory,
compressed source identifiers, diagnostic construction and speculative rewind behavior. The native adapter
supplies borrowed nodes, arena scanner values and owned diagnostics. Neither adapter adds a second grammar.

This also exercises the shared routines during ordinary source-file parsing: type references, import/export
names, property accesses and JSDoc still follow the production parser's call sites. The native public entry
point has exactly the existing isolated-name contract: a complete input with no parse diagnostics returns
`Some`; malformed input or remaining tokens return `None`.

`FileArena::try_build` publishes only successful roots. An error may own diagnostics but cannot contain
construction references; a compile-fail example checks this restriction. `build` uses the same publication
boundary through an infallible result. Failed parsing keeps unreachable allocations until the arena drops,
as canceled construction already did. It does not rewind while references may be live.

The native adapter restores scanner snapshots, diagnostic length and pending error flags after dot recovery
lookahead. It reports scanner errors in parser order, suppresses consecutive duplicate positions, applies
context/error flags at node completion, and sets immediate children's parents. Diagnostics own their text and
survive arena release. Token-byte address assertions confirm that making an identifier does not copy its text.

## Scope and prior evidence

This is the first native parser entry point, not a native source-file parser. Statements, types, expressions,
JSDoc attachment/reparsing, source-file metadata and the binder have not moved yet. `parse_and_bind` still
constructs the legacy graph and copies it. Production callers still receive `P<Node>`; no CLI/API behavior or
default allocator changes in this stage. The next migration step is the remaining parser routines and state,
followed by binding within the same construction scope.

The prior lifetime/scanner/factory reports, the rejected list in `perf-round2-followups.md`, the techniques in
`docs/RUST.md`, and `mem-round4.md` remain the baseline. The changed constraint is the owner's explicit
lifetime migration, not a performance claim. In particular, this does not reverse the measured costs of
native references or 64 KiB minimum file chunks, or qualify a production frontend switch.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --lib parser::tests
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test parser \
  parsed_names_own_source_and_survive_region_release
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test factory
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Nine new runtime cases cover native and legacy agreement, 3,042 keyword/name/trivia/recovery combinations,
a 4,096-part qualified name with every parent checked, native token-byte identity, context flags, failed
construction with owned diagnostics, scanner error replay after lookahead, and release followed by concurrent
readers. All runtime cases and 22 compile-fail/2 positive doctests pass. Strict-provenance Miri passes the five
native parser unit cases, the owner/concurrent-reader case and all five factory cases.

The native-versus-legacy comparison cannot run with strict-provenance Miri: the legacy `Node::payload` calls
`arena_neighbor`, which reconstructs pointers from integers. The attempted run stopped there, before any
native failure. The strict runs above keep that legacy helper out of the exercised path; the differential
fixtures run natively. No change to legacy pointer provenance is hidden in this stage.

A saved AST oracle binary from before the refactor and the rebuilt oracle produce identical hashes on all
15,537 public source files (VS Code, XState, Drizzle; revisions/manifests in `file-arena-lifetimes.md`). This is
independent of the new native/legacy comparison sharing grammar. The selected conformance pass list is also
identical: 1,497 passes, 203 skips, zero failures/crashes. No generated schema/output changes are needed.

## Measurements

```sh
# Keep the measure executable built at a6c270b8 before building this revision.
cargo build --release -p tsrs_frontend_arena --example measure --example names
/tmp/measure-before-names tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Also compare published, and repeat on the pinned XState/Drizzle manifests.

target/release/examples/names legacy 16384
target/release/examples/names native 16384
```

The name microbenchmark parses 262,144 inputs (16 fixtures × 16,384 repetitions), retaining every successful
root in one owner. It mixes plain, qualified, escaped, Unicode and malformed names, then walks successful
nodes and checksums their kinds, ranges and identifier bytes. Both modes must produce the same checksum. Failed
native allocations stay in that owner until the process finishes; the legacy mode likewise keeps its arena.
This is not a source-file/checker workload or a per-file memory benchmark.

The before/after public-source harness still runs the legacy source-file parser and binder, with and without
publication. It checks checksums and diagnostic counts and measures the cost of sharing the name
grammar on the existing path. Each comparison has one warmup and five measured samples in alternating order, using
release builds, mimalloc, macOS `proc_pid_rusage` instruction deltas and process peak RSS.

Raw samples and medians are in [native-name-parser-results.json](native-name-parser-results.json).

Median isolated-name results:

| Mode | Instructions | Peak RSS |
| --- | ---: | ---: |
| legacy | 0.504754B | 43.30 MiB |
| native | 0.498757B | 61.83 MiB |

The native entry point uses 1.19% fewer instructions and
42.8% more peak RSS in this microbenchmark. It avoids the legacy parser's larger
state and retained arena diagnostics, but the wider native graph remains a memory cost. This does not
predict source-file parsing or checker performance, and it does not meet the project benchmark gate.

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.843912B → 19.887813B | +0.221% | 1515.92 → 1515.95 MiB |
| VS Code | published | 37.160870B → 37.234757B | +0.199% | 2990.23 → 2990.27 MiB |
| XState | tls | 0.320355B → 0.321009B | +0.204% | 28.61 → 28.62 MiB |
| XState | published | 0.624253B → 0.625172B | +0.147% | 57.06 → 57.08 MiB |
| Drizzle | tls | 1.396832B → 1.399848B | +0.216% | 107.33 → 107.33 MiB |
| Drizzle | published | 2.628782B → 2.632806B | +0.153% | 219.98 → 219.98 MiB |

Sharing the grammar adds about 0.15–0.22% instructions in these comparisons; peak RSS changes by at most
32 KiB. Syntax checksums and diagnostic counts match. Ordinary/forced inlining of the forwarding adapter
and keeping identifier creation out of line were screened separately; none removed the small instruction
increase, so no new inlining directives are retained. This is recorded as a cost of this branch’s requested
lifetime migration, not a speedup or a reason to change production defaults. The migration remains on its
isolated branch; the frontend performance gate has not been cleared.

The next stage, [native token and literal parsing](native-token-parser.md), shares token recovery, literal
expressions/types and template-part parsing through the same scoped factory.
