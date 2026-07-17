//! # nucle_blockdev
//!
//! A block-device abstraction over simulated DNA/molecular storage
//! backends -- fixed-size, address-addressed read/write, the layer a
//! filesystem driver actually sits on top of. `nucle_vfs`'s existing
//! `dna_write(filename, bytes)` API is a whole-file syscall with a
//! catalog, primer-tagged retrieval, and no notion of a fixed block size
//! or address space at all; this crate is deliberately narrower and
//! sits parallel to it, not underneath it.
//!
//! Two backends model two real, structurally different DNA storage
//! approaches -- named because their physical constraints genuinely
//! shape what a block device built on top of them can and can't do,
//! not as a generic "slow disk" simulator with two coats of paint:
//!
//! - [`synthesis_array`] models the **Atlas/imec** approach: a dense
//!   electrochemical DNA-synthesis array on a CMOS ASIC,
//!   "orchestrating and controlling millions of individual synthesis
//!   sites" per imec's public description of the partnership --
//!   **write-once** (a strand, once synthesized, can't be
//!   un-synthesized) with real array parallelism across blocks.
//! - [`rewritable_nanopore`] models the **Mizzou** (University of
//!   Missouri) approach: "synthesis-free and enzyme-free rewritable DNA
//!   memory through frameshift encoding and nanopore duplex
//!   interruption decoding" (their paper's own title, *PNAS Nexus*) --
//!   genuinely **rewritable** in place, but every read and write goes
//!   through one serial nanopore sensor, so there's no array-style
//!   parallelism to hide per-operation latency behind.
//!
//! Both implement the shared [`BlockDevice`] trait and compose the same
//! real encode/decode (`nucle_codec`'s ternary codec) and
//! error-correction (`nucle_ecc`'s Reed-Solomon, real combined
//! error-and-erasure Berlekamp-Welch decoding) layers `nucle_vfs`
//! already uses at the file level -- this crate doesn't reinvent
//! either, it exposes them through block addressing instead of
//! filenames. [`blob`] demonstrates a minimal filesystem-like consumer
//! built generically against the trait, proving a real driver could
//! sit on top without knowing which physical backend it's talking to --
//! the point of "the OS layer is ready when real DNA drives ship."
//!
//! **On the exact latency and error-rate figures**: neither the
//! Atlas/imec partnership announcement nor the Mizzou paper's public
//! reporting states a per-write/per-read latency, array parallelism
//! count, or per-base error rate this crate could cite as confirmed.
//! Every such number here (`write_latency`, `op_latency`,
//! `array_parallelism`, and the choice of which existing
//! `nucle_synth::HardwareProfile` approximates each backend's error
//! behavior) is caller-configurable with an illustrative, clearly
//! documented default -- not a verified figure from either source. This
//! is the same honesty stance this workspace already takes with
//! `nucle_hardware`'s vendor adapters where a real number wasn't public.

pub mod blob;
pub mod device;
pub mod rewritable_nanopore;
pub mod synthesis_array;
mod support;

pub use device::{BlockDevice, BlockDeviceError};
pub use rewritable_nanopore::{RewritableNanoporeBlockDevice, RewritableNanoporeConfig};
pub use synthesis_array::{SynthesisArrayBlockDevice, SynthesisArrayConfig};
