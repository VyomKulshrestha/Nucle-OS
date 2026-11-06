# Related Work

Where NucleScript sits among the languages and systems closest to it, what
it adds, and where those systems go further than it does. Every reference
below was checked against its DOI record at Crossref. The codec papers
NucleOS implements are covered separately in [references.md](references.md).

## Languages for designing DNA

These languages describe what DNA should be built.

- **Cello** compiles a circuit written in Verilog into the DNA sequence of a
  genetic circuit that computes it in living cells [1].
- **Eugene** specifies biological parts, devices and systems, and the
  rules constraining how they may be composed [2].
- **GSL**, the Genotype Specification Language, describes genome edits at a
  high level and compiles them into DNA assembly designs [3].
- **SBOL** is a shared data standard for exchanging such designs, not a
  programming language [4].

Their values are biological parts, and their checks ask whether a design is
well formed. NucleScript's values are stored files and pools of strands
read through noisy machines. The DNA it writes is whatever a codec makes of
a file, and its checks are about noise, cost and loss. The two families
meet only at the strand: a sequence literal in NucleScript is checked
against synthesis constraints (GC content, homopolymers, hairpins), the
kind of check these tools also make.

## Languages and systems for running lab work

- **BioCoder** writes biology protocols as programs, so a protocol can be
  stated without ambiguity and turned into instructions for people or
  machines [5].
- **Puddle** is an operating system and runtime for digital microfluidic
  devices, programmed from a high-level language. It corrects droplet
  errors while a program runs, by watching the device [6].

Puddle is the nearest relative in spirit: a full stack, an operating system
and a language over physical operations that can fail. It drives real
hardware, which NucleScript has not done (see below). NucleScript checks
before anything runs instead: whether an operation that costs money or
destroys material carries its confirmation, and whether a recovery target
can be met on the chosen sequencer. Puddle handles failure dynamically and
NucleScript plans for it statically. The two approaches could be combined.

## DNA storage systems and toolkits

- Bornholt et al. designed a DNA archival storage system with random
  access through PCR primers and encodings with tunable redundancy [7].
- Organick et al. stored and randomly accessed about 200 MB in DNA [8].
- Takahashi et al. automated the whole write, store and read cycle in one
  device [9].
- **Chamaeleo** [10], **MESA** [11], **DNA-Storalator** [12] and the **DNA
  Storage Toolkit** [13] are libraries and simulators. Chamaeleo
  implements and compares coding schemes. MESA scores sequences for
  synthesis and sequencing risk and simulates the errors. DNA-Storalator
  simulates the errors of the storage channel. The DNA Storage Toolkit
  combines a codec and a simulator end to end.

NucleOS covers similar ground to the toolkits: codecs including DNA
Fountain [14] and HEDGES [15], Reed-Solomon parity, consensus and
clustering of real reads, and a simulator with channels measured from real
pools. What none of them has is a language over those parts, with types
and effects a compiler checks. The systems papers [7, 8, 9] ran real
synthesis and sequencing, which NucleOS has not.

## Types for effects and physical quantities

- NucleScript's effects follow the effect systems of Lucassen and Gifford
  [16]: an expression's type says what it may do as well as what it
  returns, and a function's effect is the join of its body's.
- Koka carries effects in function types [17], as NucleScript's
  `Fn(...) -> T confirm hardware` does with a ceiling.
- A pool's error rate in its type (`Pool<Illumina, 0.35%>`) is closest to
  units of measure in types [18]: a physical quantity carried by the type
  checker, here the error rate of the channel a pool was read through,
  changed by consensus recovery.

## Checking a compiler by running programs

The evidence in [soundness.md](soundness.md) comes from runtime checks on
generated programs, in the tradition of QuickCheck's random testing [19]
and of Csmith, which found compiler bugs with randomly generated programs
[20]. Like Csmith, the generator found real defects: five, all now fixed.

## What NucleScript adds

As far as this survey found, the following are not in the work above:

- **Pools typed by the channel they were read through.** A pool's type
  names its sequencer profile and error rate. `consensus_vote` changes
  both, and functions can be generic over the profile.
- **Effects for synthesis, sequencing and deletion, checked at compile
  time.**
  - Each cost-bearing or destructive operation needs its own confirmation
    (`confirm hardware`, `confirm physical_key`).
  - A function's effect is what its body, and any closure it is given,
    can do.
  - Effect ceilings on function types are checked against every kind of
    effect a closure may have.
- **Redundancy and recovery checked against the chosen hardware before
  writing.**
  - An unreachable recovery target on Nanopore at low coverage is
    reported.
  - So is too little redundancy for data tagged critical.
  - A `retain: 10000y` requirement is checked against published decay
    kinetics for the pool's storage medium.
- **One program from file to order sheet and back from real reads.**
  - Storage operations run against the simulator.
  - `nucle hardware order` turns a program's stores into an oligo-pool
    order for Twist or IDT.
  - `decode reads` scores real sequencing reads from inside a program.

## Where it falls short

- No NucleScript program has driven a real synthesis or sequencing run.
  The order sheets have not been submitted, and the real-read decoding
  uses public datasets made by others.
- The type and effect rules are tested, not proved
  ([soundness.md](soundness.md)).
- The error rates in pool types come from published profiles and the
  measured channels of one study, and say nothing about a lab's own
  instruments.

## References

1. Nielsen, A. A. K., Der, B. S., Shin, J., et al. (2016). Genetic circuit design automation. *Science* 352(6281), aac7341. doi:[10.1126/science.aac7341](https://doi.org/10.1126/science.aac7341)
2. Bilitchenko, L., Liu, A., Cheung, S., et al. (2011). Eugene – A Domain Specific Language for Specifying and Constraining Synthetic Biological Parts, Devices, and Systems. *PLoS ONE* 6(4), e18882. doi:[10.1371/journal.pone.0018882](https://doi.org/10.1371/journal.pone.0018882)
3. Wilson, E. H., Sagawa, S., Weis, J. W., et al. (2016). Genotype Specification Language. *ACS Synthetic Biology* 5(6), 471–478. doi:[10.1021/acssynbio.5b00194](https://doi.org/10.1021/acssynbio.5b00194)
4. Galdzicki, M., Clancy, K. P., Oberortner, E., et al. (2014). The Synthetic Biology Open Language (SBOL) provides a community standard for communicating designs in synthetic biology. *Nature Biotechnology* 32(6), 545–550. doi:[10.1038/nbt.2891](https://doi.org/10.1038/nbt.2891)
5. Ananthanarayanan, V., & Thies, W. (2010). BioCoder: A programming language for standardizing and automating biology protocols. *Journal of Biological Engineering* 4, 13. doi:[10.1186/1754-1611-4-13](https://doi.org/10.1186/1754-1611-4-13)
6. Willsey, M., Stephenson, A. P., Takahashi, C., et al. (2019). Puddle: A Dynamic, Error-Correcting, Full-Stack Microfluidics Platform. *ASPLOS '19*, 183–197. doi:[10.1145/3297858.3304027](https://doi.org/10.1145/3297858.3304027)
7. Bornholt, J., Lopez, R., Carmean, D. M., et al. (2016). A DNA-Based Archival Storage System. *ASPLOS '16*, 637–649. doi:[10.1145/2872362.2872397](https://doi.org/10.1145/2872362.2872397)
8. Organick, L., Ang, S. D., Chen, Y.-J., et al. (2018). Random access in large-scale DNA data storage. *Nature Biotechnology* 36(3), 242–248. doi:[10.1038/nbt.4079](https://doi.org/10.1038/nbt.4079)
9. Takahashi, C. N., Nguyen, B. H., Strauss, K., & Ceze, L. (2019). Demonstration of End-to-End Automation of DNA Data Storage. *Scientific Reports* 9, 4998. doi:[10.1038/s41598-019-41228-8](https://doi.org/10.1038/s41598-019-41228-8)
10. Ping, Z., Zhang, H., Chen, S., et al. (2020). Chamaeleo: a robust library for DNA storage coding schemes. *bioRxiv*. doi:[10.1101/2020.01.02.892588](https://doi.org/10.1101/2020.01.02.892588)
11. Schwarz, M., Welzel, M., Kabdullayeva, T., et al. (2020). MESA: automated assessment of synthetic DNA fragments and simulation of DNA synthesis, storage, sequencing and PCR errors. *Bioinformatics* 36(11), 3322–3326. doi:[10.1093/bioinformatics/btaa140](https://doi.org/10.1093/bioinformatics/btaa140)
12. Chaykin, G., Sabary, O., Furman, N., et al. (2025). DNA-Storalator: a computational simulator for DNA data storage. *BMC Bioinformatics* 26, 204. doi:[10.1186/s12859-025-06222-0](https://doi.org/10.1186/s12859-025-06222-0)
13. Sharma, P., Yipeng, G. G., Gao, B., et al. (2024). DNA Storage Toolkit: A Modular End-to-End DNA Data Storage Codec and Simulator. *ISPASS 2024*, 144–155. doi:[10.1109/ISPASS61541.2024.00023](https://doi.org/10.1109/ISPASS61541.2024.00023)
14. Erlich, Y., & Zielinski, D. (2017). DNA Fountain enables a robust and efficient storage architecture. *Science* 355(6328), 950–954. doi:[10.1126/science.aaj2038](https://doi.org/10.1126/science.aaj2038)
15. Press, W. H., Hawkins, J. A., Jones, S. K., et al. (2020). HEDGES error-correcting code for DNA storage corrects indels and allows sequence constraints. *PNAS* 117(31), 18489–18496. doi:[10.1073/pnas.2004821117](https://doi.org/10.1073/pnas.2004821117)
16. Lucassen, J. M., & Gifford, D. K. (1988). Polymorphic effect systems. *POPL '88*, 47–57. doi:[10.1145/73560.73564](https://doi.org/10.1145/73560.73564)
17. Leijen, D. (2014). Koka: Programming with Row Polymorphic Effect Types. *EPTCS* 153, 100–126. doi:[10.4204/EPTCS.153.8](https://doi.org/10.4204/EPTCS.153.8)
18. Kennedy, A. J. (1997). Relational parametricity and units of measure. *POPL '97*, 442–455. doi:[10.1145/263699.263761](https://doi.org/10.1145/263699.263761)
19. Claessen, K., & Hughes, J. (2000). QuickCheck: a lightweight tool for random testing of Haskell programs. *ICFP '00*, 268–279. doi:[10.1145/351240.351266](https://doi.org/10.1145/351240.351266)
20. Yang, X., Chen, Y., Eide, E., & Regehr, J. (2011). Finding and understanding bugs in C compilers. *PLDI '11*, 283–294. doi:[10.1145/1993498.1993532](https://doi.org/10.1145/1993498.1993532)
