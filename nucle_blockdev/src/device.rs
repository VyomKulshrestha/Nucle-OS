//! The `BlockDevice` trait: the interface a filesystem driver actually
//! needs (read this fixed-size block, write that one), as distinct from
//! `nucle_vfs`'s existing whole-file syscall API (`dna_write(filename,
//! bytes)`), which has no notion of block size or address space at all.

use thiserror::Error;

/// Errors a `BlockDevice` implementation can return. Deliberately
/// mirrors the shape of real block-device error conditions (bad address,
/// wrong transfer size, medium refuses the write, unrecoverable read) --
/// this is what a filesystem driver built against this trait would
/// actually need to branch on.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum BlockDeviceError {
    #[error("block address {lba} is out of range (capacity is {capacity_blocks} blocks)")]
    OutOfRange { lba: u64, capacity_blocks: u64 },
    #[error("write of {got} bytes doesn't match this device's block size of {expected} bytes")]
    BlockSizeMismatch { expected: usize, got: usize },
    #[error("block {lba} was already written and this device does not support overwriting")]
    AlreadyWritten { lba: u64 },
    #[error("block {lba} was never written")]
    NeverWritten { lba: u64 },
    #[error("block {lba} could not be read back correctly -- error correction was exhausted")]
    UncorrectableError { lba: u64 },
}

/// A fixed-block-size, address-addressed storage device.
///
/// This is deliberately narrow -- just `read_block`/`write_block` over a
/// linear address space, exactly the seam a real block-layer driver (or a
/// toy filesystem built on top, see [`crate::blob`]) needs, and nothing
/// `nucle_vfs`'s filename/catalog/primer-addressed API already covers.
/// Every method is synchronous and blocking, matching how a real
/// in-kernel block device request is handled -- latency is real elapsed
/// time inside the call, not a background task the caller has to poll.
pub trait BlockDevice {
    /// Size, in bytes, of every block this device exposes. Fixed for the
    /// device's lifetime -- real block devices don't change sector size
    /// mid-operation.
    fn block_size(&self) -> usize;

    /// Total addressable blocks. Valid `lba` values are `0..capacity_blocks()`.
    fn capacity_blocks(&self) -> u64;

    /// Whether a block can be overwritten after its first write.
    /// `false` for write-once/permanent media (the Atlas/imec-style
    /// [`crate::synthesis_array`] backend -- you can't un-synthesize a
    /// strand); `true` for genuinely rewritable media (the Mizzou-style
    /// [`crate::rewritable_nanopore`] backend).
    fn is_rewritable(&self) -> bool;

    /// Read the block at `lba`, running it through this device's
    /// error-correction layer. Returns exactly `block_size()` bytes on
    /// success.
    fn read_block(&self, lba: u64) -> Result<Vec<u8>, BlockDeviceError>;

    /// Write `data` (must be exactly `block_size()` bytes) to `lba`.
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockDeviceError>;
}
