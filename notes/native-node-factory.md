# Native node construction and publication

Continuation of [per-file scanner storage](scanner-file-arena.md), starting at `cf268473` on
`codex/file-arena-lifetimes`. `FileArena::build` constructs native nodes, payloads, lists, symbols, flow graphs
and metadata directly in their final slots. Publishing a root does not copy or walk that graph.

```rust
use tsrs_frontend_arena::{FileArena, TextRange};

let arena = FileArena::new();
let root = arena.build(|factory| {
    let name = factory.new_identifier(factory.text(b"value"));
    name.set_loc(TextRange::new(0, 5));
    let expression = factory.new_parenthesized_expression(name);
    name.set_parent(Some(expression));
    expression
});
std::thread::scope(|scope| {
    scope.spawn(|| assert!(root.parent.is_none())).join().unwrap();
});
```

The closure receives a factory with an invariant construction lifetime. Graph links are actual
`&'build construction::Node<'build, 'ast>` references, including parent, symbol and flow cycles. Every record
is fully initialized when allocated. Its private `Cell` fields permit changes through generated getters and
setters during this scope. The records cannot be moved, cloned out of the arena, constructed directly, shared
across threads, or mixed with another construction scope. Copyable symbol/pattern entries retain the same
invariant lifetime and are copied into factory-owned slices.

After the callback returns, construction references and setters cannot escape. One documented unsafe cast
exposes the same initialized bytes as the existing immutable `&'ast Node<'ast>` graph. This extends internal
construction borrows to the arena's lifetime at a single publication boundary: branded records come from
that arena; slices allocated elsewhere require their captured owner to live for the published borrow through
the higher-ranked callback; and unbranded text/metadata inputs explicitly live for `'ast`. There is no `P<Node>`, raw
pointer handle or global registry in this API, and no new unsafe `Send`/`Sync` implementation. The final graph
is shareable because its types contain only immutable references and values.

The construction generator consumes the existing published record definitions and AST schema. It rejects
unrecognized field syntax rather than silently omitting a field. Size, alignment and every field offset are
asserted against the published counterpart. Construction and published enums have the same explicit C/u16
representation and variant order. The checker-only `Any` payload remains unconstructible; its discriminant
position is retained. There are 194 mutable record types, two immutable entry types, and 192 generated syntax
constructors, plus handwritten source-file/list helpers. The native header/list layout is unchanged; the
scope markers consume no bytes. Constructors and visitors retain the schema's argument order, flag masks,
kind aliases and JSDoc child ordering.

`Text::from_bytes(scanner.token_value())` lets a native node retain cooked bytes without another allocation.
Lone UTF-16 surrogates remain bytes. Text and plain metadata slices may be retained independently for their
arena lifetime; `values<T: Copy + 'ast>` cannot extend construction references beyond their scope.

Canceled construction leaves unreachable slots in its arena, which are reclaimed when the owner drops.
There is no per-node reclamation or arena rewind. Several completed scopes may share an owner, but mutable
edges cannot cross their boundaries. Cross-file checking, lazy mutation, checker-created graphs referencing
existing files, and incremental ownership remain later work.

## Scope and prior evidence

This implements the native factory foundation. The main `parse_and_bind` function still runs the legacy
parser/binder and copies their graph. It has not been switched to the new factory, and no TypeScript grammar
subset or partial native parser is introduced. The next step is to carry the construction lifetime through the
parser and binder, keep their writes inside this scope, and return its immutable root.

The existing per-file-region and representation costs from `file-arena-lifetimes.md`, the measured/rejected
list in `perf-round2-followups.md`, the techniques in `docs/RUST.md`, and `mem-round4.md` remain relevant. The
changed constraint is the explicitly requested lifetime migration. This stage removes the need for a graph
copy for callers of the new factory; it does not establish a production performance gain or change defaults.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test factory
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
tools/gen-check.sh --write
git diff --check
```

Five new runtime cases pass natively and under strict-provenance Miri. They cover node/parent/symbol/table/flow
cycles, mutable payload replacement and ranges, lists/modifiers, JSDoc, source metadata and ambient patterns,
both auxiliary flow payloads, chunk growth, cancellation, concurrent immutable readers and successive owners.
Captured addresses for roots, payloads, nodes, lists, symbols, flows, metadata and token bytes remain identical
after publication. Native scanner-to-factory construction consumes zero bytes of an entered legacy region and
survives its release.

Eleven new compile-fail examples reject escaped nodes, payloads, lists, factories and copied entries; bypassing
the scope with the value allocator; edges between nested scopes; concurrent construction mutation; published
mutation; and references surviving the arena or an external slice owner. The crate's existing runtime and doctest cases pass as well.
The selected conformance set remains exactly 1,497 passes, 203 skips and zero failures/crashes. The generator
check compares hashes before/after running all generators with `--write`, since this branch intentionally has
new generated output; all Rust sources must remain byte-identical after regeneration.

## Measurements

Two separate comparisons avoid attributing unimplemented parser/binder changes to the new factory:

```sh
# Save the measure executable built at cf268473, then build this revision.
cargo build --release -p tsrs_frontend_arena --examples
/tmp/measure-before-factory tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Also compare published before/after, and the same pinned XState/Drizzle manifests.

# Factory-only synthetic trees: identical value + 1; statements and parent links, retained together.
target/release/examples/factory legacy 256 4096
target/release/examples/factory native 256 4096
target/release/examples/factory legacy 16384 8
target/release/examples/factory native 16384 8
```

Each comparison has one warmup and five measured samples, reversing mode order each round. macOS
`proc_pid_rusage` instruction deltas and process peak RSS use the same release profile and mimalloc allocator.
The factory-only comparison includes construction, a syntax walk and parent-link assertions, with identical
checksums required. It does not scan, parse, bind, copy a legacy graph, or resolve dependencies. It is a
synthetic allocation workload, not a qualifying project benchmark or default-checker-count memory result.
The existing parse/bind/publication harness uses the three public source manifests pinned in the prototype
report, requires matching syntax checksums/diagnostic counts, and still exercises the old bridge.

Raw samples and medians are in [native-node-factory-results.json](native-node-factory-results.json).

Median factory-only results:

| Workload | Instructions, legacy → native | Change | Peak RSS, legacy → native | Change |
| --- | ---: | ---: | ---: | ---: |
| 256 files × 4,096 statements | 0.948B → 1.260B | +33.0% | 190.61 → 412.02 MiB | +116.2% |
| 16,384 files × 8 statements | 0.143B → 0.291B | +103.8% | 27.72 → 263.83 MiB | +851.8% |

This is still more expensive than the compressed legacy graph. Wider native records and separate
64 KiB arena chunks remain costs, especially for small files. Publishing without a copy does not remove
those representation and allocation costs. These measurements do not clear the performance bar and do
not justify switching the production frontend. This commit remains an isolated lifetime-correctness stage.

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.904987B → 19.898194B | -0.034% | 1515.92 → 1515.92 MiB |
| VS Code | published | 37.302665B → 37.321988B | +0.052% | 2990.23 → 2990.23 MiB |
| XState | tls | 0.320338B → 0.320310B | -0.009% | 28.61 → 28.61 MiB |
| XState | published | 0.624005B → 0.624128B | +0.020% | 57.06 → 57.06 MiB |
| Drizzle | tls | 1.396658B → 1.396722B | +0.005% | 107.33 → 107.33 MiB |
| Drizzle | published | 2.628423B → 2.628415B | -0.000% | 219.98 → 219.98 MiB |

All six instruction comparisons are within 0.06%, with identical peak RSS medians, syntax checksums and
diagnostic counts. This confirms the existing path remains stable; it is not evidence for the cost of a
future native parser/binder, which this harness does not yet run.
