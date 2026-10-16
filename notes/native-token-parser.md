# Native token and literal parsing

Continuation of [native name parsing](native-name-parser.md), starting at `535bec98` on
`codex/file-arena-lifetimes`. The scoped parser now constructs punctuation/missing tokens, keyword and
literal expressions, literal types (including negative numbers/BigInts), and template heads/middles/tails
in the final file arena. These nodes and their text carry real lifetimes:

```rust
use tsrs_frontend_arena::{FileArena, ScriptKind, TokenParser};
let arena = FileArena::new();
let root = arena.build(|factory| {
    let mut parser = factory.parser("-1n", ScriptKind::TS);
    parser.next_token();
    parser.parse_literal_type_node(true)
});
assert_eq!(root.kind, tsrs_ast::Kind::LiteralType);
```

`NodeFactory::parser` consumes the scoped factory. The parser, its mutable construction references and
scanner state cannot escape `FileArena::build`/`try_build`; a new compile-fail example checks the parser
itself. Publication still freezes the same storage. Source is copied once into the file arena, cooked
literal values are borrowed directly from the scanner, and raw template text borrows that source copy.
A caller can discard its input immediately after creating the parser. `into_diagnostics` returns owned
errors that remain usable after both the parser and arena are dropped.

## Shared grammar

`tsrs_parser::token_parser::TokenParser` holds the existing token/literal routines. Production parser
methods forward to the trait, which preserves their branch ordering, scanner advances, recovery ranges,
token flags, keyword-escape checks and diagnostic calls. The template callers retain responsibility for
whether rescanning is tagged; invalid untagged heads and rescanned tails report scanner errors in order.
`NameParser` inherits the same primitive parser operations, eliminating duplicate token/recovery helpers.

Adapters supply scanner storage, constructors, diagnostic storage and node completion. The native adapter
uses construction references, byte-based `Text`, script-kind context flags and arena-local scanner data;
the existing adapter retains its legacy factory and diagnostics. The hot production token-advancing
methods still call `scan_legacy`. No scanner/inlining experiment from the prior reports is repeated.

The public scoped parser exposes only migrated routines. It is not a complete expression, type or
source-file parser. The fixtures invoke these routines at the same token positions as production callers;
they do not introduce a second partial TypeScript grammar. The production full-source parser exercises
the shared routines through its existing call sites, including JSDoc punctuation handling.

## Scope and prior evidence

The lifetime/scanner/factory/name notes, the rejected list in `perf-round2-followups.md`, the techniques in
`docs/RUST.md`, and `mem-round4.md` were reviewed before this stage. The changed constraint remains the
owner's explicit lifetime migration. Native references and the minimum file-arena chunk still have the
costs recorded in those reports; this stage makes no claim to reverse them.

`parse_and_bind` still parses and binds a legacy graph, then copies it. Full expression/type/statement
parsing, parser lists/speculation/source-file metadata/JSDoc state, binding in the construction scope,
and production callers/checker lifetimes remain to migrate. `P<Node>` is not yet removed from those
paths. The CLI and default frontend behavior are unchanged. This remains an isolated migration branch;
these measurements do not qualify a production switch or clear the project performance gate.

## Validation

```sh
cargo test -p tsrs_frontend_arena --locked
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test tokens native::
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
cargo build --profile dev -p tsrs_parser --example ast_oracle
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

All enabled arena runtime cases, 23 compile-fail doctests and three positive doctests pass. Eight new
runtime cases cover:

- Literal/keyword metadata, cooked values and ordered diagnostics against the real source-file parser
  in TS, JS, TSX and JSX contexts, including malformed literals and regex rescanning.
- Literal types and negative literal parent links, plus tagged/untagged template parts with raw text,
  invalid escapes, CRLF, Unicode, unterminated tails and missing closing braces.
- Missing punctuation positions and error flags, diagnostic suppression and JSDoc token helpers.
- Zero legacy-region allocation, cooked/raw pointer identity, lone UTF-16 surrogates, input/parser/region
  release, arena growth while published nodes have concurrent readers, and detached error ownership.

Strict-provenance Miri passes all five native cases. The three differential cases use legacy payload
helpers and run natively, for the provenance limitation documented in the preceding name-parser report.
No unsafe code or generated record layouts change in this stage.

The saved pre-refactor AST oracle and the rebuilt oracle produce identical hashes for all 15,537 public
source files (VS Code, XState and Drizzle; pinned revisions/manifests in `file-arena-lifetimes.md`). The
selected conformance pass list is also identical: 1,497 passes, 203 skips, zero failures/crashes.
Workspace clippy, source lint and the diff whitespace check pass.

## Measurements

```sh
# Save the release measure executable built at 535bec98 before building this revision.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-tokens tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
# Repeat for published mode and the pinned XState/Drizzle manifests.
```

The existing parse/bind harness measures the cost of sharing token/literal grammar on the production
path, with and without graph publication. It does not yet measure a complete native source-file parser.
Each comparison uses one warmup and five measured samples in alternating order, with release builds,
mimalloc, macOS `proc_pid_rusage` instruction deltas and process peak RSS. Syntax checksums and diagnostic
counts must agree across both versions and modes. Builds, conformance and Miri finished before sampling.
Raw samples and medians are in [native-token-parser-results.json](native-token-parser-results.json).

Existing parse/bind harness, before → after this stage:

| Corpus | Mode | Instructions | Change | Peak RSS |
| --- | --- | ---: | ---: | ---: |
| VS Code | tls | 19.927436B → 19.923523B | -0.020% | 1515.95 → 1515.94 MiB |
| VS Code | published | 37.375728B → 37.373875B | -0.005% | 2990.27 → 2990.25 MiB |
| XState | tls | 0.321007B → 0.320902B | -0.033% | 28.62 → 28.61 MiB |
| XState | published | 0.625168B → 0.625037B | -0.021% | 57.08 → 57.05 MiB |
| Drizzle | tls | 1.399838B → 1.399222B | -0.044% | 107.33 → 107.33 MiB |
| Drizzle | published | 2.632911B → 2.632163B | -0.028% | 219.98 → 219.98 MiB |

Instruction differences are below 0.05%; peak RSS differs by at most 32 KiB. These are effectively
unchanged results on this macOS screening harness, not evidence of a qualifying performance gain.
