# Native JSDoc tag descriptions

Continuation of [native JSDoc links and names](native-parser-jsdoc.md), starting at `26de5456` on
`codex/file-arena-lifetimes`.

## Shared grammar

`JSDocParser` now shares tag-description parsing, trailing indentation handling, unknown tags, `@see`
and simple-tag construction between the existing frontend and the scoped native parser. The legacy
entry points forward to the same grammar bodies. The description state machine keeps its existing
handling of whitespace, leading asterisks, inline backticks, fenced code blocks, embedded links and
resumption at the next tag. `@see` preserves the name-reference probe and URL exclusion.

Native descriptions contain arena-owned lists of borrowed text parts and native link references. Source
text already belongs to the file arena; indentation produced in a temporary string is copied into that
arena before being retained. The native parser reuses two scratch vectors, clearing them after each
complete description parse. Retained lists and text do not borrow those vectors. As with list parsing,
callers finish each parse before rewinding; checkpoints do not restore scratch buffers.

The enclosing comment parser still supplies comment bounds, context flags, the initial token, tag names,
indentation and simple-tag constructors. Complete comment parsing and tag dispatch, including the
`@deprecated` attachment side effect, remain legacy. Typed tags, JSDoc attachment/reparsing, diagnostic
routing into the JSDoc stream, the recursive expression/type/source-file parser, native binding and
checker migration remain unfinished. The production CLI and `parse_and_bind` copying bridge retain
their existing frontend paths.

No unsafe code, construction/publication layouts, thread traits or arena allocation policy change.
The newly migrated native routines use scoped references instead of `P<Node>`.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test jsdoc_comments native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter|import|Import|export|Export|jsx|Jsx|JSX' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Six new runtime cases include 232 differential fixtures: six simple tags plus an unknown tag with
15 description forms, and 11 `@see` forms, each in JS and TS. They compare node kinds, spans, flags,
parent edges, names, individual text parts and comment-list metadata against complete legacy
source-file parsing. JS fixtures also compare diagnostics, including malformed link/name recovery.

Four native cases cover empty descriptions and scratch-buffer reuse after failed link probes, temporary
indentation release, fenced-code token handling and scanner reset at the next tag. Retained descriptions
and escaped link names survive rewind/replay, source/parser/legacy-region release, arena growth and
reads from two scoped threads. Their native construction uses zero bytes in the entered legacy region.

All 81 enabled arena runtime cases, 25 compile-fail doctests and three positive doctests pass; two existing
runtime cases remain ignored. Strict-provenance Miri passes all four new native cases. All 15,537 public
source files retain identical AST hashes. The 6,136-variant conformance selection retains its identical
pass list: 5,223 passes, 912 skips and the same existing codes-only result
(`compiler/typeParameterWithInvalidConstraintType`), with no failures, crashes or timeouts.
Workspace clippy, source lint and whitespace checks pass.

## Measurements and scope

The previous migration reports, `docs/RUST.md`'s techniques, `perf-round2-followups.md`'s rejected list
and `mem-round4.md` were reviewed during this migration. This is the owner's requested lifetime
migration, with no performance claim or production-default switch. Wider native records, per-file
chunk costs and the legacy graph copying bridge are unchanged.

The existing public parse/bind harness compares this stage with `26de5456`, in `tls` mode and
`published` mode (legacy parse/bind plus native graph publication). This screens the incremental
shared-grammar change. It does not measure the native description routines in a complete native
frontend or the cumulative branch against main. Each case uses one warmup and five alternating
before/after samples, release builds, mimalloc, macOS `proc_pid_rusage` instruction deltas and peak
process RSS. Builds, Miri and conformance finish before sampling. Checksums and diagnostic totals
must match across both revisions and modes.

```sh
# Save the release harness at 26de5456, then rebuild after the change.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-jsdoc-comments tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat in published mode and on the pinned XState and Drizzle manifests.
```

Raw samples and medians are in [native-parser-jsdoc-comments-results.json](native-parser-jsdoc-comments-results.json).

| Corpus | Mode | Instructions before → after | Change | Peak RSS before → after |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.919084B → 19.920692B | +0.0081% | 1515.95 → 1515.94 MiB |
| VS Code | published | 37.402451B → 37.400081B | -0.0063% | 2990.27 → 2990.25 MiB |
| XState | tls | 0.320114B → 0.320139B | +0.0080% | 28.62 → 28.61 MiB |
| XState | published | 0.624794B → 0.624776B | -0.0030% | 57.08 → 57.06 MiB |
| Drizzle | tls | 1.399307B → 1.399563B | +0.0183% | 107.36 → 107.34 MiB |
| Drizzle | published | 2.634362B → 2.634336B | -0.0010% | 220.00 → 219.98 MiB |

Instruction medians change by -0.0063% to +0.0183%; before/after sample ranges overlap in
every case. Median peak RSS is 16 KiB lower in each comparison, an immaterial difference.
Checksums and diagnostic totals match throughout. These results do not establish a qualifying
performance gain or remove the previously measured native representation and graph-copying costs.
