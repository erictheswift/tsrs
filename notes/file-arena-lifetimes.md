# Per-file arenas with native Rust lifetimes: prototype

This experiment is in `tsrs_frontend_arena`, separate from the production compiler. Its public graph uses
`&'ast Node<'ast>`, borrowed payloads, borrowed lists, symbols, flow links and file metadata. It has no `P<Node>`
links or lifetime-erased references. The existing parser and binder still use `P` as temporary construction
handles; this does **not** complete their migration or change the CLI/project system's default representation.

The changed constraint relative to `mem-leaf-regions-cost.md` and `mem-free-leaf-files.md` is compile-time lifetime
checking, including multi-file borrows and immutable publication, rather than only earlier reclamation. The prior
all-file-region overhead and the measured/rejected list in `perf-round2-followups.md` still apply. This experiment
must not be presented as a production performance improvement.

## Ownership and publication

```rust
use tsrs_frontend_arena::{FileArena, ParseOptions, parse_and_bind};

let arena = FileArena::new();
let file = parse_and_bind(&arena, "export const answer = 42", ParseOptions::default());
let root = file.root; // &Node<'_>, tied to arena
std::thread::scope(|scope| {
    scope.spawn(|| assert!(root.source_file().is_some()));
});
```

The caller owns storage outside the graph; the returned `BoundFile` is a view, not a self-referential owner.
Independent owners can be borrowed for the same shorter checking scope. Retaining an old owner lets a request
finish reading its version while a new version uses different storage. Dropping the last owner is permitted only
when no remaining references will be used. `FileArena` is `Send` but not `Sync`; native immutable records are
`Send + Sync` without new unsafe trait implementations. Concurrent allocation through one arena is deliberately
unavailable. Moving a long-lived owning parsed-file package across arbitrary worker queues is not implemented;
this prototype demonstrates scoped graph sharing.

The native arena uses stable, owned chunks of `UnsafeCell<MaybeUninit<_>>` backing storage. It allocates directly,
without TLS or global registration, and never overwrites an older allocation. Records cannot have destructors.
Its owner releases all chunks on drop, including after an interrupted publication.

Publication has two stages:

1. Eager parse and bind in a temporary registered `Region`. A feature-gated factory accepts an explicit
   `ConstructionAllocator`; ordinary node/list/text allocations through that factory bypass TLS. Remaining
   legacy binder and helper allocations still use the scoped region through their existing APIs. Speculative
   parser and flow-label allocations are retained until the whole construction region is released. The source
   registration's cleanup callback is installed before parsing starts.
2. Copy the complete reachable graph into the native arena. Address maps allocate each destination once and a
   work queue fills raw records. Graph edges are `NonNull`, including edges in raw slices, until **every** record
   is initialized. Only then may the root be read as a native reference. Drop the maps, leave the construction
   scope and release its region before returning.

The raw/native record pairs have `repr(C)` and compile-time size, alignment and per-field offset assertions.
Payload enums share `repr(C, u16)` and identical generated variant order. There is no transmute of a live
mutable graph and no reference to an unfinished node. Parent, declaration, symbol and flow cycles become valid
at the same publication boundary. The implementation's unsafe operations are confined to arena allocation and
raw-record initialization/interpretation; the public graph contains no mutation cells.

The generator imports the existing AST schema and writes actual lifetime-parameterized payload structs and
visitors. `tools/gen-check.sh` regenerates both ASTs through `gen-borrowed.ts`. Handwritten publication also covers
JSDoc, symbol tables, flows, reference directives, pragmas, module indicators and ambient modules. Checker-only
synthetic `Any` payloads are rejected. Lazy parsing/binding, checker overlays, emit factories, content mapping,
project graph ownership, language-service caches and incremental reuse across files remain outside this phase.

Cooked JavaScript literals can contain lone UTF-16 surrogates. The legacy scanner puts CESU-8 bytes in `String`;
blindly copying those into a new `&str` failed on the public VS Code corpus. Published `Text<'ast>` therefore
borrows bytes and offers checked `as_str()`, preserving lone surrogates losslessly. The original source text and
file metadata strings remain UTF-8. The legacy scanner's string representation remains separate technical debt.

Miri also exposed a pre-existing destructor-provenance problem in `P::new` / `P::new_scratch`: their drop pointers
were derived from mutable references and could be invalidated by later interior mutation. They now retain the
original allocation pointer for destruction. A focused region/drop regression test covers that failure.

## Validation

Commands from this checkout:

```sh
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo test -p tsrs_core -p tsrs_ast -p tsrs_parser -p tsrs_binder -p tsrs_frontend_arena
TSRS_ARENA_POISON=1 TSRS_CHECK_SHARED=1 cargo test -p tsrs_frontend_arena
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
target/debug/tsrs-test run --suite all --filter 'parser|binder|jsdoc|controlFlow' --jobs 4
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --lib native_cycles_and_disjoint_growth
cargo +nightly-2026-10-04 miri test -p tsrs_frontend_arena --features plain-ptrs --test lifetimes
cargo +nightly-2026-10-04 miri test -p tsrs_core --features plain-ptrs --lib region_drops_values_after_interior_mutation
```

Six compile-fail examples reject escaped nodes, text, list iterators, symbol declarations, a multi-file cache,
and mutation through a published reference. Runtime cases exercise cross-file covariance, scoped concurrent
readers, retained old versions, detached diagnostics, repeated release, unwind, syntax recovery, JSDoc/JSX and
lone surrogates. Repeated-release coverage runs 32 cycles natively and three under Miri. The strict-provenance test constructs native payload/parent/flow/slice cycles without invoking
the legacy parser; full pipeline Miri uses the legacy exposed-provenance pointer APIs and warns about that limit.

The conformance selection had 1,043 passes, 86 skips, and no failures/crashes. Independently, field-by-field
publication verification passed for all 15,537 files from the three pinned public corpora below. This compares
all generated payload fields, child order, source ranges and flags, and handwritten symbol/flow/metadata edges
against their construction objects. Focused fixtures additionally compare normal eager-parser diagnostics and
syntax with both construction allocation modes. It is not a checker parity claim.

To repeat the public-corpus graph check, set `TSRS_LIFETIME_MANIFEST` to a newline-separated list of absolute
source paths and run:

```sh
cargo test -p tsrs_frontend_arena --lib public_corpus_matches_construction_field_by_field -- --ignored
```

Do not enable poison mode for the existing core free-list/reuse tests: four of those deliberately assert address
reuse, while poison mode deliberately disables it. Normal affected-crate verification and the prototype's
poison verification are separate runs.

## Measurements

The release example `measure` and `tools/bench-file-arena.py` compare eager front-end work. Source strings are
loaded before the timer and instruction snapshot. Each run visits syntax after binding, and forces JSDoc in
legacy modes to match publication. The harness requires identical syntax checksums and diagnostic counts across
all modes. It preloads identical input bytes in every process and records process peak RSS, so that metric also
includes inputs and allocator overhead.

Public source revisions come from `bench/projects.json`: VS Code `9cf0128b9822dcd8a533f08ef7312956a4390301`,
XState `fbee62e7c1586315ed478c2fedf530d7e0ff5a3e`, Drizzle `15454dbe49d827c6081f3d0231e2e7985e517295`.
Manifests contain all regular `.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`, `.mjs`, `.cjs` files in each source
archive, sorted by path, excluding `node_modules`. Dependencies are not installed or resolved. These are front-end
corpus measurements, **not** the end-to-end project results in `bench/RESULTS.md` or default-checker-count RSS.

```sh
cargo build --release -p tsrs_frontend_arena --example measure
target/release/examples/measure layout
python3 tools/bench-file-arena.py --out notes/file-arena-lifetimes-results.json \
  vscode=/absolute/vscode.txt xstate=/absolute/xstate.txt drizzle=/absolute/drizzle.txt
```

Modes: `tls` uses the original thread arena; `region` uses the original eager front end in retained per-file
regions; `retained` additionally keeps speculation/flow allocations and copies source text into construction;
`direct` matches `retained` but explicitly allocates factory nodes/lists/text; `published-tls` adds publication to
`retained`; `published` adds it to `direct`. The two `-drop` modes release each file as soon as it is visited.
The latter simulate a caller whose borrows have ended, not the checker’s actual dependency/cache lifetimes.

Each mode has one warmup and five fresh-process samples, with reversed mode order on alternate rounds. macOS
instruction values are `proc_pid_rusage(RUSAGE_INFO_V4)` deltas; they are not interchangeable with Linux
`perf_event_open` user-only deterministic counts. Use `bench/count.py` on Linux for the repository's landing gate.
No production performance gate is established by this experiment.

Results on macOS 27.0.1 / arm64 (2026-10-16), medians of five samples. The source checkout is based on
`a087e7e3`; all modes use the same release executable with the prototype feature compiled in. The TLS
baseline therefore includes the factory’s inactive prototype branch. Raw samples and arena byte counters are
in [file-arena-lifetimes-results.json](file-arena-lifetimes-results.json).

| Corpus | Mode | Instructions (G) | Peak RSS (MiB) | Wall (ms) |
|---|---|---:|---:|---:|
| vscode | tls | 19.810 | 1515.9 | 1452.5 |
| vscode | region | 20.140 | 1702.0 | 1482.9 |
| vscode | retained | 19.868 | 1728.6 | 1461.2 |
| vscode | direct | 19.484 | 1728.6 | 1437.2 |
| vscode | published-tls | 37.586 | 2990.2 | 2889.4 |
| vscode | published | 37.196 | 2990.2 | 2860.0 |
| vscode | region-drop | 19.859 | 258.6 | 1429.2 |
| vscode | published-drop | 35.978 | 380.9 | 2700.6 |
| xstate | tls | 0.320 | 28.6 | 23.8 |
| xstate | region | 0.326 | 33.1 | 24.4 |
| xstate | retained | 0.323 | 32.8 | 24.1 |
| xstate | direct | 0.317 | 32.8 | 23.6 |
| xstate | published-tls | 0.630 | 57.0 | 48.3 |
| xstate | published | 0.624 | 57.0 | 48.0 |
| xstate | region-drop | 0.321 | 9.3 | 23.4 |
| xstate | published-drop | 0.604 | 13.4 | 45.3 |
| drizzle | tls | 1.395 | 107.3 | 92.8 |
| drizzle | region | 1.414 | 117.9 | 94.1 |
| drizzle | retained | 1.401 | 118.4 | 93.3 |
| drizzle | direct | 1.374 | 118.4 | 91.4 |
| drizzle | published-tls | 2.658 | 219.9 | 188.9 |
| drizzle | published | 2.629 | 219.9 | 187.1 |
| drizzle | region-drop | 1.401 | 21.9 | 91.6 |
| drizzle | published-drop | 2.548 | 36.4 | 176.4 |

Corpus sizes: VS Code 14,118 files, XState 447 files, Drizzle 972 files. All allocation modes produced the
same syntax checksum and diagnostic count within each corpus.

On this 64-bit compressed-pointer build, legacy/native links are 4/8 bytes, node headers 24/56 bytes, and
node lists 16/32 bytes. The prototype also keeps a payload tag and JSDoc slice in each header and allocates
payloads separately; its increased storage is not caused by lifetimes alone. Its 64 KiB minimum chunks add
unused reserved capacity for small files. Arena byte counters omit legacy auxiliary heap allocations; peak RSS
includes them and includes the temporary construction/publication overlap.

## Decision

**Keep this as an isolated correctness prototype; do not switch production defaults.** Explicit factory
allocation reduced eager front-end instructions by 1.93% / 1.83% / 1.96% (VS Code / XState / Drizzle) against
otherwise identical retained construction. Publication erased that gain: compared with the original TLS path,
it used 87.8% / 95.2% / 88.5% more instructions and 97.3% / 99.6% / 105.0% more peak RSS.

Releasing each native file promptly reduced peak RSS by 87.3% / 76.5% / 83.5% compared with retaining all native
files. Existing per-file regions released promptly used still less memory. This demonstrates bounded ownership
and reclamation, not a new end-to-end checker memory improvement.

The next implementation stage needs direct construction of the final lifetime-aware representation to remove
the copy and temporary overlap, plus a smaller native header/list layout and better chunk sizing. Then measure
actual checker retention and caches before choosing a production migration. Removing `P` from the legacy
builders, checker and emit phases is still future work. The public graph and its borrowing rules provide a
concrete target for that work.

The next committed step is [scanner source lifetimes](scanner-lifetimes.md), which removes the scanner's
requirement for static input text before migrating native token and node allocation.
