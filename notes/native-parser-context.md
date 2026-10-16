# Native parser checkpoints and syntax lookahead

Continuation of [native token/literal parsing](native-token-parser.md), starting at `625c4e09` on
`codex/file-arena-lifetimes`. The native parser now has construction-scoped checkpoints, nested lookahead,
context flag scopes, syntax-start decisions, semicolon recovery and individual modifier parsing.

```rust
use tsrs_frontend_arena::{ContextParser, FileArena, LookaheadParser, ScriptKind, TokenParser};
let arena = FileArena::new();
let root = arena.build(|factory| {
    let mut parser = factory.parser("async function", ScriptKind::TS);
    parser.next_token();
    assert!(parser.look_ahead(|p| p.parse_contextual_modifier(tsrs_ast::Kind::AsyncKeyword)));
    // Lookahead restored the token; the real parse can still consume the modifier.
    parser.try_parse_modifier(false, false, false).unwrap()
});
assert_eq!(root.kind, tsrs_ast::Kind::AsyncKeyword);
```

## State and ownership

`ContextParser` shares the existing `look_ahead`, `set_context_flags`, `do_in_context` and five context
queries. The legacy adapter uses its existing checkpoint and rewind implementation, including diagnostic
and JSDoc bookkeeping, arena pinning, and allocation recycling. The existing out-of-line lookahead boundary
is retained in the shared implementation.

The native `ParserState<'build, 'ast>` contains the scanner snapshot, context flags, diagnostic length,
await-identifier marker and pending-error flag. Its invariant construction brand prevents it from escaping
`FileArena::build`; a compile-fail doctest checks that restriction. Native rewind restores all of this
state, including scanner token values, positions and comment directives, without reclaiming arena storage.
References to speculative nodes, cooked token bytes and scanner caches can therefore remain live through
rollback, another parse and later arena growth. The arena releases that storage when its owner drops.

This is logical parser rollback, not a transaction over mutations to previously constructed nodes. As with
the legacy parser, grammar callers must respect the checkpoint protocol. Additional mutable parser state
introduced by later migrations must follow the legacy snapshot/retention rules. `do_in_context` restores
context flags while keeping parsing progress and the await-identifier marker, matching its original behavior.

Name parsing now uses this shared lookahead path instead of its separate native scanner-only snapshot.
Pending errors present before a checkpoint are restored; errors generated speculatively are removed and
are emitted once when parsing resumes. Nested lookahead preserves the outer diagnostic stream.

## Shared syntax decisions

`LookaheadParser` contains the production parser's 53 scanner/modifier routines for:

- Declaration, statement, expression, type and parameter starts, including nested import/type lookahead,
  contextual keywords, using/await-using declarations and heritage clauses.
- Type/class member starts, literal property names and binary-operator context checks.
- Contextual modifier recognition, individual modifier construction, static blocks/duplicate static
  modifiers, const type-parameter modifiers and line-break restrictions.
- Explicit semicolons, automatic semicolon insertion and missing-semicolon diagnostics.

The production parser forwards to these same bodies. Function names, branch ordering, token advances,
lookahead commit/rollback decisions and diagnostic calls are preserved. Each native modifier is an ordinary
construction reference and freezes in the same final arena slot. The shared routines provide no second
partial TypeScript grammar and no fallback to legacy nodes.

Modifier lists with decorators, list parsing/recovery, full expression/type/statement parsing, source-file
metadata and binding remain to migrate. `parse_and_bind` still builds a legacy graph and copies it, and
production callers/checkers still use `P<Node>`. The CLI and default frontend remain unchanged.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test context native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template|modifier|Modifier|declaration|Declaration|using|Using|parameter|Parameter' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

All enabled arena runtime cases, 24 compile-fail doctests and three positive doctests pass. Nine new
runtime cases cover native modifier metadata against real TS/JS source-file parsing, nested context
restoration, successful/failed modifier speculation, static/const modifier options, syntax-start decisions,
semicolon recovery, scanner directive rollback and diagnostic replay. They also retain speculative nodes
and lone-surrogate cooked bytes through rewind and growth, release the input/parser/legacy region, and
read the published graph on scoped threads. Native parsing allocates zero bytes in the entered legacy region.

Strict-provenance Miri passes all eight native cases. The differential modifier case runs natively because
of the legacy payload provenance limitation documented in the preceding reports. No unsafe code or generated
record layouts change.

The saved pre-refactor oracle and rebuilt oracle produce identical AST hashes for all 15,537 public source
files (VS Code, XState and Drizzle; revisions/manifests in `file-arena-lifetimes.md`). The expanded 4,381-variant
conformance selection has an identical pass list: 3,720 passes, 660 skips and the same one existing codes-only
result (`compiler/typeParameterWithInvalidConstraintType`), with no failures/crashes/timeouts. Workspace
clippy, source lint and the whitespace check pass.

## Measurements and scope

The prior lifetime/scanner/factory/name/token reports, `docs/RUST.md`'s techniques,
`perf-round2-followups.md`'s rejected list and `mem-round4.md` were reviewed before this stage. The changed
constraint remains the owner's requested lifetime migration, not a new performance claim. In particular,
this does not revisit speculative arena recycling: native references require keeping borrowed allocations
alive, and the additional retention cost of a complete native source-file parser remains to be measured.

```sh
# Save the release measure executable built at 625c4e09 before building this revision.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-context tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat for published mode and the pinned XState/Drizzle manifests.
```

The existing parse/bind harness measures the shared grammar's cost on the legacy source-file path, with
and without graph publication. It does not measure a full native parser, binder or checker. Each comparison
uses one warmup and five measured samples in alternating order, release builds, mimalloc, macOS
`proc_pid_rusage` instruction deltas and peak process RSS. Builds, Miri and conformance finished before
sampling. Checksums and diagnostic counts must agree across versions and modes.
Raw samples and medians are in [native-parser-context-results.json](native-parser-context-results.json).

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.880197B → 19.992315B | +0.564% | 1515.94 → 1515.95 MiB |
| VS Code | published | 37.212224B → 37.323530B | +0.299% | 2990.25 → 2990.27 MiB |
| XState | tls | 0.320921B → 0.322390B | +0.458% | 28.61 → 28.64 MiB |
| XState | published | 0.625012B → 0.626476B | +0.234% | 57.05 → 57.09 MiB |
| Drizzle | tls | 1.399460B → 1.410038B | +0.756% | 107.33 → 107.34 MiB |
| Drizzle | published | 2.632804B → 2.643103B | +0.391% | 219.98 → 220.00 MiB |

Sharing these routines adds about 0.23–0.76% instructions in this screening comparison; peak RSS changes
by at most 48 KiB. This is a measured cost of the requested migration. A three-run follow-up on XState
and Drizzle restored the old inherent legacy lookahead boundary and forwarded the trait adapter to it.
That isolated probe did not remove the increase, so the alternate boundary was reverted. It was motivated
by this stage's new shared speculation call path; the prior broad name-parser inlining experiments were
not repeated. The retained implementation has no new inlining directives beyond the existing boundaries.

The stage remains on the isolated migration branch. It does not clear the project performance gate or
justify switching the default frontend; complete native source parsing/binding and its retention costs
still need to be measured before that decision.

The next stage, [native list parsing and recovery](native-parser-lists.md), shares list construction,
enclosing-context recovery and JSDoc declaration propagation with construction-scoped references.
