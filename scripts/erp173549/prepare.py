"""Prepare the Illumina reads of Gimpel et al. 2026 (ENA ERP173549) for
NucleScript's `decode reads`.

Downloads read 1 of runs ERR15127739 (high-fidelity pool) and ERR15127744
(low-fidelity pool) from ENA and the 11,293 designed 129 nt references from
fml-ethz/dt4dds-benchmark_notebooks at a pinned commit (GPL-3.0, which is why
none of this data is in the repository). Each read is trimmed at the
Illumina adapter, assigned to the reference most of its 12-mers come from,
and at most 30 reads per reference are kept, in file order. The result is
the clustered layout of Microsoft's Clustered Nanopore Reads dataset:
`Centers.txt` (one reference per line) and `Clusters.txt` (each reference's
reads in a block under a line of `=`), one pair per pool.

    python scripts/erp173549/prepare.py [--out DIR]

then, from this directory,

    nucle run decode.nsl

Standard library only.
"""

import argparse
import gzip
import pathlib
import shutil
import sys
import urllib.request
from collections import Counter

REFERENCES_URL = (
    "https://raw.githubusercontent.com/fml-ethz/dt4dds-benchmark_notebooks/"
    "c45b36098bb3ded2d0c75d6b0e7468056841b7cb/07_PoolExperiments/final/combined/padded_indexed.fasta"
)
RUNS = {
    "hifi": "https://ftp.sra.ebi.ac.uk/vol1/fastq/ERR151/039/ERR15127739/ERR15127739_1.fastq.gz",
    "lofi": "https://ftp.sra.ebi.ac.uk/vol1/fastq/ERR151/044/ERR15127744/ERR15127744_1.fastq.gz",
}
# Read 1 starts at the insert and runs on into the adapter after it.
ADAPTER_FULL = "AGATCGGAAGAGCACACGTCT"
ADAPTER = ADAPTER_FULL[:13]
ADAPTER_TAIL = ADAPTER_FULL[-10:]
INSERT_LENGTH = 129
K = 12
READS_PER_REFERENCE = 30
# A read is assigned only if this many of its 12-mers point at the winner
# and strictly more than at any other reference.
MIN_VOTES = 20


def fetch(url, path):
    if path.exists():
        return path
    print(f"downloading {url}", file=sys.stderr)
    partial = path.with_suffix(path.suffix + ".part")
    with urllib.request.urlopen(url) as response, open(partial, "wb") as out:
        shutil.copyfileobj(response, out)
    partial.rename(path)
    return path


def read_references(path):
    references, current = [], []
    for line in path.read_text().splitlines():
        if line.startswith(">"):
            if current:
                references.append("".join(current))
            current = []
        elif line.strip():
            current.append(line.strip())
    if current:
        references.append("".join(current))
    return references


def kmer_index(references):
    index = {}
    for i, ref in enumerate(references):
        for j in range(len(ref) - K + 1):
            index.setdefault(ref[j : j + K], set()).add(i)
    return index


def trim(read):
    """The insert before the adapter, and how the adapter was found.

    The adapter was synthesized as part of each oligo, so in the
    low-fidelity pool it carries errors too. Failing an exact match, the
    adapter is placed by its last ten bases, then by its first six near the
    end of a 129 nt insert, and failing those the read is cut at 129.
    """
    cut = read.find(ADAPTER)
    if cut > 0:
        return read[:cut], "exact"
    tail = read.find(ADAPTER_TAIL, 110)
    if tail > 0:
        return read[: tail - (len(ADAPTER_FULL) - len(ADAPTER_TAIL))], "tail"
    head = read.find(ADAPTER[:6], 120, 140)
    if head > 0:
        return read[:head], "head"
    return read[:INSERT_LENGTH], "fixed"


def assign(read, index):
    votes = Counter()
    for j in range(len(read) - K + 1):
        for i in index.get(read[j : j + K], ()):
            votes[i] += 1
    top = votes.most_common(2)
    if not top or top[0][1] < MIN_VOTES or (len(top) == 2 and top[1][1] == top[0][1]):
        return None
    return top[0][0]


def prepare(fastq, references, index, out_dir):
    clusters = [[] for _ in references]
    seen = with_n = unassigned = 0
    how = Counter()
    with gzip.open(fastq, "rt") as f:
        for n, line in enumerate(f):
            if n % 4 != 1:
                continue
            seen += 1
            insert, found = trim(line.strip())
            how[found] += 1
            if "N" in insert:
                with_n += 1
                continue
            ref = assign(insert, index)
            if ref is None:
                unassigned += 1
            elif len(clusters[ref]) < READS_PER_REFERENCE:
                clusters[ref].append(insert)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "Centers.txt").write_text("\n".join(references) + "\n")
    with open(out_dir / "Clusters.txt", "w") as f:
        for reads in clusters:
            f.write("=" * 20 + "\n")
            for r in reads:
                f.write(r + "\n")
    kept = sum(len(c) for c in clusters)
    covered = sum(1 for c in clusters if c)
    print(
        f"{out_dir.name}: {seen} reads; adapter found {how['exact']} exactly, {how['tail']} by its tail, "
        f"{how['head']} by its head, {how['fixed']} not (cut at {INSERT_LENGTH}); {with_n} with an N, "
        f"{unassigned} not assigned; kept {kept} for {covered} of {len(references)} references"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    here = pathlib.Path(__file__).resolve().parent
    parser.add_argument("--out", type=pathlib.Path, default=here / "data", help="where the downloads and the two pools go")
    args = parser.parse_args()
    downloads = args.out / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)

    references = read_references(fetch(REFERENCES_URL, downloads / "padded_indexed.fasta"))
    index = kmer_index(references)
    for pool, url in RUNS.items():
        fastq = fetch(url, downloads / url.rsplit("/", 1)[1])
        prepare(fastq, references, index, args.out / pool)


if __name__ == "__main__":
    main()
