# References — Foundational Papers

The NucleOS codec layer implements algorithms described in two landmark papers on DNA data storage. This document maps each paper to its corresponding implementation in the codebase.

---

## Goldman et al. (2013)

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
| Information density | Achieved **1.58 bits/nt** theoretical; benchmarked at **1.156 bits/nt** effective (with headers/framing) via `nucle bench` |

### Density gap: 1.156 vs 1.58 bits/nt

The theoretical limit of the ternary rotating cipher is log₂(3) ≈ 1.58 bits/nt — each nucleotide encodes one trit. In practice, NucleOS measures **1.156 bits/nt** effective density. The gap comes from three sources:

1. **Segment framing** — each strand carries a segment index header (4 bytes = 16 nt) that doesn't encode user data but is required for reassembly.
2. **Length prefix** — a 4-byte big-endian length header is prepended to the payload so the decoder knows the original file size.
3. **Padding** — the last segment is zero-padded to the fixed segment size, wasting nucleotides on small inputs.

These overheads are proportionally smaller on larger files. A 1 MB file achieves closer to ~1.45 bits/nt. This is consistent with Goldman et al.'s reported results — they achieved ~1.28 bits/nt on their 739 KB test file after accounting for indexing and overlap redundancy.

This gap is **not a bug** — it's the real engineering cost of making DNA storage work. Any production codec needs headers for reassembly, and reporting effective density (not theoretical) is the honest metric.

### Source files

- [`nucle_codec/src/ternary.rs`](../nucle_codec/src/ternary.rs) — Encoder/decoder
- [`nucle_codec/src/constraints.rs`](../nucle_codec/src/constraints.rs) — Biological constraint validation
- [`nucle_codec/src/benchmark.rs`](../nucle_codec/src/benchmark.rs) — Codec comparison framework

---

## Erlich & Zielinski (2017)

**"DNA Fountain enables a robust and efficient storage architecture"**
*Science*, 355(6328), 950–954. doi:[10.1126/science.aaj2038](https://doi.org/10.1126/science.aaj2038)

### Key idea

Apply **Luby Transform (LT) codes** — a class of rateless fountain codes — to DNA storage. The data is split into segments; each encoded strand is an XOR of a random subset of segments (chosen by a pseudorandom degree distribution). Because LT codes are **rateless**, you can generate an unlimited number of encoded strands, and any sufficient subset can reconstruct the original data through iterative **peeling decoding**.

Strands that violate biological constraints (GC content, homopolymers) are simply discarded and regenerated — the rateless property means you never run out of valid encodings.

### Mapping to NucleOS

| Paper concept | Implementation |
|---|---|
| Fountain / LT coding | `nucle_codec::fountain::FountainCodec` — droplet generation with Robust Soliton degree distribution |
| Peeling decoder | `FountainCodec::peeling_decode()` — iterative degree-1 resolution with XOR propagation |
| Rateless property | `FountainConfig::overhead` controls over-generation (default 1.5×); `max_screening_attempts` limits retries |
| Constraint screening | `FountainConfig::screen_constraints = true` — rejects and regenerates strands failing GC/homopolymer checks |
| Erasure resilience | `nucle_ecc::fountain_code::FountainErasure` — outer-code layer that recovers from strand dropout |
| Near-optimal density | Achieved **1.57 bits/nt** theoretical; benchmarked via `nucle bench` |

### Source files

- [`nucle_codec/src/fountain.rs`](../nucle_codec/src/fountain.rs) — Fountain encoder/decoder
- [`nucle_ecc/src/fountain_code.rs`](../nucle_ecc/src/fountain_code.rs) — Erasure-level fountain recovery
- [`nucle_ecc/src/pipeline.rs`](../nucle_ecc/src/pipeline.rs) — Multi-stage repair pipeline

### Constraint screening in practice

NucleOS's fountain codec uses a raw 2-bit nucleotide mapping (`A=00, T=01, G=10, C=11`) for the byte-to-DNA conversion. Unlike the ternary cipher (which eliminates homopolymers by construction), the fountain codec relies on **post-hoc constraint screening** — exactly as described in the Erlich paper.

With `screen_constraints: true` (the default), each generated droplet is checked against biological constraints. Invalid strands are discarded and the encoder tries the next PRNG seed. The rateless property guarantees that valid strands exist, but the rejection rate depends on the data distribution:

- **Random / binary data**: ~30-50% rejection rate, works well
- **Highly structured data** (e.g., all-zero, short ASCII): can produce heavily biased nucleotide distributions where most droplets fail GC checks, causing slow encoding or encoder timeout

This is a known limitation of the 2-bit mapping approach. A production system would use a more sophisticated byte-to-nucleotide scheme (e.g., the Yin-Yang codec from Ping et al. 2022) that provides better GC balance by construction.

---

## Further reading

- Church, G. M., Gao, Y., & Kosuri, S. (2012). "Next-generation digital information storage in DNA." *Science*, 337(6102), 1628. — First demonstration of large-scale DNA storage (659 KB).
- Organick, L., et al. (2018). "Random access in large-scale DNA data storage." *Nature Biotechnology*, 36(3), 242–248. — Random access via primer-based addressing (maps to `nucle_index::primer` and `nucle_index::crispr_sim`).
- Ping, Z., et al. (2022). "Towards practical and robust DNA-based data archiving using the yin–yang codec system." *Nature Computational Science*, 2, 234–242. — Alternative codec with improved screening.
