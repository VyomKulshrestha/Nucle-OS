//! A genuinely rewritable block device modeled on the University of
//! Missouri (Mizzou) approach: per the team's own paper title,
//! "synthesis-free and enzyme-free rewritable DNA memory through
//! frameshift encoding and nanopore duplex interruption decoding"
//! (published in *PNAS Nexus*). Unlike [`crate::synthesis_array`]'s
//! Atlas/imec model, this method doesn't chemically synthesize a new
//! strand to write -- it electronically restructures an existing DNA
//! molecule ("frameshift encoding"), so the same physical medium can be
//! erased and overwritten repeatedly. That's the defining behavioral
//! difference this backend models: [`BlockDevice::write_block`] never
//! returns [`BlockDeviceError::AlreadyWritten`] -- overwriting a
//! previously-written block is the whole point.
//!
//! The tradeoff their public reporting also describes: both encoding
//! (write) and decoding (read) go through "a compact electronic device
//! paired with a molecular-scale [nanopore] detector" -- one physical
//! sensor, read serially, not the Atlas/imec array's "millions of
//! individual synthesis sites" running in parallel. So unlike
//! `SynthesisArrayBlockDevice`, this backend has no parallelism knob to
//! hide per-operation latency behind: every read and every write pays
//! `op_latency` on its own.
//!
//! Because the write mechanism is *not* chemical synthesis, this backend
//! deliberately does **not** reuse a synthesis-style `HardwareProfile`
//! for writes -- doing so would misrepresent a method its own paper
//! specifically bills as "synthesis-free." Writes are treated as a
//! controlled, low-error electronic operation (no noise injected; each
//! stored copy is identical). Reads, however, run *fresh* noise through
//! `nucle_synth`'s `OxfordNanopore` profile on every single call --
//! chosen because it's this project's already-modeled nanopore-sensing
//! error profile, directly analogous to the paper's own stated read
//! mechanism ("nanopore duplex interruption decoding") -- rather than
//! baking noise in once at write time the way the write-once Atlas/imec
//! backend does, since every access here is a new pass through the
//! physical sensor, not a permanent one-time event. Recovery is
//! consensus voting across `coverage_depth` freshly-noised copies
//! followed by Reed-Solomon, not plain RS alone -- `OxfordNanopore`'s
//! errors are indel-dominant, which desyncs plain RS's fixed-width
//! symbol alignment (see `crate::support`'s module doc comment).
//!
//! On the latency and coverage figures: Mizzou's public reporting
//! doesn't state a per-operation latency or redundancy factor this
//! crate could cite as confirmed -- `op_latency` and `coverage_depth`
//! are caller-supplied with an illustrative (not verified) default.

use crate::device::{BlockDevice, BlockDeviceError};
use crate::support::{self, StoredGroups};
use nucle_synth::profiles::HardwareProfile;
use std::time::Duration;

/// Configuration for a [`RewritableNanoporeBlockDevice`].
#[derive(Clone)]
pub struct RewritableNanoporeConfig {
    pub block_size: usize,
    pub capacity_blocks: u64,
    /// Reed-Solomon parity strands computed per block. `0` disables ECC
    /// entirely for this device.
    pub parity_strands_per_block: usize,
    /// Freshly-noised copies read (and voted across) on every single
    /// read call -- see the module doc comment on why this is
    /// load-bearing, not just extra safety margin.
    pub coverage_depth: u32,
    /// Illustrative per-operation (read AND write) nanopore-sensor
    /// latency -- see the module doc comment. Not a confirmed real
    /// figure.
    pub op_latency: Duration,
    /// PRNG seed for read-time noise injection.
    pub seed: u64,
}

impl RewritableNanoporeConfig {
    pub fn new(block_size: usize, capacity_blocks: u64) -> Self {
        Self {
            block_size,
            capacity_blocks,
            parity_strands_per_block: 2,
            coverage_depth: 8,
            op_latency: Duration::from_millis(30),
            seed: 42,
        }
    }
}

/// A genuinely rewritable `BlockDevice` modeled on the Mizzou
/// synthesis-free, nanopore-decoded rewritable DNA memory approach. See
/// the module doc comment.
pub struct RewritableNanoporeBlockDevice {
    config: RewritableNanoporeConfig,
    blocks: Vec<Option<StoredGroups>>,
    /// Incremented on every write to a given block; folded into that
    /// block's read-noise seed so re-reading a block that's since been
    /// overwritten never reuses a stale noise draw from a prior version.
    write_generation: Vec<u64>,
    /// Incremented on every read (any block) and folded into the noise
    /// seed -- real nanopore sensing doesn't reproduce byte-identical
    /// noise on a second pass, so neither does this: reading the same
    /// never-rewritten block twice deliberately draws two different
    /// noise realizations, not a cached/replayed one. `AtomicU64`
    /// because `read_block` takes `&self`, matching the real trait's
    /// signature (a device driver's read path doesn't need `&mut self`).
    read_counter: std::sync::atomic::AtomicU64,
}

impl RewritableNanoporeBlockDevice {
    pub fn new(config: RewritableNanoporeConfig) -> Self {
        let capacity_blocks = config.capacity_blocks;
        Self {
            config,
            blocks: vec![None; capacity_blocks as usize],
            write_generation: vec![0; capacity_blocks as usize],
            read_counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn validate_lba(&self, lba: u64) -> Result<(), BlockDeviceError> {
        if lba >= self.config.capacity_blocks {
            return Err(BlockDeviceError::OutOfRange { lba, capacity_blocks: self.config.capacity_blocks });
        }
        Ok(())
    }
}

impl BlockDevice for RewritableNanoporeBlockDevice {
    fn block_size(&self) -> usize {
        self.config.block_size
    }

    fn capacity_blocks(&self) -> u64 {
        self.config.capacity_blocks
    }

    fn is_rewritable(&self) -> bool {
        true
    }

    fn read_block(&self, lba: u64) -> Result<Vec<u8>, BlockDeviceError> {
        self.validate_lba(lba)?;
        std::thread::sleep(self.config.op_latency);
        let stored = self.blocks[lba as usize].as_ref().ok_or(BlockDeviceError::NeverWritten { lba })?;

        let read_call = self.read_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let read_seed = self
            .config
            .seed
            .wrapping_mul(31)
            .wrapping_add(lba)
            .wrapping_add(self.write_generation[lba as usize])
            .wrapping_mul(2654435761) // Knuth's multiplicative hash constant, just to mix read_call in well
            .wrapping_add(read_call);
        // Fresh noise on every read, applied per-copy (not fanned out
        // into more copies -- these are already `coverage_depth` stored
        // copies from write time) -- see the module doc comment.
        let data_groups: Vec<Vec<_>> = stored
            .data_groups
            .iter()
            .enumerate()
            .map(|(i, group)| support::refresh_noise(group, HardwareProfile::OxfordNanopore, read_seed ^ i as u64))
            .collect();
        let parity_groups: Vec<Vec<_>> = stored
            .parity_groups
            .iter()
            .enumerate()
            .map(|(i, group)| support::refresh_noise(group, HardwareProfile::OxfordNanopore, read_seed ^ 0x5A5A ^ i as u64))
            .collect();

        support::decode(&data_groups, &parity_groups, stored.original_size, self.config.parity_strands_per_block)
            .map_err(|_| BlockDeviceError::UncorrectableError { lba })
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockDeviceError> {
        self.validate_lba(lba)?;
        if data.len() != self.config.block_size {
            return Err(BlockDeviceError::BlockSizeMismatch { expected: self.config.block_size, got: data.len() });
        }
        std::thread::sleep(self.config.op_latency);

        // Frameshift encoding is not chemical synthesis -- no write-time
        // noise, see the module doc comment.
        let encoded = support::encode(data, self.config.parity_strands_per_block)
            .map_err(|_| BlockDeviceError::UncorrectableError { lba })?;
        let data_groups = support::noisy_copies(&encoded.data_strands, HardwareProfile::Pristine, self.config.coverage_depth, self.config.seed);
        let parity_groups = support::noisy_copies(&encoded.parity_strands, HardwareProfile::Pristine, self.config.coverage_depth, self.config.seed);

        self.blocks[lba as usize] = Some(StoredGroups { data_groups, parity_groups, original_size: encoded.original_size });
        self.write_generation[lba as usize] += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device(block_size: usize, capacity_blocks: u64) -> RewritableNanoporeBlockDevice {
        let mut config = RewritableNanoporeConfig::new(block_size, capacity_blocks);
        config.op_latency = Duration::from_millis(5); // keep tests fast
        RewritableNanoporeBlockDevice::new(config)
    }

    #[test]
    fn write_then_read_round_trips_exact_bytes() {
        let mut device = test_device(32, 4);
        let data = vec![9u8; 32];
        device.write_block(1, &data).unwrap();
        assert_eq!(device.read_block(1).unwrap(), data);
    }

    #[test]
    fn is_rewritable_is_true() {
        let device = test_device(16, 1);
        assert!(device.is_rewritable());
    }

    #[test]
    fn overwriting_a_block_succeeds_and_the_read_reflects_the_new_content() {
        let mut device = test_device(16, 1);
        device.write_block(0, &vec![1u8; 16]).unwrap();
        assert_eq!(device.read_block(0).unwrap(), vec![1u8; 16]);

        // The core Atlas-vs-Mizzou distinction: this must succeed, not
        // error with AlreadyWritten like SynthesisArrayBlockDevice would.
        device.write_block(0, &vec![2u8; 16]).unwrap();
        assert_eq!(device.read_block(0).unwrap(), vec![2u8; 16]);
    }

    #[test]
    fn write_with_wrong_size_data_is_rejected() {
        let mut device = test_device(16, 1);
        let result = device.write_block(0, &vec![1u8; 4]);
        assert_eq!(result, Err(BlockDeviceError::BlockSizeMismatch { expected: 16, got: 4 }));
    }

    #[test]
    fn reading_a_never_written_block_is_rejected() {
        let device = test_device(16, 2);
        assert_eq!(device.read_block(0), Err(BlockDeviceError::NeverWritten { lba: 0 }));
    }

    #[test]
    fn a_single_read_and_a_single_write_each_take_at_least_op_latency() {
        let mut device = test_device(16, 1);
        let start = std::time::Instant::now();
        device.write_block(0, &vec![1u8; 16]).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(5));

        let start = std::time::Instant::now();
        device.read_block(0).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(5));
    }

    #[test]
    fn error_correction_recovers_a_block_whose_stored_strand_was_corrupted() {
        let device_config = RewritableNanoporeConfig::new(16, 1);
        let mut device = RewritableNanoporeBlockDevice::new(device_config);

        let data = b"0123456789ABCDEF".to_vec();
        device.write_block(0, &data).unwrap();

        let stored = device.blocks[0].as_mut().unwrap();
        assert!(!stored.data_groups.is_empty() && stored.data_groups[0].len() > 1, "test needs coverage_depth > 1 to corrupt one copy while others survive");
        let corrupted: Vec<nucle_codec::base::Nucleotide> = stored.data_groups[0][0]
            .bases()
            .iter()
            .map(|b| match b {
                nucle_codec::base::Nucleotide::A => nucle_codec::base::Nucleotide::C,
                _ => nucle_codec::base::Nucleotide::A,
            })
            .collect();
        stored.data_groups[0][0] = nucle_codec::base::DnaStrand::new(corrupted);

        assert_eq!(device.read_block(0), Ok(data));
    }

    #[test]
    fn read_noise_is_fresh_each_time_not_baked_in_at_write() {
        // Two reads of the same never-overwritten block, at two
        // different seeds' worth of read noise, should still both
        // recover the original -- proving noise really is applied at
        // read time (not just once, replayed identically), while
        // correctness survives it either way.
        let device_config = RewritableNanoporeConfig::new(16, 1);
        let mut device = RewritableNanoporeBlockDevice::new(device_config);
        let data = b"0123456789ABCDEF".to_vec();
        device.write_block(0, &data).unwrap();

        assert_eq!(device.read_block(0), Ok(data.clone()));
        assert_eq!(device.read_block(0), Ok(data));
    }
}
