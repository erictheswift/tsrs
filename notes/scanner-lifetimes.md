# Scanner source lifetimes

The next step after the [borrowed front-end prototype](file-arena-lifetimes.md), on
`codex/file-arena-lifetimes`. The prototype is committed at `74f64f2f`.

Direct native AST construction needs a scanner that can borrow file-owned source. `Scanner` previously
required `&'static str` even for callers that finished scanning before dropping an ordinary local string.
This change introduces `Scanner<'text>`, `ScannerState<'text>`, and `CommentRangeIter<'text>` and carries
that lifetime through token values, numeric caches, regexp parsing and named capture references. The byte
dispatch table becomes an associated constant so its function pointers use the scanner's source lifetime;
the scanning algorithms and table entries are unchanged.

The scanner oracle now drops each input string after its four passes, and BigInt string validation borrows
its temporary input instead of copying it into the arena. Existing parser, checker, and formatting fields
explicitly use `Scanner<'static>` because their legacy AST still exposes static source text. This is an
incremental ownership refactor under the requested lifetime migration; it does not adopt a new production
allocation strategy or claim a performance win.

The boundary is deliberate: computed token values and comment-directive snapshots still allocate in the
current legacy arena/region. Their allocation scope must stay alive under the existing contract. The scanner
also retains the legacy representation of cooked lone surrogates. No unsafe lifetime extension or additional
unsafe block was introduced. Native allocation of cooked token bytes, a native node factory and parser/binder
construction remain the next steps; the prototype's graph copy and `P<Node>` have not been removed.

## Validation

```sh
cargo test -p tsrs_scanner -p tsrs_frontend_arena --locked
cargo +nightly-2026-10-04 miri test -p tsrs_scanner --features tsrs_core/plain-ptrs --lib
MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly-2026-10-04 miri test \
  -p tsrs_scanner --features tsrs_core/plain-ptrs --lib borrowed_values_and_snapshots_survive_the_scanner
cargo build --profile dev -p tsrs_cli -p tsrs_testrunner
target/debug/tsrs-test run --suite all \
  --filter 'parser|binder|jsdoc|controlFlow|regularExpression|bigint|BigInt|comment' --jobs 4
cargo clippy --workspace --locked --all-targets --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py
git diff --check
```

All pass. Nine scanner unit cases and four doctests cover borrowing local strings, retaining values and
snapshots after dropping a scanner, numeric normalization, regexp errors across rewind, comments, and
compile-time rejection of tokens/snapshots/comment iterators escaping their source. The existing front-end
runtime and compile-fail cases also pass. Miri passes all nine scanner unit cases; the AST utility case still
warns about legacy exposed-provenance pointer conversion. The focused borrowed scanner case passes strict
provenance. Conformance has the identical set of 1,190 passing cases before and after, with 125 skips and
no failures or crashes.

## Measurement

The existing release `measure` executable was saved at `74f64f2f`, then rebuilt with this change:

```sh
cargo build --release -p tsrs_frontend_arena --example measure
cp target/release/examples/measure /tmp/measure-before
# Apply the scanner lifetime commit, then rebuild the same target.
cargo build --release -p tsrs_frontend_arena --example measure
/tmp/measure-before tls /path/to/vscode.txt
target/release/examples/measure tls /path/to/vscode.txt
```

Repeat for `published` and for the XState and Drizzle manifests from the prototype report. Each comparison
uses one warmup and five measured samples per binary/mode, alternating binary order. The process runs one
front-end thread. Source loading precedes the instruction/timing interval; peak RSS includes loaded sources.
This measures source-corpus parse/bind/publication, not project resolution or checking at the default checker
count. All versions/modes agree on syntax checksums and diagnostic counts. Raw samples and medians are in
[scanner-lifetimes-results.json](scanner-lifetimes-results.json).

| Corpus | Mode | Instructions before/after (G) | Change | Peak RSS before/after (MiB) |
|---|---|---:|---:|---:|
| vscode | tls | 19.866595 / 19.860859 | -0.029% | 1515.89 / 1515.89 |
| vscode | published | 37.327216 / 37.329410 | +0.006% | 2990.20 / 2990.20 |
| xstate | tls | 0.319741 / 0.319775 | +0.011% | 28.56 / 28.56 |
| xstate | published | 0.624113 / 0.624170 | +0.009% | 57.00 / 57.00 |
| drizzle | tls | 1.394761 / 1.394837 | +0.005% | 107.28 / 107.28 |
| drizzle | published | 2.635738 / 2.631783 | -0.150% | 219.92 / 219.91 |

No material front-end instruction or memory change was measured. This confirms that introducing source
lifetimes does not reproduce the cost of the prototype graph copy. The original publication overhead remains.

The next step, [per-file scanner storage](scanner-file-arena.md), supplies explicit native allocation and
byte-valued cooked tokens for this borrowed scanner.
