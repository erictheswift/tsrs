# Native import clauses and module export names

Continuation of [native list parsing](native-parser-lists.md), starting at `b05983ad` on
`codex/file-arena-lifetimes`.

## Shared grammar

`ImportParser` shares the existing import-clause, namespace-import, named-import and import-specifier
routines with the scoped native parser. It also shares module export names, namespace exports, the
import/export specifier worker and three token/lookahead helpers used by the enclosing import declaration.
Production parsing forwards to the same bodies; there is no duplicate native grammar or legacy-node fallback.

The `type`/`as` decisions preserve the existing order: `type`, `type as`, `type as as` and `type as as as`
produce the same type-only marker, imported property and local name as before. Arbitrary string export
names retain their cooked bytes, including lone UTF-16 surrogates. Invalid local binding names still
produce a missing identifier at the same position with the same diagnostics and error-flag placement.
Namespace and named imports finish directly in native arena slots with ordinary construction references.

The shared clause routine retains its caller-supplied phase (`type` or `defer`), optional default binding,
source range and JSDoc scanner mode. It restores the prior await-identifier marker, and the JSDoc
leading-asterisk setting is balanced as in the original routine. Enclosing parsing contexts remain a caller
responsibility, as for all shared list grammar; the differential fixtures install the source-file context
when comparing malformed import recovery against complete source-file parsing.

Full import/export declarations, phase classification, module-specifier expressions, import attributes and
export JSDoc attachment have not migrated. The shared export-specifier worker returns its existing tuple;
the production wrapper still attaches JSDoc and performs export syntax checks. Native tests construct the
wrapper only to compare that worker's result. Full expression/type/statement/source-file parsing and native
binding remain to migrate. The CLI and `parse_and_bind` retain their existing frontend paths and `P<Node>`.

## Diagnostics and lifetime

The import-specifier branch of `check_js_syntax`, including its JavaScript/JSDoc/reparsed guard, is shared.
The legacy dispatcher uses this same helper; import specifiers have no decorators, type parameters or
modifiers for its other branches to check. `skip_range_trivia` also moves to the common token interface,
using the scanner's source text. Legacy initialization and temporary JSDoc source truncation keep that text
synchronized with the parser's source field.

Native JavaScript syntax diagnostics have a separate owned stream. Unlike parse errors, they are not
suppressed at repeated positions and do not set the pending parse-error flag on the next node. Checkpoints
record and restore its length alongside the existing parse stream, scanner and parser state. Both streams
can be detached together with `into_diagnostic_streams`; the existing `into_diagnostics` continues returning
parse diagnostics. Detached messages and arguments survive parser and arena release.

No unsafe code, construction/publication layouts or thread traits change. Speculative graph allocations
remain live through rollback and replay; publication freezes the same branded records. No arena recycling
experiment is reintroduced.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test imports native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter|import|Import|export|Export' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Seven new runtime cases cover 22 import-clause fixtures in TS and JS, nine export-specifier fixtures,
four namespace-export names, missing tokens, string/keyword binding-name recovery, escaped `type`,
trailing commas and parent edges. Parse and JavaScript diagnostic streams are compared separately; only
specifier-level JavaScript diagnostics are in the native clause's scope. Native cases check stream rollback,
existing-diagnostic retention, duplicate JS errors, pending parse errors, JSDoc/reparsed suppression,
await-marker restoration, JSDoc scanner-mode restoration and the side-effect-import probe.

All 60 enabled arena runtime cases, 25 compile-fail doctests and three positive doctests pass; two existing
runtime cases remain ignored. Strict-provenance Miri passes all five native cases. The retained native import
graph and lone-surrogate string survive input/parser/legacy-region release, subsequent arena growth and
reads from scoped threads. Native parsing allocates zero bytes in the entered legacy region. Differential
cases involving the legacy graph run natively, as described in earlier migration reports.

All 15,537 public-source AST hashes match the pre-refactor oracle. The expanded selection has an identical
pass list across 5,944 conformance variants: 5,035 passes, 908 skips, the same one existing codes-only result
(`compiler/typeParameterWithInvalidConstraintType`), and no failures/crashes/timeouts. Workspace clippy,
source lint and whitespace checks pass.

## Measurements and scope

The prior migration reports, `docs/RUST.md`'s techniques, `perf-round2-followups.md`'s rejected list and
`mem-round4.md` were reviewed. This remains the owner's requested lifetime migration, with no performance
claim or default frontend switch. It does not revisit rejected allocation-recycling or inlining experiments.

```sh
# Save the release harness built at b05983ad, then rebuild after the change.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-imports tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat in published mode and on the pinned XState and Drizzle manifests.
```

The existing parse/bind harness screens shared grammar on the legacy frontend, with and without native
graph publication. It does not measure a complete native parser, binder or checker. Each comparison has
one warmup and five samples in alternating order, release builds, mimalloc, macOS `proc_pid_rusage`
instruction deltas and peak process RSS. Builds, Miri and conformance finish before sampling. Checksums and
diagnostic totals must agree across versions and modes. Raw samples and medians are in
[native-parser-imports-results.json](native-parser-imports-results.json).

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.877809B → 19.872309B | -0.0277% | 1515.95 → 1515.95 MiB |
| VS Code | published | 37.301937B → 37.294206B | -0.0207% | 2990.27 → 2990.27 MiB |
| XState | tls | 0.320103B → 0.320196B | +0.0291% | 28.62 → 28.62 MiB |
| XState | published | 0.624881B → 0.624783B | -0.0158% | 57.08 → 57.08 MiB |
| Drizzle | tls | 1.399908B → 1.399679B | -0.0163% | 107.36 → 107.36 MiB |
| Drizzle | published | 2.634410B → 2.634617B | +0.0079% | 220.00 → 220.00 MiB |

Median peak RSS is unchanged in all six comparisons. Instruction changes are within ±0.03%, with
matching checksums and diagnostic counts. This does not establish a qualifying performance gain. The
stage remains isolated on the migration branch; native source-file parsing/binding and their retained
speculative allocations still need measurement before a production-default switch.
