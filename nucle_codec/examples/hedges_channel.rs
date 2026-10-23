//! HEDGES on single reads through a configurable indel channel.
//!
//! ```text
//! cargo run --release -p nucle_codec --example hedges_channel -- [sub ins del] [strands]
//! ```
//!
//! Defaults to the per-base rates measured on the low-fidelity array pool
//! of ERP173549 (run ERR15127744: substitution 0.98%, insertion 0.10%,
//! deletion 0.43%) and reports the share of strands decoded from one read
//! each, with no consensus, checking every decode against the original.

use nucle_codec::base::{DnaCodec, DnaStrand, Nucleotide};
use nucle_codec::hedges::{EditCosts, HedgesCodec, HedgesConfig};
use std::time::Instant;

fn main() {
    let args: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (sub, ins, del) = match args.as_slice() {
        [s, i, d, ..] => (*s, *i, *d),
        _ => (0.0098, 0.0010, 0.0043),
    };
    let strands = args.get(3).copied().unwrap_or(500.0) as usize;
    // Search costs matched to the channel being simulated.
    let codec = HedgesCodec::new(HedgesConfig { costs: EditCosts::for_channel(sub, ins, del), ..HedgesConfig::default() });
    let data: Vec<u8> = (0..strands * 16).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let encoded = codec.encode(&data).unwrap();

    let mut x: u64 = 2026;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 11) as f64 / (1u64 << 53) as f64
    };
    let start = Instant::now();
    let (mut decoded, mut wrong, mut shifted, mut edits, mut gave_up, mut rejected) = (0, 0, 0, 0usize, 0, 0);
    for (k, s) in encoded.strands.iter().enumerate() {
        let mut read = Vec::new();
        for &b in s.bases() {
            let r = rnd();
            if r < del {
                edits += 1;
                continue;
            }
            if rnd() < sub {
                let others: Vec<Nucleotide> = Nucleotide::ALL.iter().copied().filter(|&n| n != b).collect();
                read.push(others[((rnd() * 3.0) as usize).min(2)]);
                edits += 1;
            } else {
                read.push(b);
            }
            if rnd() < ins {
                read.push(Nucleotide::ALL[((rnd() * 4.0) as usize).min(3)]);
                edits += 1;
            }
        }
        if read.len() != s.len() {
            shifted += 1;
        }
        let read = DnaStrand::new(read);
        if let Some((index, payload)) = codec.decode_read(&read) {
            if usize::from(index) == k && payload[..] == data[k * 16..(k + 1) * 16] {
                decoded += 1;
            } else {
                wrong += 1;
            }
        } else if codec.decode_bits(read.bases(), codec.strand_len()).is_none() {
            gave_up += 1;
        } else {
            rejected += 1;
        }
    }
    println!("channel: sub {sub}, ins {ins}, del {del} per base; {} strands of {} nt", encoded.strands.len(), codec.strand_len());
    println!("reads with an indel (length changed): {shifted}");
    println!("mean edits per read: {:.2}", edits as f64 / encoded.strands.len() as f64);
    println!("decoded correctly from a single read: {decoded} / {} ({:.1}%)", encoded.strands.len(), 100.0 * decoded as f64 / encoded.strands.len() as f64);
    println!("decoded to wrong data that passed the CRC: {wrong}");
    println!("not decoded: {gave_up} hit the search limit, {rejected} decoded to bits the CRC rejected");
    println!("elapsed: {:.2}s", start.elapsed().as_secs_f64());
}
