# Per-file scanner storage

Continuation of [scanner source lifetimes](scanner-lifetimes.md), starting at `d16748ed` on
`codex/file-arena-lifetimes`. This implements direct native allocation for the lexical part of the
requested front-end lifetime migration.

`Scanner<'text, S>` shares one scanning algorithm across two allocation policies. Its default `LegacyStorage`
keeps the current parser's allocation contract. `FileArena::scanner(source)` copies source into its owner and
uses `&FileArena` for cooked token buffers, normalized numbers and immutable comment-directive snapshots.
Numeric cache entries borrow that storage. The native policy does not access the thread-local arena, register
global source text, or need to pin speculative allocations: `FileArena` cannot rewind while references exist.

Native token values are `&'ast [u8]`. Escaped strings and regexp atoms represent lone UTF-16 surrogates as code
units/bytes, with no invalid Rust string in the native path. UTF-8 conversion for identifiers and numeric
spellings is checked. The shared escape representation stores a decoded code point until a token buffer encodes it;
regexp range comparisons use the code point directly. The legacy literal buffer still uses the existing CESU-8-in-string
compatibility code. Removing that representation from the legacy AST and checker remains separate work.

The new token buffer owns a fresh range in `FileArena` until `finish` consumes the buffer and publishes a
borrowed immutable slice. Growth reserves another disjoint range. The source, cooked values, snapshots and
comment directives can outlive the scanner but cannot outlive their arena. There are no unsafe `Send`/`Sync`
implementations or lifetime extensions in this change.

The main parse/bind API still constructs its legacy AST and publishes a copied graph. The native scanner is
now usable by a native node factory/parser without TLS or static lexical storage; those AST construction
steps and removal of `P<Node>` are still pending. Production allocation defaults have not switched.

## Prior evidence and scope

The prototype report already records why all-file arena allocation is not a production performance choice:
previous per-file-region work cost 6–9%, and copying a complete graph nearly doubled front-end work. The
changed constraint is the requested Rust-enforced ownership migration. This step allocates final native
lexical storage directly and measures its costs; it does not retry region reclamation, add caching or claim
an end-to-end checker speed/memory improvement. The existing 64 KiB minimum native chunk is unchanged.

## Legacy parser compilation boundary

The parser calls the specialized `Scanner<LegacyStorage>::scan_legacy` entry point, which forwards to the
shared loop in the scanner crate. Generic callers use `scan` directly. This retains the old cross-crate
compilation boundary until the parser itself becomes generic/native; it adds no storage or runtime dispatch.

Calling generic `scan` directly from the legacy parser initially increased parse/bind instructions by about
2%. Release assembly showed that the scanner loop expanded before the compiler decided whether to inline
`Parser::next_token`; the larger token-advance function then stayed out of line in parser callers. A file of
30,000 semicolons reproduced the cost, ruling out allocation and escape decoding. Scanner/state layouts and
the semicolon handler were unchanged. `#[inline]` and forced inlining on `next_token` failed to recover the
original behavior; neither annotation is retained. The non-generic entry restores the original boundary
without forced inlining. The final measurements below include that entry point.

## Validation

```sh
cargo test -p tsrs_scanner -p tsrs_frontend_arena --locked
TSRS_LIFETIME_MANIFEST=/path/to/all.txt cargo test -p tsrs_frontend_arena \
  --test scanner public_corpus_scanner_storage_parity -- --ignored
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_frontend_arena --features plain-ptrs --test scanner \
  -- --skip native_and_legacy_tokens_errors_and_directives_match
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment|unicode|Unicode|scanner|template' \
  --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

Native cases cover all 2,048 lone surrogate code units, buffer growth while earlier values remain live,
snapshots and normalized-number caches after releasing a legacy region, reset, scoped concurrent readers,
JSX/JSDoc entry points and regexp rescans. The region's used-byte counter stays unchanged during native
scanning. Three new compile-fail examples reject token bytes, snapshots and directives escaping the arena.
Strict-provenance Miri passes these native ownership and rescan cases without the legacy pointer model.

The four-mode comparison (ordinary scan, trivia, regexp/template/greater-than rescans, and rescans targeting
ES5) passes on all 15,537 files in the pinned public corpora from the prototype report. It compares cooked
bytes, token kinds/ranges/flags, diagnostic codes/arguments/order and directive spans/order between policies.
The broader conformance selection has the same 1,497 passing cases before and after, 203 skips and no
failures/crashes. The scanner and published-front-end unit/runtime/doctest checks also pass.

## Measurement

Build the release example at `d16748ed` and save it, then rebuild with this change:

```sh
cargo build --release -p tsrs_frontend_arena --example measure
cp target/release/examples/measure /tmp/measure-before-storage
# Apply this commit and rebuild the same example.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before-storage tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
target/release/examples/measure scan-tls /path/to/vscode.txt
target/release/examples/measure scan-native /path/to/vscode.txt
```

Repeat on the XState and Drizzle manifests and compare `published` before/after as well. Five measured samples
follow one warmup, reversing case order each round. The counter is macOS `proc_pid_rusage` v4 instructions;
peak RSS includes preloaded source strings. The parse/bind cases include syntax walks as before. The two
scanner modes perform ordinary scans, hash token bytes/ranges/flags, collect diagnostic counts and retain all
lexical storage; both copy input into their allocation target. The scanner-only modes do not parse or bind.
Matching syntax/scanner checksums and diagnostic counts are asserted within each comparison group.

These are single-threaded source-corpus measurements, not resolved projects or default-checker-count memory
measurements. Raw samples and medians are in [scanner-file-arena-results.json](scanner-file-arena-results.json).

| Corpus | Parse/bind mode | Instructions before/after (G) | Change | Peak RSS before/after (MiB) |
|---|---|---:|---:|---:|
| vscode | tls | 19.853532 / 19.894669 | +0.207% | 1515.89 / 1515.92 |
| vscode | published | 37.319020 / 37.322254 | +0.009% | 2990.20 / 2990.23 |
| xstate | tls | 0.319815 / 0.320339 | +0.164% | 28.56 / 28.61 |
| xstate | published | 0.624145 / 0.624070 | -0.012% | 57.00 / 57.06 |
| drizzle | tls | 1.394776 / 1.396770 | +0.143% | 107.28 / 107.33 |
| drizzle | published | 2.629041 / 2.628582 | -0.017% | 219.91 / 219.98 |

The legacy parse/bind path is within +0.21% instructions of the previous commit, with at most 0.08 MiB extra
peak RSS. Publication instructions remain within 0.02%. These are ownership changes, not a performance win.

| Corpus | Scanner instructions legacy/native (G) | Change | Peak RSS legacy/native (MiB) |
|---|---:|---:|---:|
| vscode | 3.860412 / 4.014993 | +4.00% | 407.00 / 559.80 |
| xstate | 0.070179 / 0.073386 | +4.57% | 8.30 / 14.08 |
| drizzle | 0.277279 / 0.287460 | +3.67% | 25.09 / 37.02 |

Direct native scanning costs 3.7–4.6% more instructions in this harness. Its retained peak RSS is 38–70% higher;
the native arena still reserves chunks of at least 64 KiB per file. Neither result justifies a production
allocator switch under the performance gate. The native path is an explicitly owned building block for the
requested lifetime migration. It avoids the prototype's graph-copy approach for lexical data, but the main
parse/bind publication still copies its legacy graph and retains the earlier measured overhead.
