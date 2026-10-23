# References — Foundational Papers

The NucleOS codec layer implements algorithms described in three landmark papers on DNA data storage. This document maps each paper to its corresponding implementation in the codebase.

---

## 1. Goldman et al. (2013)

**"Towards practical, high-capacity, low-maintenance information storage in synthesized DNA"**
*Nature*, 494(7435), 77–80. doi:[10.1038/nature11875](https://doi.org/10.1038/nature11875)

### Key idea

Encode binary data via a **ternary rotating cipher**: convert bytes to base-3 (trits), then map each trit to a nucleotide using a rotating lookup table that changes based on the previous nucleotide emitted. This guarantees **zero homopolymer runs by construction** — the same nucleotide can never appear twice in a row — without any post-hoc screening.

The paper also introduced **overlapping segments** with 75-nt windows sharing a 25-nt overlap, providing built-in redundancy so that a few lost strands don't lose data.

### Mapping to NucleOS

| Paper concept | Implementation |
|---|---|
| Ternary encoding | `nucle_codec::ternary::TernaryCodec` — the `ROTATION_TABLE` directly implements the 4×3 rotating mapping |
| Homopolymer-free guarantee | Enforced structurally by the rotation rule; validated by `nucle_codec::constraints::ConstraintValidator` |
| Overlapping windows | `TernaryConfig { overlap: true, segment_size: 75, overlap_size: 25 }` — toggled via `TernaryConfig::with_overlap()` |
| GC content targeting | `ConstraintConfig` with `gc_min: 0.40, gc_max: 0.60` — strands outside range are flagged |
| Information density | **1.58 bits/nt** theoretical; the current built-in 89-byte `nucle bench` payload measures **1.099 bits/nt** without overlap and **0.628 bits/nt** with overlap |

### Density gap: 1.099 vs 1.58 bits/nt

The theoretical limit of the ternary rotating cipher is log₂(3) ≈ 1.58
bits/nt — each nucleotide encodes one trit. On the current 89-byte built-in
benchmark payload, NucleOS measures **1.099 bits/nt** with overlap disabled;
the default overlap configuration measures **0.628 bits/nt** because it emits
additional overlapping segments. The non-overlap gap comes from three
sources:

1. **Segment framing** — each strand carries a segment index header (4 bytes = 16 nt) that doesn't encode user data but is required for reassembly.
2. **Length prefix** — a 4-byte big-endian length header is prepended to the payload so the decoder knows the original file size.
3. **Padding** — the last segment is zero-padded to the fixed segment size, wasting nucleotides on small inputs.

These overheads are proportionally smaller on larger files. Goldman et al.
reported lower effective density than the theoretical limit once indexing and
overlap redundancy were included; NucleOS therefore reports the measured
effective figure and labels which overlap configuration produced it.

This gap is **not a bug** — it's the real engineering cost of making DNA storage work. Any production codec needs headers for reassembly, and reporting effective density (not theoretical) is the honest metric.

### Source files

- [`nucle_codec/src/ternary.rs`](../nucle_codec/src/ternary.rs) — Encoder/decoder
- [`nucle_codec/src/constraints.rs`](../nucle_codec/src/constraints.rs) — Biological constraint validation
- [`nucle_codec/src/benchmark.rs`](../nucle_codec/src/benchmark.rs) — Codec comparison framework

---

## 2. Erlich & Zielinski (2017)

**"DNA Fountain enables a robust and efficient storage architecture"**
*Science*, 355(6328), 950–954. doi:[10.1126/science.aaj2038](https://doi.org/10.1126/science.aaj2038)

### Key idea

Apply **Luby Transform (LT) codes** — a class of rateless fountain codes — to DNA storage. The data is split into segments; each encoded strand is an XOR of a random subset of segments (chosen by a pseudorandom degree distribution). Because LT codes are **rateless**, you can generate an unlimited number of encoded strands, and any sufficient subset can reconstruct the original data through iterative **peeling decoding**.

Strands that violate biological constraints (GC content, homopolymers) are simply discarded and regenerated — the rateless property means you never run out of valid encodings.

### Mapping to NucleOS

| Paper concept | Implementation |
|---|---|
| Fountain / LT coding | `nucle_codec::fountain::FountainCodec` — droplet generation with Robust Soliton degree distribution |
| Decoder | `peeling_decode()` with `gaussian_decode()` fallback for full-rank sets |
| Rateless property | `encode_batch(data, start_seed, count)` returns an exact batch and continuation seed; `overhead` selects the default batch size |
| Constraint screening | Reversible seed/payload whitening, core screening, and VFS primer-tagged screening reject GC/local-GC/homopolymer/palindrome/length violations |
| Erasure resilience | `nucle_ecc::fountain_code::FountainErasure` — outer-code layer that recovers from strand dropout |
| Near-optimal density | Achieved **1.57 bits/nt** theoretical; benchmarked via `nucle bench` |

### Source files

- [`nucle_codec/src/fountain.rs`](../nucle_codec/src/fountain.rs) — Fountain encoder/decoder
- [`nucle_ecc/src/fountain_code.rs`](../nucle_ecc/src/fountain_code.rs) — Erasure-level fountain recovery
- [`nucle_ecc/src/pipeline.rs`](../nucle_ecc/src/pipeline.rs) — Multi-stage repair pipeline

### Constraint screening in practice

NucleOS's Fountain codec reversibly whitens the seed and payload before the 2-bit nucleotide mapping (`A=00, T=01, G=10, C=11`), then applies **post-hoc constraint screening**. Whitening prevents low-entropy binary input from making nearly every candidate fail while preserving exact decoding. Pool writes additionally validate the primer-tagged molecule.

With `screen_constraints: true` (the default), each generated droplet is
checked against biological constraints. Invalid strands are discarded and the
encoder tries the next PRNG seed until it fills the target or exhausts
`num_droplets * max_screening_attempts`. The current encoder then returns the
partial collection (and logs a warning) rather than raising an encode error;
downstream decode fails if too few usable droplets were produced. The rejection
rate depends strongly on the data distribution:

- **Random / binary data**: ~30-50% rejection rate, works well
- **Highly structured data** (e.g., all-zero, short ASCII): can produce heavily biased nucleotide distributions where most droplets fail GC checks, causing slow encoding or screening-budget exhaustion

This is a known limitation of the 2-bit mapping approach. A production system would use a more sophisticated byte-to-nucleotide scheme (e.g., the Yin-Yang codec from Ping et al. 2022) that provides better GC balance by construction.

---

## 3. Ping et al. (2022) — Yin-Yang Codec

> "Towards practical and robust DNA-based data archiving using the yin–yang codec system."
> *Nature Computational Science*, 2, 234–242.

### Key ideas

The Yin-Yang codec encodes **2 bits per nucleotide** using two complementary mapping rules:

1. **Yang rule** — partitions nucleotides by GC content: `bit=0 → {A, T}`, `bit=1 → {C, G}`. This provides **structural GC balance**: for any input with roughly balanced bits, the output is ~50% GC by construction.

2. **Yin rule** — a context-dependent mapping where the previous nucleotide determines which nucleotide pair maps to each bit value. This reduces homopolymer formation by encouraging base transitions.

3. **Set intersection** — each nucleotide is uniquely determined by intersecting the Yang set (2 elements from GC partition) with the Yin set (2 elements from context rule). The sets always share exactly 1 element.

### Mapping to NucleOS

| Paper concept | Implementation |
|---|---|
| Yang rule (GC partition) | `yang_set()` in `nucle_codec::yinyang` — `0 → {A,T}`, `1 → {C,G}` |
| Yin rule (context-dependent) | `yin_set()` — 4-entry table indexed by previous nucleotide |
| Set intersection | `intersect()` — yields exactly 1 nucleotide per (bit_a, bit_b, prev) triple |
| Bitstream splitting | Each byte → 4 nucleotides: high bit of each pair → Yang, low bit → Yin |
| Rule selection (paper uses 1,536 configs) | NucleOS uses a fixed canonical rule; the paper's rule search is omitted for simplicity |
| Information density | **2.0 bits/nt** theoretical; **1.798 bits/nt** on the current 89-byte `nucle bench` payload and **1.855 bits/nt** in `nucle stress -s 256` |

### Stress test findings

The `nucle stress` command reveals the Yin-Yang codec's behavior across data distributions:

| Distribution | GC% | Homopolymer | Notes |
|---|---|---|---|
| random | 45.7% | 5 | ✓ Near-optimal GC balance |
| text (ASCII) | 40.3% | 4 | ✓ Good — ASCII has reasonable bit balance |
| sequential (0–255) | 48.5% | 8 | ✓ Excellent GC, moderate homopolymers |
| all-zero | 0.5% | 4 | ✗ Yang rule sends all bits to AT partition |
| all-0xFF | 92.6% | 143 | ✗ Yang rule sends all bits to GC partition |

The extreme-case failures are a fundamental limitation of the Yang rule: when *all* input bits are identical, the GC partition can't balance. This is consistent with the paper's observation that the codec works best on "real-world" data with natural bit entropy. A production system would add a whitening/scrambling pre-pass for pathological inputs.

### Source files

- [`nucle_codec/src/yinyang.rs`](../nucle_codec/src/yinyang.rs) — Encoder/decoder with full test suite

---

## 4. Gimpel et al. (2026) — a measured channel

> Gimpel, A. L., Remschak, A., Stark, W. J., Heckel, R., & Grass, R. N. (2026). "Comparison of state-of-the-art error-correction coding for sequence-based DNA data storage." *Nature Communications*. doi:10.1038/s41467-026-70548-3. Reads: ENA study ERP173549 / BioProject PRJEB90546.

One 11,293-oligo pool holding twelve codec configurations (DNA-Aeon, DNA Fountain, Reed-Solomon, HEDGES, Goldman, Yin-Yang), synthesized under a high- and a low-fidelity scenario and read on an Illumina iSeq 100. NucleOS uses runs ERR15127739 and ERR15127744 twice: their error counts became the `array-hifi` and `array-lofi` profiles, and their reads test the consensus and clustering decoders (see [architecture.md](architecture.md#measured-array-pool-channels)). The designed references come from [fml-ethz/dt4dds-benchmark_notebooks](https://github.com/fml-ethz/dt4dds-benchmark_notebooks), which is GPL-3.0, so they are not redistributed here.

---

## 5. Press et al. (2020) — HEDGES

> Press, W. H., Hawkins, J. A., Jones, S. K., Schaub, J. M., & Finkelstein, I. J. (2020). "HEDGES error-correcting code for DNA storage corrects indels and allows sequence constraints." *PNAS*, 117(31), 18489–18496.

### Key idea

Each base encodes a bit through a hash of the bit position and the bits before it, so a single insertion or deletion makes every following base stop matching. A decoder can then search for the bit sequence whose encoding best explains one noisy read, treating insertions and deletions as ordinary edits instead of needing many reads to vote them out.

### Mapping to NucleOS

| Paper | NucleOS |
|---|---|
| Hash of salt, bit position and previous bits picks the base | `HedgesCodec::choose` — 12 previous bits, one of 6 orderings of the 3 bases that differ from the previous base |
| Variable code rates, GC-window constraint | Rate 1/2 only; the homopolymer constraint is built into the base choice |
| A* tree search with edit penalties | `decode_bits_where` — A* with costs `-10 ln p` per edit (`EditCosts::for_channel`) and an admissible length-difference bound |
| Outer Reed-Solomon over many strands | 2-byte index + CRC-8 per strand; list decoding retries the next-cheapest path when the CRC fails; `decode` votes per index |

### Source files

- [`nucle_codec/src/hedges.rs`](../nucle_codec/src/hedges.rs) — Encoder, A* decoder, `DnaCodec` implementation
- [`nucle_codec/examples/hedges_channel.rs`](../nucle_codec/examples/hedges_channel.rs) — Single-read benchmark on a configurable indel channel

---

## Further reading

- Church, G. M., Gao, Y., & Kosuri, S. (2012). "Next-generation digital information storage in DNA." *Science*, 337(6102), 1628. — First demonstration of large-scale DNA storage (659 KB).
- Organick, L., et al. (2018). "Random access in large-scale DNA data storage." *Nature Biotechnology*, 36(3), 242–248. — Random access via primer-based addressing (maps to `nucle_index::primer` and `nucle_index::crispr_sim`).
- DNA Data Storage Alliance (a SNIA technology affiliate), first specifications, March 2024: a fixed-format Sector Zero that tells a reader how to decode a Sector One describing the archive. `nucle_codec::bootstrap` follows the two-sector idea with its own byte layout; it is not a conforming implementation.
- Nguyen, B. H., et al. (2020). "Architecting Datacenters for Sustainability: Greener Data Storage using Synthetic DNA." *Electronics Goes Green 2020+*. — Cradle-to-grave life-cycle assessment of DNA, tape and hard-disk archives; results are given as charts only, which is why `nucle cost` reports no carbon figure.
- US Government Accountability Office (2022), DNA data storage spotlight ([PDF](https://www.gao.gov/assets/730/720590.pdf)), and Blocks & Files, "DNA data storage: when the physics work but the economics don't" (16 January 2026). — The $3,500/MB and ~$100/MB write-cost estimates quoted by `nucle cost`.
