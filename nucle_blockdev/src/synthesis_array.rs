//! A write-once block device modeled on the Atlas/imec approach: a dense
//! electrochemical DNA-synthesis array on a CMOS ASIC. Per imec's public
//! description of the partnership, the chip "orchestrat[es] and
//! controll[s] millions of individual synthesis sites" -- real
//! parallelism across many independent synthesis cells, but each cell
//! physically synthesizes a strand base-by-base and, once synthesized,
//! that strand can't be un-synthesized. That's the two defining
//! constraints this backend models: **write-once** (a block, once
//! written, can never be overwritten -- attempting to returns
//! [`BlockDeviceError::AlreadyWritten`]) and **array parallelism**
//! (`array_parallelism` blocks' worth of synthesis can be in flight at
//! once, so batched writes' *throughput* scales with it, even though a
//! single write's own latency floor never shrinks below one synthesis
//! cycle).
//!
//! Errors are introduced once, at write time, via `nucle_synth`'s
//! `TwistBioscience` profile -- a real, already-modeled silicon-chip
//! synthesis error profile in this project's catalog, chosen by
//! structural analogy (dense chip-based synthesis) since neither the
//! Atlas/imec announcement nor imec's press materials publish their own
//! per-base error rate. Those errors are permanent from that point on,
//! exactly like a real synthesized molecule -- reading it back doesn't
//! reroll them, it just recovers what's actually physically there via
//! consensus + Reed-Solomon (real coverage-depth redundancy, storing
//! several independently-noised copies per logical strand -- the array's
//! massive parallelism plausibly supports exactly this kind of redundant
//! synthesis, and it's also load-bearing: `TwistBioscience`'s errors are
//! deletion-dominant, which plain Reed-Solomon alone can't correct at
//! all since a deletion desyncs its fixed-width symbol alignment --
//! only consensus voting across independent copies can, see
//! `crate::support`'s module doc comment for how this was discovered).
//!
//! On the latency and coverage figures: neither public source states a
//! real per-write cycle time, parallelism count, or redundancy factor
//! this crate could cite as confirmed -- `write_latency`,
//! `array_parallelism`, and `coverage_depth` are caller-supplied with an
//! illustrative (not verified) default.

use crate::device::{BlockDevice, BlockDeviceError};
use crate::support::{self, StoredGroups};
use nucle_synth::profiles::HardwareProfile;
use std::time::Duration;

/// Configuration for a [`SynthesisArrayBlockDevice`].
#[derive(Clone)]
pub struct SynthesisArrayConfig {
    pub block_size: usize,
    pub capacity_blocks: u64,
    /// Reed-Solomon parity strands computed per block. `0` disables ECC
    /// entirely for this device.
    pub parity_strands_per_block: usize,
    /// Independently-noised copies stored per logical strand -- see the
    /// module doc comment on why this is load-bearing, not just extra
    /// safety margin.
    pub coverage_depth: u32,
    /// Illustrative per-write electrochemical synthesis cycle latency --
    /// see the module doc comment. Not a confirmed real figure.
    pub write_latency: Duration,
    /// How many blocks' synthesis this array can run concurrently,
    /// modeling "millions of individual synthesis sites" bounded down to
    /// something a caller can actually configure and a test can exercise.
    pub array_parallelism: usize,
    /// PRNG seed for write-time noise injection.
    pub seed: u64,
}

impl SynthesisArrayConfig {
    pub fn new(block_size: usize, capacity_blocks: u64) -> Self {
        Self {
            block_size,
            capacity_blocks,
            parity_strands_per_block: 2,
            coverage_depth: 5,
            write_latency: Duration::from_millis(50),
            array_parallelism: 8,
            seed: 42,
        }
    }
}

/// A write-once (WORM) `BlockDevice` modeled on the Atlas/imec
/// electrochemical synthesis array approach. See the module doc comment.
pub struct SynthesisArrayBlockDevice {
    config: SynthesisArrayConfig,
    blocks: Vec<Option<StoredGroups>>,
}

impl SynthesisArrayBlockDevice {
    pub fn new(config: SynthesisArrayConfig) -> Self {
        let capacity_blocks = config.capacity_blocks;
        Self { config, blocks: vec![None; capacity_blocks as usize] }
    }

    /// Writes several blocks, modeling the array's real parallelism: the
    /// batch pays `write_latency` once per `ceil(writes.len() /
    /// array_parallelism)` chunk rather than once per individual write,
    /// the same aggregate timing `array_parallelism` concurrently-running
    /// synthesis sites would produce. (Simulated as elapsed wall-clock
    /// time directly, not by spawning one real OS thread per site --
    /// `write_block` can't be called concurrently on `&mut self` anyway,
    /// and the aggregate timing is what this is modeling.)
    pub fn write_batch(&mut self, writes: &[(u64, Vec<u8>)]) -> Vec<Result<(), BlockDeviceError>> {
        let mut results = Vec::with_capacity(writes.len());
        for chunk in writes.chunks(self.config.array_parallelism.max(1)) {
            std::thread::sleep(self.config.write_latency);
            for (lba, data) in chunk {
                results.push(self.write_block_no_latency(*lba, data));
            }
        }
        results
    }

    fn write_block_no_latency(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockDeviceError> {
        self.validate_lba(lba)?;
        if data.len() != self.config.block_size {
            return Err(BlockDeviceError::BlockSizeMismatch { expected: self.config.block_size, got: data.len() });
        }
        if self.blocks[lba as usize].is_some() {
            return Err(BlockDeviceError::AlreadyWritten { lba });
        }

        let encoded = support::encode(data, self.config.parity_strands_per_block)
            .map_err(|_| BlockDeviceError::UncorrectableError { lba })?;

        // Permanent write-time noise, stored as `coverage_depth`
        // independently-noised copies per strand -- baked in from now
        // on, per the module doc comment.
        let data_groups = support::noisy_copies(
            &encoded.data_strands,
            HardwareProfile::TwistBioscience,
            self.config.coverage_depth,
            self.config.seed ^ lba,
        );
        let parity_groups = support::noisy_copies(
            &encoded.parity_strands,
            HardwareProfile::TwistBioscience,
            self.config.coverage_depth,
            self.config.seed ^ lba ^ 0xA5A5,
        );

        self.blocks[lba as usize] = Some(StoredGroups { data_groups, parity_groups, original_size: encoded.original_size });
        Ok(())
    }

    fn validate_lba(&self, lba: u64) -> Result<(), BlockDeviceError> {
        if lba >= self.config.capacity_blocks {
            return Err(BlockDeviceError::OutOfRange { lba, capacity_blocks: self.config.capacity_blocks });
        }
        Ok(())
    }
}

impl BlockDevice for SynthesisArrayBlockDevice {
    fn block_size(&self) -> usize {
        self.config.block_size
    }

    fn capacity_blocks(&self) -> u64 {
        self.config.capacity_blocks
    }

    fn is_rewritable(&self) -> bool {
        false
    }

    fn read_block(&self, lba: u64) -> Result<Vec<u8>, BlockDeviceError> {
        self.validate_lba(lba)?;
        let stored = self.blocks[lba as usize].as_ref().ok_or(BlockDeviceError::NeverWritten { lba })?;

        // Permanent storage: no additional noise at read time, just
        // recovering what's physically there via consensus + RS.
        support::decode(&stored.data_groups, &stored.parity_groups, stored.original_size, self.config.parity_strands_per_block)
            .map_err(|_| BlockDeviceError::UncorrectableError { lba })
    }

    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockDeviceError> {
        std::thread::sleep(self.config.write_latency);
        self.write_block_no_latency(lba, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device(block_size: usize, capacity_blocks: u64) -> SynthesisArrayBlockDevice {
        let mut config = SynthesisArrayConfig::new(block_size, capacity_blocks);
        config.write_latency = Duration::from_millis(5); // keep tests fast
        SynthesisArrayBlockDevice::new(config)
    }

    #[test]
    fn write_then_read_round_trips_exact_bytes() {
        let mut device = test_device(32, 4);
        let data = vec![7u8; 32];
        device.write_block(0, &data).unwrap();
        assert_eq!(device.read_block(0).unwrap(), data);
    }

    #[test]
    fn write_at_an_already_written_block_is_rejected() {
        let mut device = test_device(16, 2);
        device.write_block(0, &vec![1u8; 16]).unwrap();
        let result = device.write_block(0, &vec![2u8; 16]);
        assert_eq!(result, Err(BlockDeviceError::AlreadyWritten { lba: 0 }));
    }

    #[test]
    fn is_rewritable_is_false() {
        let device = test_device(16, 1);
        assert!(!device.is_rewritable());
    }

    #[test]
    fn write_with_wrong_size_data_is_rejected() {
        let mut device = test_device(16, 1);
        let result = device.write_block(0, &vec![1u8; 8]);
        assert_eq!(result, Err(BlockDeviceError::BlockSizeMismatch { expected: 16, got: 8 }));
    }

    #[test]
    fn write_out_of_range_is_rejected() {
        let mut device = test_device(16, 2);
        let result = device.write_block(5, &vec![1u8; 16]);
        assert_eq!(result, Err(BlockDeviceError::OutOfRange { lba: 5, capacity_blocks: 2 }));
    }

    #[test]
    fn reading_a_never_written_block_is_rejected() {
        let device = test_device(16, 2);
        assert_eq!(device.read_block(0), Err(BlockDeviceError::NeverWritten { lba: 0 }));
    }

    #[test]
    fn a_single_write_takes_at_least_its_configured_latency() {
        let mut device = test_device(16, 1);
        let start = std::time::Instant::now();
        device.write_block(0, &vec![1u8; 16]).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(5));
    }

    #[test]
    fn batched_writes_pay_latency_once_per_parallelism_chunk_not_once_per_write() {
        let mut config = SynthesisArrayConfig::new(16, 16);
        // A larger latency than other tests use so the effect being
        // measured (theoretical savings = 120ms) stays comfortably
        // bigger than the real encode/noise/RS work's own run-to-run
        // jitter under CI/full-workspace-parallel load, which this test
        // was originally flaky against at 20ms.
        config.write_latency = Duration::from_millis(60);
        config.array_parallelism = 4;
        let writes: Vec<(u64, Vec<u8>)> = (0..8).map(|i| (i, vec![i as u8; 16])).collect();

        // Compare against the same 8 writes done one at a time (no
        // parallelism-chunk batching) on a fresh device with identical
        // config, rather than a fixed wall-clock threshold -- this
        // isolates the real effect being tested (chunking pays latency
        // once per chunk, not once per write) from how fast any given
        // machine/build happens to run the real encode/noise/RS work
        // both paths do identically.
        let mut sequential_device = SynthesisArrayBlockDevice::new(config.clone());
        let sequential_start = std::time::Instant::now();
        for (lba, data) in &writes {
            sequential_device.write_block(*lba, data).unwrap();
        }
        let sequential_elapsed = sequential_start.elapsed();

        let mut batched_device = SynthesisArrayBlockDevice::new(config);
        let batched_start = std::time::Instant::now();
        let results = batched_device.write_batch(&writes);
        let batched_elapsed = batched_start.elapsed();

        assert!(results.iter().all(Result::is_ok));
        // 8 writes / 4 parallelism = 2 latency-paying chunks vs. 8 --
        // batching should save close to (8 - 2) * write_latency = 360ms
        // of pure latency, on top of however long the real encode/
        // noise/RS work (identical in both paths) happens to take on
        // this machine/build -- asserting the *savings*, not an
        // absolute time, is what isolates the actual effect being
        // tested from build/machine speed. The required threshold is
        // well under the 360ms theoretical max to tolerate real-work
        // jitter under CI/parallel-test-suite load.
        let savings = sequential_elapsed.saturating_sub(batched_elapsed);
        assert!(
            savings >= Duration::from_millis(150),
            "expected batching to save close to 360ms of latency (6 fewer chunks x 60ms), saved only {savings:?} (sequential {sequential_elapsed:?}, batched {batched_elapsed:?})"
        );
    }

    #[test]
    fn error_correction_recovers_a_block_whose_stored_strand_was_corrupted() {
        // Corrupt one entire stored copy directly (bypassing write-time
        // noise) to prove consensus + RS parity -- not luck from a low
        // noise seed -- is what makes recovery work.
        let device_config = SynthesisArrayConfig::new(16, 1);
        let mut device = SynthesisArrayBlockDevice::new(device_config);

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
}
