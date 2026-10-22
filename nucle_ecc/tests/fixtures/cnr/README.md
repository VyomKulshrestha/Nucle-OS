# Clustered Nanopore Reads sample

The first 50 clusters of the Clustered Nanopore Reads (CNR) dataset, copied
unmodified from <https://github.com/microsoft/clustered-nanopore-reads-dataset>
under its MIT license (see `LICENSE` in this directory).

- `Centers.txt`: 50 reference strands, 110 nt each, synthesized by Twist Bioscience.
- `Clusters.txt`: the real Oxford Nanopore MinION reads of those strands, one block per
  reference, each block introduced by a line of `=` characters.

The full dataset holds 10,000 references and 269,709 reads. Cite:

> S. R. Srinivasavaradhan, S. Gopi, H. D. Pfister and S. Yekhanin, "Trellis BMA: Coded
> trace reconstruction on IDS channels for DNA storage," 2021 IEEE International
> Symposium on Information Theory (ISIT), pp. 2453-2458.

`nucle_ecc/tests/real_nanopore_reads.rs` scores consensus calling on this sample, with and
without its cluster labels. To score the full dataset, download it and run:

```bash
nucle reads Centers.txt Clusters.txt --blind
```

or, for the per-strategy comparison and timing:

```bash
cargo run --release -p nucle_ecc --example real_reads -- Centers.txt Clusters.txt
```
