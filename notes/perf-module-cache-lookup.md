# Borrowed module-cache lookup (2026-10-18)

Used `xcrun xctrace` Time Profiler following [the owner's performance guide](https://github.com/Boshen/rust-performance-improvement-plan/blob/main/README.md).

## Candidate and attribution

The Time Profiler recordings of webpack and mui-docs exposed a different hot path. On mui-docs,
`Program::get_resolved_module` accounted for 1.84% of self CPU sample weight. The Rust implementation
linearly searched the per-file resolution hash map because its keys hold `&'static str`, while callers
can supply a borrowed name. The pinned Go implementation already uses a direct lookup by name and mode
(`program.go:631`, commit `b85298b6a81f772d080b0455de0ca9d744cd6fd6`).

`ModeAwareCache` now uses the workspace's existing hashbrown dependency with the same Fx hasher.
An `Equivalent<ModeAwareCacheKey>` implementation lets a borrowed `(name, mode)` tuple probe the map.
This removes the scan without allocating a string, adding a cache, changing resolution, or changing
the stored fields. The trait signatures use the existing `ModeAwareCache` alias. The language server's
actual alias-resolution cache is unchanged; only its empty diagnostic-cache field needs the new alias.

The key's derived hash and the tuple's hash process the same fields in the same order. A unit check
covers matching hashes and lookups with borrowed strings, empty and Unicode names, separate modes,
missing names and missing modes. Existing map iterations either build sets/maps or record symlinks;
lookup equality is still name plus mode.

The verification Time Profiler recording reduces module lookup from 1.84% to 0.016% of self CPU sample
weight (57.4 → 0.5 ms of sampled weight). Profile percentages are attribution, not a speed measurement.

## Measurement method and status

Baseline: `41320338` plus the pre-existing workspace changes. Those changes were preserved byte-for-byte.
Rust 1.99.1 (`58cf8f9c3`, 2026-10-13), macOS 27.0.1 (26A434), arm64.
Both binaries use the release profile, with symbols retained by
`cargo rustc --release -p tsrs_cli -- -C strip=none`.

- Baseline SHA-256: `db7a203ed3092c57fbbdd92f8abaea1f8a0e2da51ee25dbd236fe72943620ef3`.
- Candidate SHA-256: `1f1d4c5e4fef467ecf421ffef53c3347b55f9b4f0444ba6f9af88b5ada6d4b9c`.

Inputs are the public projects in `bench/projects.json`; the prepared repositories' HEADs match the
manifest. Compiler and Compiler-Unions use the pinned benchmarking suite. Runs use `--noEmit
--incremental false --pretty false`; single-threaded runs add `--singleThreaded` and
`RAYON_NUM_THREADS=1`. Default runs omit both thread overrides. One before/after warm-up pair precedes
alternating measured pairs. Every measured run's exit code and ordered diagnostic-byte hash is checked.

`/usr/bin/time -l` supplies instructions, peak RSS and wall time. Its macOS instruction count includes
kernel work. These measurements screen the candidate; they do **not** satisfy the deterministic Linux
`bench/count.py` landing gate. No memory improvement or default-mode wall-time improvement is claimed.
The candidate is a draft pending Linux verification; do not land it until a required
performance threshold is confirmed. Linux verification was not run in this local macOS session.

## Measurements

Medians; instructions in billions and peak RSS in MiB. Each arrow is baseline → candidate. The initial
batch used five pairs for mui-docs, webpack and the Compiler fixtures, and three pairs for every other
project. The table substitutes the seven-pair MUI single-threaded repeat and the seven-pair default-mode
repeats for webpack, formbricks-web, vscode and playwright. All initial and repeated samples are retained
in [the raw measurements](perf-module-cache-lookup-results.json).

| Project | Single instructions (G) | Change | Single RSS (MiB) | Default RSS (MiB) |
| --- | ---: | ---: | ---: | ---: |
| vscode | 79.566502 → 79.448155 | -0.149% | 1603.19 → 1603.03 | 2112.42 → 2114.36 |
| xstate-main | 6.251038 → 6.244591 | -0.103% | 192.06 → 191.89 | 319.80 → 319.64 |
| webpack | 13.364861 → 13.368614 | +0.028% | 433.89 → 433.70 | 632.27 → 634.23 |
| mui-docs | 36.879442 → 35.932978 | -2.566% | 729.36 → 729.23 | 1577.38 → 1576.78 |
| Compiler | 1.719333 → 1.719414 | +0.005% | 57.33 → 57.25 | 78.41 → 78.75 |
| Compiler-Unions | 3.213453 → 3.213532 | +0.002% | 60.02 → 59.95 | 83.83 → 83.47 |
| cal-diy | 32.509181 → 32.477586 | -0.097% | 815.08 → 814.88 | 2015.86 → 2024.45 |
| formbricks-web | 39.309965 → 39.279808 | -0.077% | 1114.02 → 1113.83 | 1956.17 → 1962.16 |
| supabase-studio | 42.696479 → 42.652188 | -0.104% | 869.19 → 868.98 | 1650.28 → 1656.45 |
| t3code-server | 37.955104 → 37.943589 | -0.030% | 730.02 → 729.77 | 2109.08 → 2078.75 |
| mikro-orm | 52.504407 → 52.497019 | -0.014% | 1120.31 → 1120.14 | 1994.12 → 1964.02 |
| next-packages-next | 16.371946 → 16.360452 | -0.070% | 443.27 → 443.06 | 664.41 → 659.38 |
| next-root | 15.725327 → 15.699266 | -0.166% | 313.64 → 313.42 | 482.75 → 483.12 |
| storybook | 10.199481 → 10.188163 | -0.111% | 306.30 → 306.09 | 536.88 → 533.70 |
| nuxt | 11.149343 → 11.132953 | -0.147% | 302.11 → 301.91 | 527.11 → 528.67 |
| playwright | 7.514540 → 7.509562 | -0.066% | 251.78 → 251.58 | 464.97 → 457.58 |
| drizzle-orm | 12.155158 → 12.144658 | -0.086% | 390.53 → 390.31 | 783.11 → 753.47 |

MUI single-threaded instructions fall 2.461% in the initial batch and 2.566% in the separate repeat.
The initial candidate batch includes one high sample (37.352 G followed by 35.933–35.991 G); the baseline
stays at 36.846–36.888 G. The independent repeat confirms the reduction. MUI single-threaded median wall
time is 2.80 → 2.73 s initially and 2.85 → 2.79 s in the repeat. These are local measurements, not two
benchmark publishes; they do not establish the repository’s wall-time acceptance gate.

Default-mode work stealing varies total work and residency. The initial webpack +1.327% instruction
change becomes +0.231% in the repeat; formbricks-web +0.769% becomes +0.006%, with its median wall time
equal at 0.78 s. Vscode’s +1.472% RSS change becomes +0.092%. Playwright’s +1.878% instructions and
+2.099% RSS become -1.354% and -1.589%. Do not attribute these default-mode fluctuations to the patch.

All 306 measured runs preserve ordered diagnostic bytes and exit codes across all 34 project/mode cells.
No measured run crashed. Only normal success (0) and diagnostic (2) exits occurred.

## Correctness

These checks were run in the original measurement workspace described above.

- Built with `cargo build --profile dev -p tsrs_cli -p tsrs_testrunner`.
- All 18 conformance result lists match the baseline with `--suite all --baselines types,symbols
  --timeout 120`: 13,458 diagnostic passes and 12,779 type/symbol passes, existing non-passing cases unchanged.
- All canonical-history result lists also match with `TS_TEST_PROGRAM_SINGLE_THREADED=false
  TSRS_HISTORY=canonical`: the existing 12,778 type/symbol pass counts are unchanged.
- `cargo test -p tsrs_module`: 28 passed, one existing ignored check.
- `tools/regressions.sh target/debug/tsrs`: 36 passed.
- `cargo clippy --all-targets --config 'build.warnings="deny"'` and `tools/lint/source.py` pass.

For the draft PR, the code patch was applied to main `a553e0a9`. The dev build, module checks,
36 regressions, Clippy and source checks pass again on that branch. Both conformance histories'
18 result lists still match the recorded baselines. Performance measurements above remain from the
original workspace; the Linux verification must measure the rebased branch before landing.

## Reproduction artifacts

`/tmp/tsrs-xctrace-20261018/` contains `before-symbols`, `module-cache`, `module-cache.patch`,
the three new Time Profiler recordings (`webpack-before`, `mui-before`, `mui-module-cache`), exported
XML and summaries, paired measurement scripts, raw JSON and validation logs. Time Profiler uses
100 µs sampling (`highFrequencySampling: true`), with kernel stacks, waiting threads and context-switch
sampling disabled. Its output is valid even when a benchmark's diagnostic exit makes `xctrace record`
return 54.

For a new paired MUI comparison using the saved binaries:

```sh
python3 /tmp/tsrs-xctrace-20261018/measure-all.py \
  --before /tmp/tsrs-xctrace-20261018/before-symbols \
  --after /tmp/tsrs-xctrace-20261018/module-cache \
  --projects mui-docs --modes single,default --reps 7 \
  --output /tmp/tsrs-xctrace-20261018/module-cache-review.json
```

The counter and profiler need unsandboxed access to macOS profiling services. For Linux acceptance,
build the same baseline and candidate with `--release`, then run `bench/count.py` on the pinned MUI
fixture with `RAYON_NUM_THREADS=1 --singleThreaded`, comparing diagnostic bytes as well as instructions
and RSS. Reject the patch if it fails to reach a repository landing threshold there.
