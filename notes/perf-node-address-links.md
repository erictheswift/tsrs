# Symbol links after removing node IDs

PR #555 replaces assigned node IDs with allocation addresses. Its first version replaced the dense
`symbol_node_links` store with the generic hashed `LinkStore`. The node header remained 24 bytes, and
CI instructions increased on all 17 projects: Compiler +1.308%, vscode +0.943%, median +0.433%.
Restoring a private atomic index was explicitly rejected: the node must have no assigned index.

The fix changes only `NodeLinkStore`. Divide the allocation address by `size_of::<Node>()` to obtain
a slot. Two live nodes cannot share a slot because their headers alone occupy that many nonoverlapping
bytes. Compressed handles use the equivalent division in eight-byte units. A two-level sparse table
allocates groups of eight values on demand; each second-level page holds 128 group pointers. Plain
pointers use a hash table of pages to avoid a directory spanning the virtual address space.

Values remain in fixed arena allocations. Existing callers can keep their references across recursive
checks, so no checker algorithms or cache APIs change. Forks copy entire groups from their frozen
parent, preserving untouched values and explicit empty overrides. The node header stays 24 bytes.

This revisits address-indexed storage under a changed constraint: dense IDs are no longer available.
The rejected handle arrays in `mem-dense-link-tables.md` concerned sparse signature links. Here the
values are four-byte symbol handles, address units are a whole node header, and groups have eight
slots. Other link stores are unchanged.

## Measurements

Apple M5 Max, macOS, release builds, `/usr/bin/time -l`. Public TypeScript benchmark suite pinned to
`41f652ab2df5077b1115f73eaeefc2fe9f674132`. Commands use `--noEmit --incremental false --pretty false`;
single mode also uses `--singleThreaded` and `RAYON_NUM_THREADS=1`. Interleaved runs, one warmup per
binary discarded, medians of ten single runs and six default runs. Output and exit status identical.

The controlled comparison keeps the original PR's source baseline: dense = `d4f5b06f`, hash =
`dcb3d5ac`, fixed = `dcb3d5ac` with the final `links.rs` change. This isolates storage from the parser
inlining changes subsequently brought in by rebasing onto `6dd3374b`. These are local hardware
counter measurements, not Linux `bench/count.py` results or a wall-time claim.

| Project | Mode | Dense instructions (G) | Hash instructions (G) | Fixed instructions (G) | Dense peak RSS (MiB) | Hash peak RSS (MiB) | Fixed peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Compiler | single | 1.75216 | 1.77696 | 1.74988 | 57.25 | 61.81 | 59.10 |
| Compiler-Unions | single | 3.24300 | 3.26667 | 3.24091 | 59.91 | 64.05 | 61.62 |
| Compiler | default | 2.06102 | 2.08706 | 2.05768 | 78.78 | 83.24 | 81.18 |
| Compiler-Unions | default | 3.91792 | 3.93979 | 3.91359 | 83.56 | 87.48 | 85.64 |

Against the PR's hash store: single instructions -1.52% / -0.79%, single RSS -4.39% / -3.78%, default
RSS -2.48% / -2.10%. The instruction regression is recovered locally; memory still exceeds the original
dense store by 2.5–3.2%. No overall speedup or qualifying memory gain over the original dense store is claimed.

Earlier candidates, measured on the same public projects:

- Inline hash buckets holding both node and symbol removed the separate arena values but retained most
  of the instruction regression (Compiler +1.03% versus dense).
- Custom linear probing remained +1.00% versus dense and did not justify another hash implementation.
- Address bitmaps with packed values saved memory but still cost +0.91% instructions versus dense.
- Fixed groups of 32 recovered instructions; groups of eight reduce unused slots with a small instruction
  tradeoff. Compiler's store census falls from about 2.72 MiB to 2.47 MiB, including its arena allocations.

## Validation

Conformance pass sets are unchanged: 13,458 errors, 12,779 types, 12,779 symbols. Default canonical
threading retains only the existing `objectLiteralNormalization` type/symbol mismatch. Emit pass sets
are unchanged: 13,462 errors, 13,392 JS/declaration, 149 JS maps, 156 source-map records.

AST/checker/API/codec and core checks pass, as do the 36 regression projects, arena recycling and LSP
lifetime checks, shared-parent isolation, plain pointers, sizing/shared-graph feature compilation, WASM
compilation, workspace Clippy, and source lint. The link-store checks retain references across growth
and exercise copying, empty overrides and nested forks.
