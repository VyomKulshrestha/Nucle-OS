//! A minimal demonstration that a filesystem-like consumer can be built
//! generically against [`BlockDevice`], without knowing which physical
//! backend it's talking to -- proving the abstraction is the seam a real
//! filesystem driver would need, not just a read/write API in name.
//! Deliberately not a real filesystem (no directory structure, no
//! free-space bitmap, no multi-file layout) -- just a length-prefixed,
//! multi-block blob, the smallest thing that's genuinely "a layer on top
//! of block addressing" rather than another block-level API.

use crate::device::{BlockDevice, BlockDeviceError};

/// Writes `data` starting at `first_lba`: one dedicated header block
/// (an 8-byte little-endian length, matching how a real inode/superblock
/// records a file's size) followed by as many data blocks as needed (the
/// last zero-padded). Returns the total number of blocks used (header +
/// data).
pub fn write_blob(device: &mut dyn BlockDevice, first_lba: u64, data: &[u8]) -> Result<u64, BlockDeviceError> {
    let block_size = device.block_size();
    let data_blocks_needed = data.len().div_ceil(block_size.max(1)) as u64;
    let total_blocks_needed = 1 + data_blocks_needed;
    let last_lba = first_lba + total_blocks_needed - 1;
    if last_lba >= device.capacity_blocks() {
        return Err(BlockDeviceError::OutOfRange { lba: last_lba, capacity_blocks: device.capacity_blocks() });
    }

    let mut header = vec![0u8; block_size];
    header[0..8].copy_from_slice(&(data.len() as u64).to_le_bytes());
    device.write_block(first_lba, &header)?;

    for (i, chunk_start) in (0..data.len()).step_by(block_size).enumerate() {
        let chunk_end = (chunk_start + block_size).min(data.len());
        let mut block = vec![0u8; block_size];
        block[0..chunk_end - chunk_start].copy_from_slice(&data[chunk_start..chunk_end]);
        device.write_block(first_lba + 1 + i as u64, &block)?;
    }

    Ok(total_blocks_needed)
}

/// Reverse of [`write_blob`]: reads the header block at `first_lba` to
/// learn the real length, then reads exactly as many data blocks as that
/// length requires and trims the last block's zero padding.
pub fn read_blob(device: &dyn BlockDevice, first_lba: u64) -> Result<Vec<u8>, BlockDeviceError> {
    let header = device.read_block(first_lba)?;
    let len = u64::from_le_bytes(header[0..8].try_into().expect("header block is at least 8 bytes")) as usize;
    let block_size = device.block_size();
    let data_blocks_needed = len.div_ceil(block_size.max(1));

    let mut result = Vec::with_capacity(len);
    for i in 0..data_blocks_needed {
        result.extend_from_slice(&device.read_block(first_lba + 1 + i as u64)?);
    }
    result.truncate(len);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewritable_nanopore::{RewritableNanoporeBlockDevice, RewritableNanoporeConfig};
    use crate::synthesis_array::{SynthesisArrayBlockDevice, SynthesisArrayConfig};
    use std::time::Duration;

    fn fast_synthesis_array(block_size: usize, capacity_blocks: u64) -> SynthesisArrayBlockDevice {
        let mut config = SynthesisArrayConfig::new(block_size, capacity_blocks);
        config.write_latency = Duration::from_millis(1);
        SynthesisArrayBlockDevice::new(config)
    }

    fn fast_rewritable_nanopore(block_size: usize, capacity_blocks: u64) -> RewritableNanoporeBlockDevice {
        let mut config = RewritableNanoporeConfig::new(block_size, capacity_blocks);
        config.op_latency = Duration::from_millis(1);
        RewritableNanoporeBlockDevice::new(config)
    }

    /// The same blob code, unmodified, against both real-world-modeled
    /// backends -- the point of the abstraction.
    fn round_trips_a_multi_block_blob(device: &mut dyn BlockDevice) {
        let data = b"a message that is deliberately longer than one block".to_vec();
        let blocks_used = write_blob(device, 0, &data).unwrap();
        assert!(blocks_used > 1, "expected the message to span more than one data block plus the header");
        assert_eq!(read_blob(device, 0).unwrap(), data);
    }

    #[test]
    fn round_trips_across_the_synthesis_array_backend() {
        round_trips_a_multi_block_blob(&mut fast_synthesis_array(16, 8));
    }

    #[test]
    fn round_trips_across_the_rewritable_nanopore_backend() {
        round_trips_a_multi_block_blob(&mut fast_rewritable_nanopore(16, 8));
    }

    #[test]
    fn empty_blob_round_trips_to_empty() {
        let mut device = fast_rewritable_nanopore(16, 2);
        write_blob(&mut device, 0, &[]).unwrap();
        assert_eq!(read_blob(&device, 0).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_blob_needing_more_blocks_than_available_is_rejected() {
        let mut device = fast_rewritable_nanopore(16, 2);
        // 2 capacity blocks, but a header block + a 20-byte payload
        // needs a header + 2 data blocks = 3.
        let result = write_blob(&mut device, 0, &vec![1u8; 20]);
        assert!(matches!(result, Err(BlockDeviceError::OutOfRange { .. })));
    }

    #[test]
    fn overwriting_a_blob_on_the_rewritable_backend_replaces_its_content() {
        let mut device = fast_rewritable_nanopore(16, 8);
        write_blob(&mut device, 0, b"first version").unwrap();
        write_blob(&mut device, 0, b"second, different version").unwrap();
        assert_eq!(read_blob(&device, 0).unwrap(), b"second, different version");
    }
}
