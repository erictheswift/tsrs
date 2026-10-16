# Native parser lists and recovery

Continuation of [native parser context and lookahead](native-parser-context.md), starting at `eb3e1114`
after rebasing `codex/file-arena-lifetimes` onto main `d5682df5`.

## Shared grammar and ownership

`ListParser` shares the production parser's indexed, ordinary, delimited and bracketed list routines,
list finishing, empty/missing list creation, element/terminator recognition, enclosing-context recovery
and context-specific diagnostics. The legacy parser forwards to these bodies. Token advances,
error ordering, recovery branches and JSDoc declaration propagation keep their existing structure.
Now-unused legacy forwarding methods for scanner lookahead have been removed.

The native implementation stores construction references in its reusable node stack and reparse queue.
Nested lists finish above the outer list's stack position, allocate their final slice in the file arena,
and restore the outer stack length and parsing contexts. A failed element callback returns `None` and
restores that same scratch state. Reparsed JSDoc type/import declarations propagate outward until a
source or block statement list can accept them, preserving their original order and index accounting.
Native JSDoc grammar itself remains to migrate; a construction fixture exercises this queue protocol.

As in the legacy parser, checkpoints do not snapshot the scratch stack or list-context mask: grammar
callbacks must complete each list before rewinding. Native rollback retains allocations, so a completed
speculative list, its elements and cooked token text remain valid through replay and later arena growth.
`Parser::factory` borrows the parser's existing scoped factory for list-element callbacks and enclosing
node construction. It does not duplicate the factory or permit a second parser in the same scope.

Missing lists now have an explicit `is_missing` bit in native construction and published records. This
preserves the distinction between an absent opening delimiter and a present empty list without copying
the legacy raw-pointer sentinel. Legacy graph publication also preserves the bit. The construction
record is regenerated from the handwritten model, including the existing field-offset/layout assertions.
Parsed native lists derive trailing-comma state from the last element's end and the list's end, matching
the legacy parsed-list rule; clone-only legacy node markers are not part of this construction path.

No new unsafe code, unsafe thread traits or lifetime erasure is introduced. Lists and their mutable
cells cannot escape construction, and published lists borrow their arena and can be read by scoped
threads. The test drivers combine migrated leaves and lists; they are not new production expression or
type parsers. Full expression/type/statement/source-file grammar and native binding remain to migrate.
`parse_and_bind` still copies the legacy graph, and production callers still use `P<Node>`.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test lists native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
node tools/gen-ast/gen-borrowed.ts
git diff --check
```

Eight new runtime cases cover array/tuple lists against real TS/JS source-file parsing, empty versus
missing parameters through native construction and legacy publication, nested recovery, failed callbacks,
semicolon recovery, zero-width progress, speculative list retention, JSDoc queue propagation and list indexes.
They compare positions, flags, trailing commas, text, diagnostics and parent edges. Retained lists survive
input/parser/legacy-region release and are read on scoped threads. The native path allocates zero bytes
in the entered legacy region. A new compile-fail doctest rejects a parsed list escaping construction.

All 53 enabled arena runtime cases, 25 compile-fail doctests and three positive doctests pass (two existing
runtime cases remain ignored). Strict-provenance Miri passes the six native cases. The two differential
cases use the legacy frontend and run natively, as described in the earlier migration reports.

All 15,537 public-source AST hashes match the saved pre-refactor oracle. The 4,381 selected conformance
variants have an identical pass list: 3,720 passes, 660 skips, the same one existing codes-only result
(`compiler/typeParameterWithInvalidConstraintType`), and no failures/crashes/timeouts.

## Measurements and scope

Prior migration reports, `docs/RUST.md`'s techniques, the rejected list in `perf-round2-followups.md` and
`mem-round4.md` were reviewed. This is the owner's requested lifetime migration, with no performance claim
or production-default switch. Speculative allocation recycling is not revisited: borrowed speculative
nodes must remain live until the file arena drops.

```sh
# Build and save this executable at eb3e1114 before changing the parser.
cargo build --release -p tsrs_frontend_arena --example measure
cp target/release/examples/measure /tmp/measure-before-lists
# Rebuild after the change, then alternate before/after over the pinned public manifests.
/tmp/measure-before-lists tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat in published mode and for XState and Drizzle.
```

The existing parse/bind harness screens the shared grammar on the legacy frontend, with and without
native graph publication. It does not measure a complete native parser, binder or checker. Each comparison
has one warmup and five measured samples in alternating order, release builds, mimalloc, macOS
`proc_pid_rusage` instruction deltas and peak process RSS. Checksums and diagnostic counts must match.
Raw samples and medians are in [native-parser-lists-results.json](native-parser-lists-results.json).

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.909331B → 19.876145B | -0.1667% | 1515.95 → 1515.95 MiB |
| VS Code | published | 37.286063B → 37.247967B | -0.1022% | 2990.27 → 2990.27 MiB |
| XState | tls | 0.320847B → 0.320226B | -0.1936% | 28.62 → 28.62 MiB |
| XState | published | 0.624814B → 0.624815B | +0.0002% | 57.08 → 57.08 MiB |
| Drizzle | tls | 1.403445B → 1.399892B | -0.2532% | 107.36 → 107.36 MiB |
| Drizzle | published | 2.636971B → 2.634750B | -0.0842% | 220.00 → 220.00 MiB |

Median peak RSS is unchanged in all six comparisons. Instructions range from -0.2532% to +0.0002%;
this is a screening result below the project performance gate, not a qualifying speedup. The native list
record remains 32 bytes: the missing-list bit uses existing padding. Checksum and diagnostic totals agree
across both revisions and both modes. Builds, Miri and conformance finished before sampling.

Workspace clippy, source lint and whitespace checks pass. Regenerating the AST and construction records
reproduces the checked-in generated content. The stage remains isolated on the migration branch; complete
native source-file parsing/binding and their allocation-retention costs still need measurement before a
default frontend switch.
