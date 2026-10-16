# Native JSDoc links and names

Continuation of [native JSX names and text](native-parser-jsx.md), starting at `a94c6d88` on
`codex/file-arena-lifetimes`.

## Shared grammar

`JSDocParser` shares the existing link, link-prefix, link-name, name-reference, entity-name and identifier
routines with the scoped native parser. It also shares whitespace/asterisk handling, the whitespace-to-EOF
probe, and expected/optional JSDoc token consumption. Production parsing forwards to these same bodies.

`{@link}`, `{@linkcode}` and `{@linkplain}` allocate native nodes and borrowed text-part slices directly.
Qualified names preserve dots, `#member` conversion and the existing recovery for `Foo.#member`. Name
references retain the enclosing `@see` parser's name-start precondition. Parameter/property entity names
consume accepted `[]` suffixes without storing them, exactly as before. Identifiers use the scanner's checked
UTF-8 token view; comment text retains its original source slices and part boundaries.

The common token interface now exposes `finish_node_with_end`. Native `finish_node` delegates to it, keeping
the same range/flag/parent assignment order. This lets a JSDoc identifier end at the identifier token even
when subsequent whitespace remains visible, and lets a link include the closing brace without consuming it.

JSDoc and ordinary scanning remain distinct. Name-reference parsing resets to the next token's full start
before resuming JSDoc scanning; hash re-scanning precedes `#member` name parsing. Whitespace before the end
of a comment remains unconsumed. Indentation after newlines and leading asterisks follows the existing helper.
Failed link probes rewind scanner/parser state and diagnostics; native allocations remain valid after rewind.

Complete comment/tag parsing, source-prefix setup/restoration, diagnostic routing into the JSDoc stream,
JSDoc attachment/reparsing and type expressions remain with the enclosing legacy parser. Native callers
supply comment bounds, context flags and the initial token. The scoped routines currently return ordinary
parse diagnostics; their future enclosing comment parser must route them as the legacy comment parser does.
Full expression/type/source-file parsing, native binding and checker migration remain unfinished. The CLI
and `parse_and_bind` bridge retain their existing frontend paths.

No unsafe code, construction/publication layouts, thread traits or arena allocation policy change. Native
link/name nodes contain scoped references; their published graphs and text borrow the owning file arena.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test jsdoc native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter|import|Import|export|Export|jsx|Jsx|JSX' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Seven new runtime cases cover 18 link fixtures in both JS and TS, ten `@see` name references and nine
parameter entity names. Differential cases compare node kinds, ranges, flags, parent edges, cooked name
bytes and individual display-text parts with complete legacy source-file JSDoc parsing. JS cases also compare
diagnostic order, codes, spans, messages and arguments against the legacy JSDoc diagnostic stream.

Native cases check failed-prefix rollback, existing diagnostics and pending parse errors, whitespace at EOF,
indentation/asterisk boundaries, exact identifier ends and comment-mode token consumption. Retained links
and escaped names survive rewind/replay, source/parser/legacy-region release, arena growth and reads from
two scoped threads. That native construction uses zero bytes in the entered legacy region.

All 75 enabled arena runtime cases, 25 compile-fail doctests and three positive doctests pass; two existing
runtime cases remain ignored. Strict-provenance Miri passes all four native JSDoc cases. The public-source
oracle produces identical hashes for all 15,537 files. The 6,136-variant conformance selection retains the
identical pass list: 5,223 passes, 912 skips, the same one existing codes-only result
(`compiler/typeParameterWithInvalidConstraintType`), and no failures, crashes or timeouts. Workspace clippy,
source lint and whitespace checks pass.

## Measurements and scope

The prior migration reports, `docs/RUST.md`'s techniques, `perf-round2-followups.md`'s rejected list and
`mem-round4.md` were reviewed during this migration. This is the owner's requested lifetime migration, with
no performance claim or production-default switch. Wider native records, per-file chunk costs and the legacy
graph copying bridge are unchanged; this stage does not resolve the previously measured native memory costs.

The existing public parse/bind harness compares this stage against `a94c6d88`, in legacy `tls` mode and
`published` mode (legacy parse/bind plus native graph publication). This screens the incremental shared-grammar
change, not a complete native frontend or the cumulative branch against main. Each case has one warmup and
five alternating before/after samples, release builds, mimalloc, macOS `proc_pid_rusage` instruction deltas
and peak process RSS. Builds, Miri and conformance finish before sampling. Checksums and diagnostic totals
must match across both revisions and both modes.

```sh
# Save the release harness at a94c6d88, then rebuild after the change.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-jsdoc tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat in published mode and on the pinned XState and Drizzle manifests.
```

Raw samples and medians are in [native-parser-jsdoc-results.json](native-parser-jsdoc-results.json).

| Corpus | Mode | Instructions before → after | Change | Peak RSS before → after |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.901685B → 19.898652B | -0.0152% | 1515.95 → 1515.95 MiB |
| VS Code | published | 37.379060B → 37.370065B | -0.0241% | 2990.27 → 2990.27 MiB |
| XState | tls | 0.320061B → 0.320060B | -0.0002% | 28.62 → 28.62 MiB |
| XState | published | 0.624869B → 0.624640B | -0.0366% | 57.08 → 57.08 MiB |
| Drizzle | tls | 1.399040B → 1.399037B | -0.0002% | 107.36 → 107.36 MiB |
| Drizzle | published | 2.634740B → 2.633901B | -0.0318% | 220.00 → 220.00 MiB |

Median peak RSS is identical in all six comparisons. Instruction medians change by -0.0366%
to -0.0002%, below 0.04% in magnitude, with matching checksums and diagnostic totals. This does
not establish a qualifying performance gain. These incremental results do not measure the cumulative branch
against main or remove the previously measured native representation and graph-copying costs.
