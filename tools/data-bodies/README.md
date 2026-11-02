# Data-driven body acceptance

This is a **standalone, non-production** acceptance host. It does not modify either consumer, templates, locks, `site.data`, or page enumeration. It reads a pinned XIV CSV and all Cargo-declared ranim examples into an immutable fixture. It does not invoke consumer `prepare`, render media, access credentials, or publish anything.

```sh
python3 tools/data-bodies/prepare.py \
  --xiv /path/to/icexiv-db --ranim /path/to/ranim \
  --out /tmp/opencode/data-bodies-fixture

CARGO_TARGET_DIR=/tmp/opencode/notc-data-bodies-target \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 \
cargo run --example data-bodies-acceptance -- \
  /tmp/opencode/data-bodies-fixture /tmp/opencode/data-bodies-output
```

Use new output directories. The snapshot verifies the CSV's manifest SHA-256 and rereads each source file to reject changes during capture. Only the fixture's immutable, explicitly allowed bytes are visible to evaluation. Relative paths are rooted at `/validation`; every language read still passes through `EvaluationInputs.file()` before the shared parse cache.

`view.notc` renders:

- Every XIV detail: all 13 fields, with exact integers and explicit Chinese boolean labels.
- Every catalog page: 100 records except the final partial page, with stable row order and links.
- Every Cargo-declared ranim example, including hidden examples for coverage: both original `scenes.rs` and `main.rs` as raw Rust, ordered preview links, MIME-labelled output links, and the WASM availability flag. Media URLs come from shadow refs, not filename guesses. This verifies source/preview bodies, **not the complete player, original assembled Markdown field, or consumer website**.

All detail pages share **one** by-ID JSON store; catalogs use the existing consumer's four-field, 100-record projections. The full run checks every field and every row against the source snapshot, writes every HTML body, and reports parsing/projection counts, checked instances, time, peak RSS, logical size, cache residency, and output counts. It does not reduce the dataset or move body generation to the client.

The default data limits intentionally reject the full store. This host explicitly opts into 1,000,000 nodes, 32 MiB logical values, a 1 GiB conservative parser working-set/cache bound, and 100,000 batch read requests; source size remains 16 MiB and cache entries remain 4096. Every read is charged, including a parse-cache hit. The JSON parser's conservative token estimate also rejects a 512 MiB working-set bound for this store. These are **host choices**, not new CLI defaults. Cache accounting, actual RSS, and logical bytes are different quantities.

## Recorded full debug run

Pinned CSV `abb68a057f68e4d9d37eae76d31354555d30f76a88554d0d1c04315598c7268f`:

- 51,225 details, 513 catalogs, 34 ranim bodies; 51,772 HTML files, 34,245,320 bytes.
- 548 parses (one per immutable data file), 615 materialized projections, 1163 cache entries; cheap per-record scalar decoding creates no memo entries.
- Maximum 21 checked function instances per page; 12 mounted content specializations across the batch.
- Debug build: 127.28 s body pipeline, peak RSS 447,580 KiB; conservative cache accounting 379,537,960 bytes. This includes a separately held JSON reference for exhaustive comparisons and is not a production-site benchmark.
- Release build: 53.00 s, peak RSS 444,840 KiB; analysis 11.57 s, evaluation 38.10 s (including 0.175 s cold parsing and 0.003 s materialized projection), rendering 0.64 s, writing 0.70 s. Three real-data cases additionally compare Notist/Markdown output byte-for-byte. Full store: 13,408,102 source bytes, 20,039,549 logical bytes, 717,151 nodes. RSS and the conservative cache figure are not interchangeable.

The host writes `report.json` alongside the bodies. Timings depend on build profile and machine load; rerun rather than treating these as a performance promise.
