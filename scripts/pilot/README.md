# Wet-lab pilot kit

Everything a first real write-and-read of NucleScript needs, short of the
order itself: the program that writes the archive, order sheets for Twist
and IDT, a dry run that predicts the result from measured channels, and the
program that scores the sequencer's reads. Nothing here orders, pays for or
ships anything. A person places the order, with a vendor quote and a
sequencing lab lined up.

## What the pilot tests

`pilot.nsl` stores `pilot.txt` (250 bytes) in a Twist pool with the HEDGES
codec and 4 Reed-Solomon parity strands: 23 oligos, 3 header strands that
describe the archive and 20 file strands (16 data, 4 parity). With a
primer site on each end they run 100 to 232 nt, 5,144 nt in all.

The pilot passes if the sequencer's reads, clustered with no labels,
reproduce at least 95% of the ordered oligos exactly (`check.nsl`), and the
header strands give back the file table.

## 1. Pick the primer pair

Every oligo gets one primer site on each end, so the whole pool, header
strands included, amplifies with a single PCR; each file's own primers stay
inside for picking one file out later. Ask the sequencing lab which pair
(or adapter-tailed pair) it uses for oligo pools. `--flank` refuses a pair
outside the usual rules of thumb: 18 to 30 nt, 40 to 60% GC, no run of four
identical bases, melting temperatures within 5 °C, and neither primer's last
12 bases anywhere in the payload, on either strand.

The pair below, `ACGTCAGTGCATGACTGACT,TGCATCAGTCAGACTGCATG`, passes those
checks but was made up for the tests and has never been in a PCR. Replace
it with the lab's.

## 2. Write the order sheets

```bash
cd scripts/pilot
nucle hardware order pilot.nsl --vendor twist --flank ACGTCAGTGCATGACTGACT,TGCATCAGTCAGACTGCATG --out order.fasta
nucle hardware order pilot.nsl --vendor idt --flank ACGTCAGTGCATGACTGACT,TGCATCAGTCAGACTGCATG --out order.csv
nucle bootstrap inspect order.fasta --flank ACGTCAGTGCATGACTGACT,TGCATCAGTCAGACTGCATG
```

Twist takes the FASTA as it is. For IDT, paste the CSV's Name and Sequence
columns into the template on the oPools page; 23 oligos fit its 50 pmol and
10 pmol scales. The last command reads the file table back from the sheet
alone (`pilot/pilot.txt v1 250 B hedges 16d+4p strands`), which is what
the reads must do later.

## 3. Dry run

`pilot_reads` sends every oligo on the sheet through a measured channel,
with that channel's spread of coverage between strands, and writes the
reads a sequencer would return once paired reads are merged:

```bash
cargo run --release -p nucle_ecc --example pilot_reads -- order.fasta reads.fastq array-lofi 30
nucle run check.nsl
```

Oligos recovered exactly, with no labels, on 26 November 2026 (seed 2026):

| Channel | 10 copies per oligo | 30 | 100 |
|---|---:|---:|---:|
| `array-hifi` (measured, ERR15127739) | 20 of 23 | 23 | 23 |
| `array-lofi` (measured, ERR15127744) | 17 of 23 | 23 | 22 |
| `twist-illumina` | 23 | 23 | 23 |
| `twist-nanopore` | 23 | 23 | 23 |

The measured array channels drop many copies outright (their log-normal
coverage spread), so at 10 copies some oligos get no reads. Any sequencing
run returns far more than 30 reads for each of 23 oligos; ask for the
pool's share of a run, not a whole run.

## 4. What to ask for

- **Synthesis:** a Twist oligo pool or IDT oPools pool of the 23 oligos on
  the sheet, up to 232 nt. Ask for a quote. `nucle cost pilot.txt --codec
  hedges -r 4` puts the payload at $0.58 at an assumed $0.00015 per
  nucleotide; a pool this small is billed at the vendor's minimum, which is
  far more.
- **Amplification and sequencing:** a core facility, with the primer pair
  from step 1. Reads must span the longest oligo, 232 nt: Illumina paired
  end with reads long enough to overlap and merge (2 x 150 overlaps by 68
  nt), or Nanopore. Ask for the reads as FASTQ, pairs merged.

## 5. Read it back

Save the reads as `reads.fastq` here and run the same scoring as the dry
run:

```bash
nucle run check.nsl
```

It reports the reads, the clusters found, the merge threshold fitted to the
reads' own noise, and how many ordered oligos came back exactly. Compare
with the dry run's row for the same number of reads per oligo.

## Not done yet

- Rebuilding `pilot.txt`'s bytes from the reads. `decode reads` shows each
  strand came back; no command yet turns sequencing reads into a pool that
  `nucle retrieve` can read.
- The order itself, and choosing the sequencing lab and its primer pair.
  Both need someone with a budget and a lab.
