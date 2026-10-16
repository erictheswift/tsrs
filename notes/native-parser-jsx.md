# Native JSX names, text and closing tags

Continuation of [native import clauses](native-parser-imports.md), starting at `57270235` on
`codex/file-arena-lifetimes`.

## Shared grammar

`JsxParser` shares six existing routines with the scoped native parser: JSX element names, tag names,
attribute names, text, closing elements and closing fragments. The production parser forwards to the same
bodies. Native nodes, property-access chains, namespaces and text borrow the per-file arena; they do not
allocate legacy nodes or copy a legacy graph. Storage adapters supply constructors, JSX scanner operations
and tag-name access/equivalence. Native name equivalence follows `ast::tag_names_are_equivalent`, including
recursive property access, `this` and namespaced names.

The scanner transitions remain significant. A matching nested closing tag resumes JSX text scanning; a
mismatched closing tag or a closing tag in expression context resumes ordinary token scanning. Namespaced
names stop before a dot. JSX identifiers allow hyphens but keep the existing diagnostics for Unicode escape
sequences. `this` becomes a keyword expression only when it is not a namespace prefix.

The child re-scan operation deliberately returns the scanner token without assigning the parser's token,
as in the existing child loop. It drains scan errors each time. Re-scanning malformed text can mark the text
node as erroneous after the first scan's error was already consumed by the opening node; the differential
fixtures preserve that caller protocol. Scanner and parse diagnostics retain their existing order, ranges,
deduplication and pending-error behavior.

Opening elements, JSX child loops, expression/spread attributes, type arguments, complete JSX expressions,
full source-file parsing and native binding remain to migrate. Attribute string fixtures exercise the newly
available JSX attribute-value scanner with the existing shared literal grammar; they do not introduce a
partial attribute-expression parser. CLI/checker behavior and the `parse_and_bind` bridge are unchanged.

No unsafe code, node/list layouts, thread traits or allocation policy change. Retained speculative nodes
remain valid after rewind, and publication freezes the same construction records.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test jsx native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter|import|Import|export|Export|jsx|Jsx|JSX' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Eight new runtime cases cover 17 element names, 12 attribute-name/string fixtures and eight text/closing-tag
fixtures in both TSX and JSX, plus four fragment fixtures. They compare node kinds, ranges, flags, payloads,
parent edges and diagnostics with complete legacy source-file parsing. The malformed fragment comparison
covers the closing routine's diagnostics; subsequent expression recovery is outside its scope.

Native cases cover matching/mismatched names and both scanner continuations, namespaces stopping at dots,
unterminated attribute strings, pending-error restoration and speculative text retained across rollback and
replay. Retained text survives source/parser/legacy-region release, subsequent arena growth and reads from
two scoped threads. That native construction uses zero bytes in the entered legacy region. Detached diagnostics
remain readable after arena release.

All 68 enabled arena runtime cases, 25 compile-fail doctests and three positive doctests pass; two existing
runtime cases remain ignored. Strict-provenance Miri passes the four native JSX cases. All 15,537 public-source
AST hashes match the previous revision. The expanded 6,136-variant conformance selection has the identical
pass list: 5,223 passes, 912 skips, the same one existing codes-only result
(`compiler/typeParameterWithInvalidConstraintType`), and no failures, crashes or timeouts. Workspace clippy,
source lint and whitespace checks pass.

## Measurements and scope

The prior migration reports, `docs/RUST.md`'s techniques, `perf-round2-followups.md`'s rejected list and
`mem-round4.md` were reviewed. This continues the owner's requested lifetime migration, with no performance
claim or production-default switch. Existing native record widths, per-file chunk costs and the copying bridge
are unchanged; this step does not address the earlier native-versus-legacy memory regressions.

The existing public parse/bind harness compares this stage against `57270235`, in legacy `tls` mode and
`published` mode (legacy parse/bind plus native graph publication). It is an incremental regression screen,
not a complete native parser or an end-to-end checker comparison against main. Release builds use mimalloc,
macOS `proc_pid_rusage` instruction deltas and peak process RSS. Each case has one warmup and five samples
in alternating before/after order; compilation, conformance and Miri finish before sampling. Checksums and
diagnostic totals must agree across revisions and modes.

```sh
# Save the release harness at 57270235, then rebuild after the change.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-jsx tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat in published mode and on the pinned XState and Drizzle manifests.
```

Raw samples and medians are in [native-parser-jsx-results.json](native-parser-jsx-results.json).

| Corpus | Mode | Instructions before → after | Change | Peak RSS before → after |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.918544B → 19.922416B | +0.0194% | 1515.95 → 1515.95 MiB |
| VS Code | published | 37.386466B → 37.375593B | -0.0291% | 2990.27 → 2990.27 MiB |
| XState | tls | 0.320967B → 0.320386B | -0.1811% | 28.62 → 28.62 MiB |
| XState | published | 0.625120B → 0.626322B | +0.1923% | 57.08 → 57.08 MiB |
| Drizzle | tls | 1.402717B → 1.401961B | -0.0539% | 107.36 → 107.36 MiB |
| Drizzle | published | 2.638107B → 2.646087B | +0.3025% | 220.00 → 220.00 MiB |

Median peak RSS is identical in all six comparisons. Instruction medians range from -0.1811% to
+0.3025%, and before/after sample ranges overlap in each comparison. The small-corpus runs vary enough
that this screening does not establish a consistent speed change: for example, Drizzle published samples
range from 2.636558B to 2.651815B before and 2.634760B to 2.651576B after. There is no qualifying
performance gain. These results do not measure the cumulative branch against main or remove the previously
measured native representation and copying costs.

The next stage shares [JSDoc link and name grammar](native-parser-jsdoc.md).
