//! # Consensus Sequencing Engine
//!
//! When the same DNA strand is sequenced multiple times (coverage depth),
//! each copy has independent errors. By aligning multiple noisy copies
//! and taking a majority vote at each position, the consensus sequence
//! is dramatically more accurate than any individual read.
//!
//! Typical coverage: 5–20× per strand in DNA storage systems.
//!
//! Consensus is built with **partial-order alignment (POA)**: reads are
//! folded one at a time into a single growing DAG (a [`PoaGraph`]) rather
//! than each read being realigned pairwise against one arbitrarily-picked
//! "reference" read. That distinction matters once a single read carries
//! *several* indels (routine under Nanopore-grade noise on a 150+nt
//! strand): realigning read after read against one noisy anchor lets that
//! anchor's own errors -- and each read's individual drift -- corrupt every
//! subsequent vote, whereas folding every read into a shared graph lets
//! independent reads that agree on a base or an indel reinforce the same
//! graph node regardless of which read happened to be processed first.
//!
//! A single pass still seeds the graph's column numbering from one raw
//! (possibly indel-bearing) read, which can leave a small residual error
//! even after voting otherwise corrects everything else. So
//! [`build_consensus`] polishes: it reseeds a fresh graph from the
//! previous pass's own (already-corrected) result and re-folds every
//! read, repeating to a fixed point (capped) -- the same multi-round
//! approach real long-read polishers (Racon, Medaka) use, simplified for
//! DNA storage's much shorter strands. The reseed is unweighted (the
//! backbone contributes no vote of its own) so a read doesn't get counted
//! twice just because its content happens to already match the backbone.

use nucle_codec::base::{DnaStrand, Nucleotide};

/// Result of consensus calling for a single strand.
#[derive(Debug, Clone)]
pub struct ConsensusResult {
    /// The consensus sequence (majority-voted).
    pub sequence: DnaStrand,
    /// Number of reads that contributed to this consensus.
    pub coverage: usize,
    /// Per-position confidence (fraction of reads agreeing with consensus).
    pub confidence: Vec<f64>,
    /// Average confidence across all positions.
    pub avg_confidence: f64,
}

const MATCH: i64 = 2;
const MISMATCH: i64 = -1;
// Must satisfy 2*GAP + MATCH < 2*MISMATCH (i.e. GAP < MISMATCH - MATCH/2 = -2)
// or a run of plain substitutions ties exactly with an alternative alignment
// that opens two gaps to "recover" one match -- and on a tie, a real DAG can
// have many equally-scored paths, so accepting mismatches directly must be
// the strictly better choice whenever there is no genuine indel to correct.
const GAP: i64 = -4;

/// A partial-order alignment graph: a DAG of nucleotide nodes built up
/// incrementally as reads are folded in.
///
/// Edges (not nodes) carry the vote weight -- how many reads' realized
/// path took that specific transition. That distinction matters for
/// insertions: an inserted base is a real node like any other, but if only
/// a minority of reads have it, the *direct* edge bypassing it (taken by
/// the majority that don't) must be able to outscore the two weak edges
/// of the detour through it. A "maximize total node count visited"
/// objective can never express that (every extra node visited only adds
/// to the total, so the algorithm would always prefer to detour through
/// even a single-read insertion); maximizing total edge weight naturally
/// prefers whichever route more reads actually took.
struct PoaGraph {
    bases: Vec<Nucleotide>,
    /// Predecessor node indices for each node. An empty list means the
    /// node's only "predecessor" is the graph's implicit start.
    preds: Vec<Vec<usize>>,
    /// Parallel to `preds`: weight of the edge from `preds[node][k]` to
    /// `node`, i.e. how many reads' path took that specific transition.
    pred_weights: Vec<Vec<usize>>,
    /// Weight of the implicit "start -> node" edge: how many reads' path
    /// began directly at this node with nothing skipped before it.
    start_weight: Vec<usize>,
    /// How many reads' own alignment *ended* at this node -- the mirror
    /// of `start_weight` at the other end. Needed for the same reason:
    /// a trailing insertion that only one read has still gives that
    /// insertion node a positive edge weight, and "maximize total edge
    /// weight along the path" would always prefer including any node
    /// with a positive edge over stopping one node earlier, the same
    /// flaw `start_weight` existed to fix for the beginning of the
    /// sequence. Picking the final node by which one most reads actually
    /// ended on (not by graph-theoretic sink-ness, since a stray trailing
    /// insertion gives the true final node a successor too) is what lets
    /// a majority "stop here" outvote a minority "keep going".
    terminal_weight: Vec<usize>,
    /// A stable reference-column identity, assigned once at node creation
    /// and never revised: `(backbone_position, insertion_depth)`.
    /// Mismatch alternatives at the same backbone position share their
    /// sibling's column exactly; an inserted node gets its predecessor's
    /// backbone position with `insertion_depth + 1`, so it can never
    /// collide with a genuine backbone/mismatch node (always depth 0) no
    /// matter what the backbone position number happens to be -- an
    /// earlier version used `column + 1` in the same namespace as backbone
    /// positions themselves and could alias an insertion onto a real
    /// downstream position, occasionally even creating a cycle. Backbone
    /// positions are 1-indexed here so `(0, k)` is free to mean "inserted
    /// before the very first backbone position."
    ///
    /// Node *predecessor sets* are not a safe identity key on their own:
    /// they can grow over the graph's life as later reads add edges (see
    /// `record_transition`), so two nodes from genuinely different
    /// reference positions can end up with coincidentally-equal
    /// predecessor sets after enough reads. Column identity can't drift
    /// like that, since it's fixed at birth.
    column: Vec<(usize, usize)>,
}

impl PoaGraph {
    /// Seed the graph from a single read that is itself one of the votes:
    /// a plain linear chain with weight 1 throughout, representing that
    /// read's own contribution.
    fn seed(read: &[Nucleotide]) -> Self {
        Self::seed_with_weight(read, 1)
    }

    /// Seed the graph from a backbone sequence that is *not* itself a
    /// vote -- e.g. a previous polishing pass's own result, used only to
    /// fix the column numbering for a refinement pass where every real
    /// read (including whichever one seeded the first pass) gets folded
    /// in explicitly. Using `seed` here would double-count: its baked-in
    /// weight of 1 plus every read's own fold would let some positions
    /// out-vote their true coverage.
    fn seed_unweighted(read: &[Nucleotide]) -> Self {
        Self::seed_with_weight(read, 0)
    }

    fn seed_with_weight(read: &[Nucleotide], weight: usize) -> Self {
        let n = read.len();
        let preds: Vec<Vec<usize>> = (0..n).map(|i| if i == 0 { Vec::new() } else { vec![i - 1] }).collect();
        let pred_weights = preds.iter().map(|p| vec![weight; p.len()]).collect();
        let start_weight = (0..n).map(|i| if i == 0 { weight } else { 0 }).collect();
        let terminal_weight = (0..n).map(|i| if i + 1 == n { weight } else { 0 }).collect();
        let column = (0..n).map(|i| (i + 1, 0)).collect();
        PoaGraph { bases: read.to_vec(), preds, pred_weights, start_weight, terminal_weight, column }
    }

    fn len(&self) -> usize {
        self.bases.len()
    }

    /// An existing node at the given reference column with a given base
    /// that is safe to reuse as the successor of `after` (if given)
    /// without creating a cycle, if one exists.
    ///
    /// Column identity is a strong but not absolute guarantee of "same
    /// reference slot": a graph built by folding in enough reads can
    /// still, rarely, let an edge get established between two nodes that
    /// share a column (e.g. two insertion alternates at the same slot)
    /// when one was reached via a path through the other in some earlier
    /// read. Reusing such a node as `after`'s successor would close a
    /// cycle, so this is checked before every reuse rather than trusting
    /// column equality alone.
    fn find_reusable_at_column(&self, column: (usize, usize), base: Nucleotide, after: Option<usize>) -> Option<usize> {
        (0..self.len())
            .filter(|&k| self.column[k] == column && self.bases[k] == base)
            .find(|&k| after.is_none_or(|p| !self.reaches(k, p)))
    }

    /// Whether `from` is `to` itself or can already reach `to` by
    /// following existing predecessor edges backward from `to`. If so,
    /// adding a new edge `to -> from` would close a cycle.
    fn reaches(&self, from: usize, to: usize) -> bool {
        if from == to {
            return true;
        }
        let mut stack = vec![to];
        let mut seen = vec![false; self.len()];
        while let Some(n) = stack.pop() {
            if seen[n] {
                continue;
            }
            seen[n] = true;
            for &p in &self.preds[n] {
                if p == from {
                    return true;
                }
                stack.push(p);
            }
        }
        false
    }

    fn successors_of(&self, node: usize) -> Vec<usize> {
        (0..self.len()).filter(|&k| self.preds[k].contains(&node)).collect()
    }

    /// Add `pred` as a predecessor of `node` (`pred -> node`), unless it's
    /// already present or doing so would create a cycle. This is the sole
    /// place any edge is ever added, so it's the one place a cycle check
    /// needs to live to cover every caller -- including ones that don't
    /// look like they need it: the "reuse an existing sibling" call sites
    /// already filter out cycle-creating candidates themselves, but the
    /// *exact-match* fast path (a read's base already equals an existing
    /// node's base, so no search happens at all) does not, and a read can
    /// legitimately visit the same node twice in its own realigned path
    /// under heavy compounding indel noise. Refusing silently here rather
    /// than trusting every caller to have already checked is what makes
    /// the graph's acyclic invariant actually hold.
    fn add_pred_if_missing(&mut self, node: usize, pred: usize) {
        if !self.preds[node].contains(&pred) && !self.reaches(node, pred) {
            self.preds[node].push(pred);
            self.pred_weights[node].push(0);
        }
    }

    /// Record that a read's realized path transitioned from `from` (or
    /// the implicit start, if `None`) into `to`, incrementing that
    /// specific edge's weight -- unless `add_pred_if_missing` refused the
    /// edge as cycle-forming, in which case this one read's vote for this
    /// specific transition is dropped rather than corrupting the graph.
    fn record_transition(&mut self, from: Option<usize>, to: usize) {
        match from {
            None => self.start_weight[to] += 1,
            Some(p) => {
                self.add_pred_if_missing(to, p);
                if let Some(idx) = self.preds[to].iter().position(|&x| x == p) {
                    self.pred_weights[to][idx] += 1;
                }
            }
        }
    }

    /// Whether `node` sits at a column that can legitimately be reached
    /// directly from the graph's implicit start with nothing real skipped
    /// -- the genuine first backbone position (and its mismatch siblings,
    /// which share its column exactly), or a leading insertion before it.
    ///
    /// This must be checked independently of whether `node` also has real
    /// predecessors: a node can gain a real predecessor later (e.g. the
    /// first backbone position becomes reachable via a leading-insertion
    /// node once some read has one) without that ever invalidating the
    /// direct "start" route other reads still take. Treating "has
    /// predecessors" and "reachable from start" as mutually exclusive (the
    /// original design) silently drops whichever route wasn't checked --
    /// here, that dropped an overwhelming direct-start majority in favor
    /// of a single stray leading insertion.
    fn reachable_from_start(&self, node: usize) -> bool {
        self.column[node] == (0, 0) || self.column[node] == (1, 0)
    }

    /// A topological order over the graph's nodes (Kahn's algorithm).
    ///
    /// Node indices are assigned in creation order, but that is *not* a
    /// valid topological order by itself: splicing an alternate node in as
    /// a predecessor of an already-existing (lower-indexed) node -- which
    /// `align_and_merge` does whenever a later read's path rejoins the
    /// graph after a mismatch or insertion -- can make a higher-indexed
    /// node a predecessor of a lower-indexed one. Both the per-read
    /// alignment DP and the final heaviest-path extraction need every
    /// node's predecessors processed before the node itself, so both
    /// compute this fresh rather than assuming index order.
    fn topological_order(&self) -> Vec<usize> {
        let n = self.len();
        let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (node, preds) in self.preds.iter().enumerate() {
            for &p in preds {
                successors[p].push(node);
            }
        }
        let mut in_degree: Vec<usize> = self.preds.iter().map(|p| p.len()).collect();
        let mut queue: std::collections::VecDeque<usize> =
            (0..n).filter(|&v| in_degree[v] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(u) = queue.pop_front() {
            order.push(u);
            for &v in &successors[u] {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }
        debug_assert_eq!(order.len(), n, "PoaGraph must always remain acyclic");
        order
    }

    /// Align `read` against the graph built so far (Needleman-Wunsch
    /// generalized to a DAG: the "previous column" of the classic
    /// recurrence becomes "any predecessor of this node") and fold it in,
    /// creating new nodes for insertions and true mismatches while
    /// reinforcing existing nodes wherever this read agrees with them.
    fn align_and_merge(&mut self, read: &[Nucleotide]) {
        let n = self.len();
        let m = read.len();
        if m == 0 {
            return;
        }

        #[derive(Clone, Copy)]
        enum Move {
            Diag(usize),
            Del(usize),
            Ins,
            StartDiag,
            StartDel,
        }

        // dp[i][j]: j == 0 is the graph's implicit start; j in 1..=n is
        // real node (j - 1). Columns must be filled in topological order,
        // not raw index order -- see `topological_order`'s doc comment.
        let topo = self.topological_order();
        let mut dp = vec![vec![i64::MIN / 2; n + 1]; m + 1];
        let mut bp: Vec<Vec<Option<Move>>> = vec![vec![None; n + 1]; m + 1];
        dp[0][0] = 0;

        for &node in &topo {
            let j = node + 1;
            let mut best: Option<(i64, Move)> = None;
            if self.reachable_from_start(node) {
                best = Some((dp[0][0] + GAP, Move::StartDel));
            }
            for &p in &self.preds[node] {
                let v = dp[0][p + 1] + GAP;
                if best.is_none_or(|(b, _)| v > b) {
                    best = Some((v, Move::Del(p)));
                }
            }
            let (score, mv) = best.expect("every node is reachable from start or has a real predecessor");
            dp[0][j] = score;
            bp[0][j] = Some(mv);
        }

        for i in 1..=m {
            dp[i][0] = dp[i - 1][0] + GAP;
            bp[i][0] = Some(Move::Ins);

            for &node in &topo {
                let j = node + 1;
                let match_score = if read[i - 1] == self.bases[node] { MATCH } else { MISMATCH };
                let preds = &self.preds[node];
                let startable = self.reachable_from_start(node);

                let mut diag_best: Option<(i64, Move)> = startable
                    .then(|| (dp[i - 1][0] + match_score, Move::StartDiag));
                for &p in preds {
                    let v = dp[i - 1][p + 1] + match_score;
                    if diag_best.is_none_or(|(b, _)| v > b) {
                        diag_best = Some((v, Move::Diag(p)));
                    }
                }

                let mut del_best: Option<(i64, Move)> = startable
                    .then(|| (dp[i][0] + GAP, Move::StartDel));
                for &p in preds {
                    let v = dp[i][p + 1] + GAP;
                    if del_best.is_none_or(|(b, _)| v > b) {
                        del_best = Some((v, Move::Del(p)));
                    }
                }

                let ins_score = dp[i - 1][j] + GAP;

                let (best_score, best_move) = [diag_best.unwrap(), del_best.unwrap(), (ins_score, Move::Ins)]
                    .into_iter()
                    .max_by_key(|&(v, _)| v)
                    .unwrap();

                dp[i][j] = best_score;
                bp[i][j] = Some(best_move);
            }
        }

        let j_end = (0..=n).max_by_key(|&j| dp[m][j]).unwrap();

        enum Step {
            Use(usize, Nucleotide),
            Skip,
            Insert(Nucleotide),
        }

        let mut steps_rev = Vec::new();
        let (mut i, mut j) = (m, j_end);
        while i > 0 || j > 0 {
            match bp[i][j].expect("every cell on a traced-back path has a backpointer") {
                Move::Diag(p) => {
                    steps_rev.push(Step::Use(j - 1, read[i - 1]));
                    i -= 1;
                    j = p + 1;
                }
                Move::StartDiag => {
                    steps_rev.push(Step::Use(j - 1, read[i - 1]));
                    i -= 1;
                    j = 0;
                }
                Move::Del(p) => {
                    steps_rev.push(Step::Skip);
                    j = p + 1;
                }
                Move::StartDel => {
                    steps_rev.push(Step::Skip);
                    j = 0;
                }
                Move::Ins => {
                    steps_rev.push(Step::Insert(read[i - 1]));
                    i -= 1;
                }
            }
        }
        steps_rev.reverse();

        let mut prev_node: Option<usize> = None;
        for step in steps_rev {
            match step {
                Step::Skip => {}
                Step::Use(node, base) => {
                    let actual = if self.bases[node] == base {
                        node
                    } else {
                        let column = self.column[node];
                        if let Some(sibling) = self.find_reusable_at_column(column, base, prev_node) {
                            sibling
                        } else {
                            let preds_here = self.preds[node].clone();
                            let successors = self.successors_of(node);
                            let new_id = self.bases.len();
                            self.bases.push(base);
                            self.pred_weights.push(vec![0; preds_here.len()]);
                            self.preds.push(preds_here);
                            self.start_weight.push(0);
                            self.terminal_weight.push(0);
                            self.column.push(column);
                            for succ in successors {
                                self.add_pred_if_missing(succ, new_id);
                            }
                            new_id
                        }
                    };
                    self.record_transition(prev_node, actual);
                    prev_node = Some(actual);
                }
                Step::Insert(base) => {
                    let column = match prev_node {
                        Some(p) => (self.column[p].0, self.column[p].1 + 1),
                        None => (0, 0),
                    };
                    let actual = if let Some(sibling) = self.find_reusable_at_column(column, base, prev_node) {
                        sibling
                    } else {
                        let preds_here: Vec<usize> = prev_node.into_iter().collect();
                        let new_id = self.bases.len();
                        self.bases.push(base);
                        self.pred_weights.push(vec![0; preds_here.len()]);
                        self.preds.push(preds_here);
                        self.start_weight.push(0);
                        self.terminal_weight.push(0);
                        self.column.push(column);
                        new_id
                    };
                    self.record_transition(prev_node, actual);
                    prev_node = Some(actual);
                }
            }
        }
        self.terminal_weight[prev_node.expect("a non-empty read always consumes at least one base via Use or Insert")] += 1;
    }

    /// The highest-total-edge-weight path through the graph, start to
    /// sink -- the consensus sequence. Returns each node on that path
    /// alongside the weight of the specific edge used to reach it (how
    /// many reads' realized path took that transition), which doubles as
    /// that position's vote count for confidence.
    fn heaviest_path(&self) -> Vec<(usize, usize)> {
        let n = self.len();
        if n == 0 {
            return Vec::new();
        }
        let topo = self.topological_order();

        let mut best_score = vec![0i64; n];
        let mut best_pred: Vec<Option<usize>> = vec![None; n];
        for &j in &topo {
            // `None` candidates use the implicit start; `Some(p)` candidates
            // are real predecessors. Both are always in play together --
            // see `reachable_from_start`'s doc comment for why treating
            // them as mutually exclusive was the bug.
            let mut best: Option<(i64, Option<usize>)> = self.reachable_from_start(j)
                .then(|| (self.start_weight[j] as i64, None));
            for (&p, &w) in self.preds[j].iter().zip(self.pred_weights[j].iter()) {
                let score = best_score[p] + w as i64;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, Some(p)));
                }
            }
            let (score, pred) = best.expect("every node is reachable from start or has a real predecessor");
            best_score[j] = score;
            best_pred[j] = pred;
        }

        // The end is picked by which node most reads' own alignment
        // actually finished on (`terminal_weight`), not by graph-theoretic
        // sink-ness: a single stray trailing insertion gives the true
        // final node a successor too, and "maximize total edge weight"
        // would always prefer detouring through any node with a positive
        // edge over stopping short of it -- see `terminal_weight`'s doc
        // comment.
        let end = (0..n)
            .max_by_key(|&j| best_score[j] + self.terminal_weight[j] as i64)
            .unwrap();

        let mut path = Vec::new();
        let mut cur = Some(end);
        while let Some(node) = cur {
            let weight = match best_pred[node] {
                None => self.start_weight[node],
                Some(pred) => {
                    let idx = self.preds[node].iter().position(|&x| x == pred).unwrap();
                    self.pred_weights[node][idx]
                }
            };
            path.push((node, weight));
            cur = best_pred[node];
        }
        path.reverse();
        path
    }
}

/// Build a consensus sequence from multiple noisy copies of the same strand.
///
/// # Algorithm
///
/// 1. Seed a [`PoaGraph`] from the read whose length is closest to the
///    group's median (a reasonable "typical" starting point).
/// 2. Fold every other read into the graph one at a time: align it
///    against the graph built so far, reinforcing nodes it agrees with
///    and creating new nodes for indels or true mismatches (reusing an
///    existing alternate node if a previous read already introduced the
///    same one, so independent reads converge instead of fragmenting).
/// 3. The consensus is the highest-total-support path through the
///    resulting graph, start to end.
/// 4. Confidence per consensus position is that node's support divided
///    by the group's total read count.
pub fn build_consensus(reads: &[DnaStrand]) -> Option<ConsensusResult> {
    if reads.is_empty() {
        return None;
    }

    if reads.len() == 1 {
        let len = reads[0].len();
        return Some(ConsensusResult {
            sequence: reads[0].clone(),
            coverage: 1,
            confidence: vec![1.0; len],
            avg_confidence: 1.0,
        });
    }

    let mut lengths: Vec<usize> = reads.iter().map(|r| r.len()).collect();
    lengths.sort_unstable();
    let median_len = lengths[lengths.len() / 2];
    let seed_idx = reads.iter()
        .enumerate()
        .min_by_key(|(_, r)| (r.len() as i64 - median_len as i64).abs())
        .map(|(i, _)| i)
        .unwrap();

    let seed_bases = reads[seed_idx].bases();
    if seed_bases.is_empty() {
        return Some(ConsensusResult {
            sequence: DnaStrand::new(Vec::new()),
            coverage: reads.len(),
            confidence: Vec::new(),
            avg_confidence: 0.0,
        });
    }

    // First pass: fold every other read into a graph seeded from one raw
    // (possibly error-prone) read.
    let mut graph = PoaGraph::seed(seed_bases);
    for (i, read) in reads.iter().enumerate() {
        if i == seed_idx {
            continue;
        }
        graph.align_and_merge(read.bases());
    }
    let mut current: Vec<Nucleotide> = graph.heaviest_path().iter().map(|&(n, _)| graph.bases[n]).collect();

    // Polishing passes: rebuild from scratch seeded on the previous pass's
    // own result (unweighted -- it's not itself a vote) and fold in
    // *every* read, including the one that seeded pass one. A raw read's
    // own indels don't just cost that one read a few wrong votes -- they
    // reshape the graph's own column numbering for everything downstream
    // of them, so an occasional residual error can survive voting even
    // after the graph otherwise correctly outvotes every substitution.
    // Seeding the next pass from a result that has already been
    // majority-corrected removes that bias, the same way real long-read
    // consensus tools (Racon, Medaka) run more than one polishing round
    // rather than trusting a single raw read as ground truth for column
    // identity. Iterate to a fixed point (capped, as a backstop against
    // two columns trading places forever rather than settling) instead of
    // a single fixed extra pass, since one round can still leave a
    // smaller residual for the next round to fix.
    const MAX_POLISH_ROUNDS: usize = 5;
    let mut graph = PoaGraph::seed_unweighted(&current);
    for _ in 0..MAX_POLISH_ROUNDS {
        graph = PoaGraph::seed_unweighted(&current);
        for read in reads {
            graph.align_and_merge(read.bases());
        }
        let next: Vec<Nucleotide> = graph.heaviest_path().iter().map(|&(n, _)| graph.bases[n]).collect();
        let converged = next == current;
        current = next;
        if converged {
            break;
        }
    }

    let path = graph.heaviest_path();
    let consensus_bases: Vec<Nucleotide> = path.iter().map(|&(n, _)| graph.bases[n]).collect();
    let confidence: Vec<f64> = path.iter().map(|&(_, w)| w as f64 / reads.len() as f64).collect();
    let avg_conf = if confidence.is_empty() {
        0.0
    } else {
        confidence.iter().sum::<f64>() / confidence.len() as f64
    };

    Some(ConsensusResult {
        sequence: DnaStrand::new(consensus_bases),
        coverage: reads.len(),
        confidence,
        avg_confidence: avg_conf,
    })
}

/// Build consensus for groups of reads, where each group corresponds
/// to copies of the same original strand.
///
/// `read_groups`: each inner Vec contains multiple noisy copies of one strand.
/// Returns one consensus sequence per group.
pub fn build_consensus_batch(read_groups: &[Vec<DnaStrand>]) -> Vec<Option<ConsensusResult>> {
    read_groups.iter().map(|group| build_consensus(group)).collect()
}

/// Determine if more coverage is needed based on confidence threshold.
///
/// Returns the positions where confidence is below the threshold.
pub fn low_confidence_positions(result: &ConsensusResult, threshold: f64) -> Vec<usize> {
    result.confidence.iter()
        .enumerate()
        .filter(|(_, &conf)| conf < threshold)
        .map(|(pos, _)| pos)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: a single stray insertion at the very start or end
    /// of a strand, outvoted 9-to-1 by clean reads, must not survive into
    /// the consensus. This is the realistic Illumina case (indel rate
    /// ~0.01%, so on any given strand at most one read out of many is
    /// likely to have one at all) and it broke twice during development --
    /// once because a node gaining a real predecessor made the code think
    /// "reachable from start" and "has predecessors" were mutually
    /// exclusive (silently dropping the 9-vote direct-start edge), and
    /// again at the trailing end for the same reason in reverse (picking
    /// the path's end by raw sink-ness let a 1-vote trailing insertion
    /// outscore stopping at the true 9-vote final node, since include-one-
    /// more-node-with-any-positive-weight always looks "heavier").
    /// Regression guard for a graph-corruption crash found while fuzzing
    /// realistic-rate Nanopore noise (substitution 3%, insertion 2%,
    /// deletion 2% per base -- matching `HardwareProfile::OxfordNanopore`)
    /// at 50x coverage: under heavy compounding indels, a read's own
    /// realigned path could revisit the same existing node twice via the
    /// exact-base-match fast path (which, unlike the sibling-reuse paths,
    /// had no cycle check at all), wiring that node up as its own
    /// predecessor and crashing the `PoaGraph must always remain acyclic`
    /// debug assertion. `PoaGraph::add_pred_if_missing` now refuses any
    /// edge that would close a cycle regardless of caller, so this just
    /// needs to not panic -- output correctness at this noise density is
    /// the separate, already-documented open question about needing true
    /// multi-read partial-order alignment refinement.
    #[test]
    fn test_high_coverage_realistic_nanopore_fuzz_does_not_crash() {
        struct Xorshift(u64);
        impl Xorshift {
            fn next(&mut self) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0
            }
            fn frac(&mut self) -> f64 {
                (self.next() % 1_000_000) as f64 / 1_000_000.0
            }
            fn range(&mut self, n: usize) -> usize {
                (self.next() % n as u64) as usize
            }
        }
        let bases_alphabet = [Nucleotide::A, Nucleotide::C, Nucleotide::G, Nucleotide::T];
        let original = "TATATATATATATATATATATATGACAGATCTGTAGTAGATGTATCAGTACTAGACACAGTATCAGTACTAGTGATAGTACTATGAGCAGTGAGAGTAGATGTACGAGACGATGTATCAGACTGATGAGCAGAGCTAGACATAGTACATGTAGA";
        let orig_bases: Vec<Nucleotide> = DnaStrand::from_str(original).unwrap().bases().to_vec();

        for seed in 0u64..20 {
            let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x1234_5678));
            let mut reads = Vec::new();
            for _ in 0..50 {
                let mut bases = Vec::with_capacity(orig_bases.len());
                for &b in &orig_bases {
                    if rng.frac() < 0.02 {
                        continue; // deletion
                    }
                    if rng.frac() < 0.03 {
                        bases.push(bases_alphabet[rng.range(4)]); // substitution
                    } else {
                        bases.push(b);
                    }
                    if rng.frac() < 0.02 {
                        bases.push(bases_alphabet[rng.range(4)]); // insertion
                    }
                }
                reads.push(DnaStrand::new(bases));
            }
            let _ = build_consensus(&reads);
        }
    }

    #[test]
    fn test_boundary_insertion_outvoted_by_clean_majority() {
        let original = "ATCGATCGTACGATCGGATCCATGACTGATCGTACGGATCAGT";
        let clean = DnaStrand::from_str(original).unwrap();

        #[derive(Clone, Copy)]
        enum Edit {
            Delete,
            Insert(Nucleotide),
        }
        let edits = [
            Edit::Delete,
            Edit::Insert(Nucleotide::A),
            Edit::Insert(Nucleotide::C),
            Edit::Insert(Nucleotide::G),
            Edit::Insert(Nucleotide::T),
        ];

        for pos in 0..clean.len() {
            for edit in edits {
                let mut reads: Vec<DnaStrand> = (0..9).map(|_| clean.clone()).collect();
                let mut bases = clean.bases().to_vec();
                match edit {
                    Edit::Delete => {
                        bases.remove(pos);
                    }
                    Edit::Insert(base) => bases.insert(pos, base),
                }
                reads.push(DnaStrand::new(bases));

                let result = build_consensus(&reads).unwrap();
                assert_eq!(
                    result.sequence.to_string(), original,
                    "a single stray edit at position {pos}, outvoted 9-to-1, should not survive consensus"
                );
            }
        }
    }

    #[test]
    fn test_single_read_consensus() {
        let read = DnaStrand::from_str("ATCG").unwrap();
        let result = build_consensus(&[read.clone()]).unwrap();

        assert_eq!(result.sequence, read);
        assert_eq!(result.coverage, 1);
        assert_eq!(result.avg_confidence, 1.0);
    }

    #[test]
    fn test_perfect_consensus() {
        // All reads identical — perfect consensus
        let reads = vec![
            DnaStrand::from_str("ATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATCG").unwrap(),
        ];

        let result = build_consensus(&reads).unwrap();
        assert_eq!(result.sequence.to_string(), "ATCGATCG");
        assert_eq!(result.coverage, 3);
        assert_eq!(result.avg_confidence, 1.0);
    }

    #[test]
    fn test_majority_voting() {
        // 2 out of 3 agree at each position
        let reads = vec![
            DnaStrand::from_str("ATCG").unwrap(), // Original
            DnaStrand::from_str("ATCG").unwrap(), // Original
            DnaStrand::from_str("GCAT").unwrap(), // All different
        ];

        let result = build_consensus(&reads).unwrap();
        // Majority at each position should match the original
        assert_eq!(result.sequence.to_string(), "ATCG");
        // Confidence should be 2/3 at each position
        for &conf in &result.confidence {
            assert!((conf - 2.0 / 3.0).abs() < 0.01);
        }
    }

    #[test]
    fn test_empty_reads() {
        assert!(build_consensus(&[]).is_none());
    }

    #[test]
    fn test_low_confidence_detection() {
        let result = ConsensusResult {
            sequence: DnaStrand::from_str("ATCG").unwrap(),
            coverage: 5,
            confidence: vec![1.0, 0.6, 0.4, 1.0],
            avg_confidence: 0.75,
        };

        let low = low_confidence_positions(&result, 0.8);
        assert_eq!(low, vec![1, 2]); // Positions 1 and 2 are below 80%
    }

    #[test]
    fn test_consensus_batch() {
        let groups = vec![
            vec![
                DnaStrand::from_str("AAAA").unwrap(),
                DnaStrand::from_str("AAAA").unwrap(),
            ],
            vec![
                DnaStrand::from_str("TTTT").unwrap(),
                DnaStrand::from_str("TTTT").unwrap(),
            ],
        ];

        let results = build_consensus_batch(&groups);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].as_ref().unwrap().sequence.to_string(), "AAAA");
        assert_eq!(results[1].as_ref().unwrap().sequence.to_string(), "TTTT");
    }

    #[test]
    fn test_consensus_corrects_frame_shifting_indels() {
        // A majority-of-reads-affected scenario: only 2 of 5 copies are
        // exact, the other 3 each have a *different* single indel. Plain
        // positional voting (comparing raw index i across all 5 reads)
        // would be dominated by three different frame-shifted reads past
        // the indel point and would not reliably reconstruct the original.
        // The POA graph anchors every read to shared graph coordinates, so
        // the two exact copies plus the correctly-realigned bases from the
        // other three still agree.
        let original = "ATCGATCGTACGATCG";
        let exact_a = DnaStrand::from_str(original).unwrap();
        let exact_b = DnaStrand::from_str(original).unwrap();
        // Deletion: drop the 'T' at index 8.
        let deletion = DnaStrand::from_str("ATCGATCGACGATCG").unwrap();
        // Insertion: an extra 'A' after index 8.
        let insertion = DnaStrand::from_str("ATCGATCGTAACGATCG").unwrap();
        // A second, independent deletion: drop the 'G' at index 3.
        let deletion2 = DnaStrand::from_str("ATCATCGTACGATCG").unwrap();

        let reads = vec![exact_a, exact_b, deletion, insertion, deletion2];
        let result = build_consensus(&reads).expect("non-empty group must produce a consensus");
        assert_eq!(
            result.sequence.to_string(),
            original,
            "alignment-anchored voting should recover the original sequence \
             even though 3 of 5 reads are frame-shifted by an indel"
        );
    }

    #[test]
    fn test_consensus_handles_different_length_groups_without_panicking() {
        // Regression guard: reads of wildly different lengths (e.g. a
        // severely truncated Nanopore read) must not panic the aligner.
        let reads = vec![
            DnaStrand::from_str("ATCGATCGATCG").unwrap(),
            DnaStrand::from_str("AT").unwrap(),
            DnaStrand::from_str("ATCGATCGATCGATCGATCG").unwrap(),
        ];
        let result = build_consensus(&reads);
        assert!(result.is_some());
    }

    #[test]
    fn test_confidence_never_exceeds_one() {
        // Regression guard for the POA support bookkeeping: no node should
        // ever be double-counted for the same read.
        let reads = vec![
            DnaStrand::from_str("ATCGATCGATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATCGATCGATCG").unwrap(),
            DnaStrand::from_str("ATCGATTCGATCGATCG").unwrap(), // insertion
            DnaStrand::from_str("ATCGATCGATCGATC").unwrap(),   // deletion
            DnaStrand::from_str("ATCGATCGATCGATCG").unwrap(),
        ];
        let result = build_consensus(&reads).unwrap();
        assert!(result.confidence.iter().all(|&c| c <= 1.0 + 1e-9), "{:?}", result.confidence);
    }

    /// The scenario that pairwise single-reference realignment could not
    /// solve at all: many reads, each independently carrying *several*
    /// simultaneous indels (a higher combined edit rate than real Nanopore
    /// noise, deliberately, to stress-test past it), rather than just one
    /// indel each. Folding every read into a shared, multi-round-polished
    /// POA graph gets this overwhelmingly right -- honestly verified here
    /// to land within 2 edits of the true 43-base sequence, not asserted
    /// as pixel-perfect, because it isn't quite: a small residual (1
    /// extra/wrong base) can survive even multiple polishing rounds at
    /// this density, traced to column identity occasionally fragmenting
    /// near a compounding cluster of edits in the graph's own initial
    /// seed read. (An earlier attempt at polishing had a double-counted
    /// vote weight that briefly broke the working Illumina case; that bug
    /// is what `PoaGraph::seed_unweighted` exists to prevent, not
    /// polishing itself, which is now verified safe -- see
    /// docs/architecture.md's "Current status" for the fuller account.)
    #[test]
    fn test_consensus_gets_within_two_edits_under_many_simultaneous_indels_per_read() {
        let original = "ATCGATCGTACGATCGGATCCATGACTGATCGTACGGATCAGT";
        let orig_bases: Vec<Nucleotide> = DnaStrand::from_str(original).unwrap().bases().to_vec();

        // A tiny deterministic PRNG (xorshift) so this test has no external
        // randomness dependency but still exercises many distinct,
        // independent multi-indel reads.
        struct Xorshift(u64);
        impl Xorshift {
            fn next(&mut self) -> u64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0
            }
            fn range(&mut self, n: usize) -> usize {
                (self.next() % n as u64) as usize
            }
        }
        let bases_alphabet = [Nucleotide::A, Nucleotide::C, Nucleotide::G, Nucleotide::T];

        let mut rng = Xorshift(0x1234_5678_9abc_def0);
        let mut reads = Vec::new();
        for _ in 0..30 {
            let mut bases = orig_bases.clone();
            // 3-6 simultaneous edits per read: a mix of deletions,
            // insertions, and substitutions at independent, scattered
            // positions -- mirroring a single Nanopore read's error count
            // over a strand this length.
            let edits = 3 + rng.range(4);
            for _ in 0..edits {
                if bases.is_empty() {
                    break;
                }
                let pos = rng.range(bases.len());
                match rng.range(3) {
                    0 => {
                        bases.remove(pos); // deletion
                    }
                    1 => {
                        bases.insert(pos, bases_alphabet[rng.range(4)]); // insertion
                    }
                    _ => {
                        bases[pos] = bases_alphabet[rng.range(4)]; // substitution
                    }
                }
            }
            reads.push(DnaStrand::new(bases));
        }

        let result = build_consensus(&reads).expect("non-empty group must produce a consensus");
        let distance = edit_distance(orig_bases.as_slice(), result.sequence.bases());
        assert!(
            distance <= 2,
            "POA consensus across 30 independently multi-indel reads should land within 2 \
             edits of the original even though no single reference read is clean -- got \
             {distance} edits (result: {}, expected: {original})",
            result.sequence
        );
    }

    /// Plain Levenshtein edit distance, used only to state the bound in
    /// `test_consensus_gets_within_two_edits_under_many_simultaneous_indels_per_read`
    /// precisely instead of vaguely.
    fn edit_distance(a: &[Nucleotide], b: &[Nucleotide]) -> usize {
        let (n, m) = (a.len(), b.len());
        let mut dp = vec![vec![0usize; m + 1]; n + 1];
        for (i, row) in dp.iter_mut().enumerate() {
            row[0] = i;
        }
        for j in 0..=m {
            dp[0][j] = j;
        }
        for i in 1..=n {
            for j in 1..=m {
                dp[i][j] = if a[i - 1] == b[j - 1] {
                    dp[i - 1][j - 1]
                } else {
                    1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
                };
            }
        }
        dp[n][m]
    }
}
