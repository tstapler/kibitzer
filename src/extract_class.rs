//! JDeodorant-style Extract Class candidate generation (#38): build a Jaccard-distance
//! graph over a type's methods (edge weight = shared field-access overlap, from
//! `ArchModel::field_accesses`, #39), cluster via average-linkage hierarchical
//! agglomerative clustering (HAC), then rank dendrogram cuts by an Entity-Placement-inspired
//! cohesion score (Fokaefs et al., *Identification and application of Extract Class
//! refactorings in object-oriented systems*, JSS 2012) — not a literal reproduction of that
//! paper's formula, see [`entity_placement_score`]'s doc for the simplification used here.
//! Naming the extracted class is out of scope (a human/LLM step, per the issue); this
//! module only proposes *which methods* would split cleanly.
//!
//! Go-only, same scope as `ArchModel::field_accesses`: a method with zero field accesses is
//! excluded from clustering entirely rather than guessed at.

use std::collections::{HashMap, HashSet};

use crate::arch_model::{ArchModel, PackageNode, SymbolKind};
use crate::jaccard::jaccard;

/// A type needs at least this many methods with field-access data before clustering is
/// attempted — matches `architecture_checks::LCOM_MIN_METHODS`'s bar (not shared as a
/// `pub` constant across modules, per this codebase's existing per-file threshold
/// convention, e.g. `MAX_FAN_OUT`/`STABLE_MAX`). Also the minimum needed for a `[2, n-1]`
/// candidate-cut range to be non-empty (`n=4` gives cuts at `k=2,3`).
const MIN_METHODS_FOR_CLUSTERING: usize = 4;

/// Upper bound on methods-with-field-access-data clustered for one type: `hac_partitions`
/// is O(n³) and `candidate_splits` adds an O(n²)-per-cut pass on top, so an unbounded type
/// (a large generated or god-class type with hundreds of methods) could burn real CPU time
/// with no per-type or total-time budget elsewhere in `extract_class_candidates`'s repo-wide
/// loop. A type above this is skipped, not an error — clustering advice on a type this
/// large is marginal anyway.
const MAX_METHODS_FOR_CLUSTERING: usize = 80;

/// Minimum Entity Placement score (see [`entity_placement_score`]) for a candidate split
/// to be worth surfacing — below this, too many methods are ambiguously placed for the
/// split to be actionable advice. No stronger literature citation than "clearly the
/// majority," same status as this codebase's other threshold constants.
const MIN_ENTITY_PLACEMENT_SCORE: f64 = 0.75;

/// One candidate Extract Class split for a type: the methods with field-access data,
/// partitioned into two or more groups. Each group is a candidate for its own type; the
/// remaining methods on the original type (those with no field-access data) aren't
/// assigned anywhere — an Extract Class refactoring on real code would still need a human
/// or LLM to decide where they land.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractClassCandidate {
    pub package: String,
    pub type_name: String,
    /// Each inner `Vec` is one candidate group's method ids (`SymbolNode::id`), in
    /// clustering order (not sorted) — at least 2 groups, each with at least 1 method.
    pub groups: Vec<Vec<String>>,
    /// `[0.0, 1.0]` — see [`entity_placement_score`]. Higher is a cleaner split.
    pub entity_placement_score: f64,
}

/// Every Go method of `type_name` in `pkg` that has at least one field access recorded in
/// `model.field_accesses`, mapped to the set of field ids it accesses. A method with zero
/// field accesses is simply absent from the returned map — see this module's doc comment.
fn method_field_sets<'a>(
    model: &'a ArchModel,
    pkg: &'a PackageNode,
    type_name: &str,
) -> HashMap<&'a str, HashSet<&'a str>> {
    let method_ids: HashSet<&str> = pkg
        .symbols
        .iter()
        .filter(|s| {
            s.kind == SymbolKind::Method
                && s.parent.as_deref() == Some(type_name)
                && s.file.extension().is_some_and(|ext| ext == "go")
        })
        .map(|s| s.id.as_str())
        .collect();

    let mut sets: HashMap<&str, HashSet<&str>> = HashMap::new();
    for access in &model.field_accesses {
        if method_ids.contains(access.from.as_str()) {
            sets.entry(access.from.as_str())
                .or_default()
                .insert(access.to.as_str());
        }
    }
    sets
}

/// Builds the full pairwise Jaccard-similarity matrix for `ids[0..n]`'s field-access sets
/// (`field_sets[ids[i]]`), `sim[i][j] == sim[j][i]`, `sim[i][i] == 0.0` (a method is never
/// compared against itself during clustering).
fn similarity_matrix(ids: &[&str], field_sets: &HashMap<&str, HashSet<&str>>) -> Vec<Vec<f64>> {
    let n = ids.len();
    let mut sim = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let s = jaccard(&field_sets[ids[i]], &field_sets[ids[j]]);
            sim[i][j] = s;
            sim[j][i] = s;
        }
    }
    sim
}

/// Average pairwise similarity between two clusters (lists of method indices) — the
/// average-linkage criterion HAC merges on at each step.
fn average_linkage(a: &[usize], b: &[usize], sim: &[Vec<f64>]) -> f64 {
    let mut total = 0.0;
    for &i in a {
        for &j in b {
            total += sim[i][j];
        }
    }
    total / (a.len() * b.len()) as f64
}

/// Finds the pair of cluster indices with the highest average-linkage similarity —
/// `hac_partitions`'s per-merge-step search, split out to keep that function's nesting
/// shallow. Ties resolve to the lowest `(i, j)` pair scanned first.
fn best_merge_pair(clusters: &[Vec<usize>], sim: &[Vec<f64>]) -> (usize, usize) {
    let mut best = (0usize, 1usize, f64::MIN);
    for i in 0..clusters.len() {
        for j in (i + 1)..clusters.len() {
            let s = average_linkage(&clusters[i], &clusters[j], sim);
            if s > best.2 {
                best = (i, j, s);
            }
        }
    }
    (best.0, best.1)
}

/// Runs average-linkage HAC over `n` methods and returns every partition produced along
/// the way, from `n` singleton clusters (index `0`) down to 1 cluster containing
/// everything (index `n-1`) — i.e. `result[i]` has `n-i` clusters.
fn hac_partitions(n: usize, sim: &[Vec<f64>]) -> Vec<Vec<Vec<usize>>> {
    let mut clusters: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut history = vec![clusters.clone()];
    while clusters.len() > 1 {
        let (i, j) = best_merge_pair(&clusters, sim);
        let mut merged = clusters[i].clone();
        merged.extend(clusters[j].iter().copied());
        clusters.remove(j);
        clusters.remove(i);
        clusters.push(merged);
        history.push(clusters.clone());
    }
    history
}

/// Average similarity of method `m` to every other member of `cluster` (excluding `m`
/// itself if present — `m` is never a member of an "other" cluster it's being compared
/// against, so the exclusion is a no-op there). `0.0` for a cluster with no other members
/// to compare against. Averaging, not summing, matters: a bigger cluster otherwise wins by
/// sheer member count even when each individual similarity is weak (e.g. five members at
/// 0.3 each summing to 1.5, beating one true clustermate at 0.9) — that would bias
/// [`is_well_placed`] toward large, loosely-related clusters over small, tightly cohesive
/// ones.
fn mean_similarity_to(m: usize, cluster: &[usize], sim: &[Vec<f64>]) -> f64 {
    let others: Vec<usize> = cluster.iter().copied().filter(|&x| x != m).collect();
    if others.is_empty() {
        return 0.0;
    }
    others.iter().map(|&x| sim[m][x]).sum::<f64>() / others.len() as f64
}

/// Whether method `m` (a member of `clusters[own_idx]`) is at least as similar, on
/// average, to its own cluster as to every other individual cluster.
fn is_well_placed(m: usize, own_idx: usize, clusters: &[Vec<usize>], sim: &[Vec<f64>]) -> bool {
    let own_sim = mean_similarity_to(m, &clusters[own_idx], sim);
    let best_other = clusters
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != own_idx)
        .map(|(_, other)| mean_similarity_to(m, other, sim))
        .fold(0.0_f64, f64::max);
    own_sim >= best_other
}

/// A simplified, documented stand-in for Fokaefs et al.'s Entity Placement metric: the
/// fraction of methods whose average similarity to their *own* cluster ([`mean_similarity_to`])
/// is at least as high as their average similarity to every other individual cluster
/// ([`is_well_placed`]). `1.0` means every method is unambiguously best-placed where the
/// clustering put it; lower scores mean some methods are about as similar to a different
/// candidate group as to their own — a weak split. The paper's actual EP metric
/// additionally weighs *coupling introduced* (calls that would cross the new class
/// boundary) using call-graph data this module doesn't have cheap access to per candidate
/// cut; that refinement is future work, not attempted here.
fn entity_placement_score(clusters: &[Vec<usize>], sim: &[Vec<f64>]) -> f64 {
    let n: usize = clusters.iter().map(Vec::len).sum();
    if n == 0 {
        return 0.0;
    }
    let well_placed = clusters
        .iter()
        .enumerate()
        .flat_map(|(own_idx, cluster)| cluster.iter().map(move |&m| (m, own_idx)))
        .filter(|&(m, own_idx)| is_well_placed(m, own_idx, clusters, sim))
        .count();
    well_placed as f64 / n as f64
}

/// Sums `sim[a][b]` over every `a` in `cluster` and `b` in `other`, plus the pair count —
/// `separates_meaningfully`'s cross-group accumulation, split out to keep that function's
/// nesting shallow.
fn accumulate_pairs(cluster: &[usize], other: &[usize], sim: &[Vec<f64>]) -> (f64, usize) {
    let mut total = 0.0;
    for &a in cluster {
        for &b in other {
            total += sim[a][b];
        }
    }
    (total, cluster.len() * other.len())
}

/// Mean similarity within `clusters`' groups vs. mean similarity across them. A method
/// with uniform similarity to every other method has no real cluster structure at all —
/// every entity is equally "well-placed" wherever the clustering happens to cut, so
/// [`entity_placement_score`] alone can't tell a genuine split from an arbitrary one on
/// uniform data. `candidate_splits` only considers a partition where intra-group similarity
/// is *strictly* higher than inter-group similarity, so a uniform-similarity type never
/// clears it.
fn separates_meaningfully(clusters: &[Vec<usize>], sim: &[Vec<f64>]) -> bool {
    let mut intra = (0.0_f64, 0usize);
    let mut inter = (0.0_f64, 0usize);
    for (i, cluster) in clusters.iter().enumerate() {
        for (a_pos, &a) in cluster.iter().enumerate() {
            let (total, count) = accumulate_pairs(&[a], &cluster[(a_pos + 1)..], sim);
            intra.0 += total;
            intra.1 += count;
        }
        for other in &clusters[(i + 1)..] {
            let (total, count) = accumulate_pairs(cluster, other, sim);
            inter.0 += total;
            inter.1 += count;
        }
    }
    let mean = |(total, count): (f64, usize)| {
        if count == 0 {
            0.0
        } else {
            total / count as f64
        }
    };
    mean(intra) > mean(inter)
}

/// Every dendrogram cut tied for the best qualifying [`entity_placement_score`], not a
/// single winner — issue #38 specifies "a closed set of graph-legal moves, no ranking
/// beyond genuine ties," so picking among cuts with genuinely different scores would itself
/// be an unprincipled ranking.
fn candidate_splits(ids: &[&str], sim: &[Vec<f64>]) -> Vec<(Vec<Vec<usize>>, f64)> {
    let n = ids.len();
    if n < 3 {
        return Vec::new();
    }
    let partitions = hac_partitions(n, sim);
    // partitions[i] has n-i clusters; we want cluster counts in [2, n-1], i.e. i in [1, n-2].
    let qualifying: Vec<(Vec<Vec<usize>>, f64)> = (1..=n.saturating_sub(2))
        .map(|i| {
            let clusters = &partitions[i];
            (clusters.clone(), entity_placement_score(clusters, sim))
        })
        .filter(|(clusters, score)| {
            *score >= MIN_ENTITY_PLACEMENT_SCORE && separates_meaningfully(clusters, sim)
        })
        .collect();

    let Some(best) = qualifying
        .iter()
        .map(|(_, score)| *score)
        .fold(None, |acc: Option<f64>, s| {
            Some(acc.map_or(s, |a| a.max(s)))
        })
    else {
        return Vec::new();
    };
    qualifying
        .into_iter()
        .filter(|(_, score)| *score == best)
        .collect()
}

fn candidates_for_type(
    model: &ArchModel,
    pkg: &PackageNode,
    type_name: &str,
) -> Vec<ExtractClassCandidate> {
    let field_sets = method_field_sets(model, pkg, type_name);
    if field_sets.len() < MIN_METHODS_FOR_CLUSTERING || field_sets.len() > MAX_METHODS_FOR_CLUSTERING
    {
        return Vec::new();
    }
    // Sorted, not left in HashMap iteration order: std's HashMap seeds its hasher randomly
    // per process, so leaving this unsorted would feed hac_partitions a different initial
    // index order (and, through best_merge_pair's positional tie-break) a possibly
    // different clustering result on every run — the same non-determinism
    // extract_class_candidates already guards against for its own type_names.
    let mut ids: Vec<&str> = field_sets.keys().copied().collect();
    ids.sort_unstable();
    let sim = similarity_matrix(&ids, &field_sets);

    candidate_splits(&ids, &sim)
        .into_iter()
        .map(|(clusters, score)| {
            let groups: Vec<Vec<String>> = clusters
                .into_iter()
                .map(|cluster| cluster.into_iter().map(|i| ids[i].to_string()).collect())
                .collect();
            ExtractClassCandidate {
                package: pkg.path.clone(),
                type_name: type_name.to_string(),
                groups,
                entity_placement_score: score,
            }
        })
        .collect()
}

/// All Extract Class candidates across the whole model — for every type that clears
/// [`MIN_METHODS_FOR_CLUSTERING`], every dendrogram cut tied for the best qualifying score
/// (see [`candidate_splits`]; usually exactly one, but genuine ties all come back rather
/// than an arbitrary pick among them). Sorted by package then type name for deterministic
/// output (`BTreeMap` iteration on `model.packages` already provides the package order;
/// types within a package are sorted explicitly since `PackageNode::symbols` isn't) — tied
/// candidates for the same type keep `candidate_splits`' own order.
pub fn extract_class_candidates(model: &ArchModel) -> Vec<ExtractClassCandidate> {
    let mut out = Vec::new();
    for pkg in model.packages.values() {
        let mut type_names: Vec<&str> = pkg
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .filter_map(|s| s.parent.as_deref())
            .collect();
        type_names.sort_unstable();
        type_names.dedup();
        for type_name in type_names {
            out.extend(candidates_for_type(model, pkg, type_name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{FieldAccessEdge, PruningSummary, SymbolNode};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn method(pkg: &str, type_name: &str, name: &str) -> SymbolNode {
        SymbolNode {
            id: format!("{pkg}::{type_name}.{name}"),
            name: name.to_string(),
            kind: SymbolKind::Method,
            file: PathBuf::from(format!("{pkg}/{name}.go")),
            line: 1,
            exported: true,
            parent: Some(type_name.to_string()),
        }
    }

    fn access(from: &str, to: &str) -> FieldAccessEdge {
        FieldAccessEdge {
            from: from.to_string(),
            to: to.to_string(),
            access: crate::arch_model::AccessKind::Read,
            file: PathBuf::from("f.go"),
            line: 1,
        }
    }

    fn model_of(pkg: PackageNode, field_accesses: Vec<FieldAccessEdge>) -> ArchModel {
        let mut packages = BTreeMap::new();
        packages.insert(pkg.path.clone(), pkg);
        ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            field_accesses,
            pruning: PruningSummary {
                include_private: false,
                excluded_dirs: vec![],
                generated_files_skipped: 0,
                private_symbols_skipped: 0,
                pruned_symbol_ids: vec![],
                files_with_parse_errors: vec![],
                unsupported_language_files: 0,
                total_files_scanned: 0,
            },
        }
    }

    #[test]
    fn proposes_a_clean_two_way_split_for_two_unrelated_field_clusters() {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![
                method("pkg", "T", "A"),
                method("pkg", "T", "B"),
                method("pkg", "T", "C"),
                method("pkg", "T", "D"),
            ],
        };
        let field_accesses = vec![
            access("pkg::T.A", "pkg::T.X"),
            access("pkg::T.B", "pkg::T.X"),
            access("pkg::T.C", "pkg::T.Y"),
            access("pkg::T.D", "pkg::T.Y"),
        ];
        let model = model_of(pkg, field_accesses);

        let candidates = extract_class_candidates(&model);
        assert_eq!(candidates.len(), 1, "got: {candidates:?}");
        let candidate = &candidates[0];
        assert_eq!(candidate.package, "pkg");
        assert_eq!(candidate.type_name, "T");
        assert_eq!(candidate.groups.len(), 2, "got: {:?}", candidate.groups);

        let mut ab = candidate
            .groups
            .iter()
            .find(|g| g.contains(&"pkg::T.A".to_string()))
            .expect("A's group")
            .clone();
        ab.sort();
        assert_eq!(ab, vec!["pkg::T.A".to_string(), "pkg::T.B".to_string()]);
        assert!(candidate.entity_placement_score >= MIN_ENTITY_PLACEMENT_SCORE);
    }

    #[test]
    fn proposes_no_candidate_for_a_fully_cohesive_type() {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![
                method("pkg", "T", "A"),
                method("pkg", "T", "B"),
                method("pkg", "T", "C"),
                method("pkg", "T", "D"),
            ],
        };
        // Every method touches the same single field — no meaningful split exists.
        let field_accesses = ["A", "B", "C", "D"]
            .iter()
            .map(|m| access(&format!("pkg::T.{m}"), "pkg::T.X"))
            .collect();
        let model = model_of(pkg, field_accesses);

        assert!(extract_class_candidates(&model).is_empty());
    }

    #[test]
    fn skips_types_above_the_maximum_method_threshold_without_hanging() {
        // hac_partitions is O(n^3); if the cap didn't gate this, MAX_METHODS_FOR_CLUSTERING+1
        // methods would take real CPU time to cluster. The test only needs to prove the type
        // is skipped, not clustered, so it should return instantly either way.
        let n = MAX_METHODS_FOR_CLUSTERING + 1;
        let symbols: Vec<SymbolNode> = (0..n)
            .map(|i| method("pkg", "T", &format!("M{i}")))
            .collect();
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols,
        };
        let field_accesses: Vec<FieldAccessEdge> = (0..n)
            .map(|i| access(&format!("pkg::T.M{i}"), &format!("pkg::T.F{i}")))
            .collect();
        let model = model_of(pkg, field_accesses);

        assert!(extract_class_candidates(&model).is_empty());
    }

    #[test]
    fn skips_types_below_the_minimum_method_threshold() {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![method("pkg", "T", "A"), method("pkg", "T", "B")],
        };
        let field_accesses = vec![
            access("pkg::T.A", "pkg::T.X"),
            access("pkg::T.B", "pkg::T.Y"),
        ];
        let model = model_of(pkg, field_accesses);

        assert!(extract_class_candidates(&model).is_empty());
    }

    #[test]
    fn excludes_methods_with_no_field_access_from_clustering() {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![
                method("pkg", "T", "A"),
                method("pkg", "T", "B"),
                method("pkg", "T", "C"),
                method("pkg", "T", "D"),
                method("pkg", "T", "NoFieldAccess"),
            ],
        };
        let field_accesses = vec![
            access("pkg::T.A", "pkg::T.X"),
            access("pkg::T.B", "pkg::T.X"),
            access("pkg::T.C", "pkg::T.Y"),
            access("pkg::T.D", "pkg::T.Y"),
        ];
        let model = model_of(pkg, field_accesses);

        let candidates = extract_class_candidates(&model);
        assert_eq!(candidates.len(), 1, "got: {candidates:?}");
        let all_methods: Vec<&String> = candidates[0].groups.iter().flatten().collect();
        assert!(!all_methods.contains(&&"pkg::T.NoFieldAccess".to_string()));
        assert_eq!(all_methods.len(), 4);
    }

    // --- mean_similarity_to / is_well_placed / entity_placement_score (sum-vs-average) ---

    #[test]
    fn is_well_placed_averages_rather_than_sums_across_cluster_size() {
        // Method 0's own (small, tight) cluster has one other member at similarity 0.9. A
        // larger 5-member "other" cluster has similarity 0.3 to method 0 with each of its
        // members. Summing (the old, buggy behavior) gives the other cluster 1.5 > 0.9,
        // reading method 0 as better placed in the big, loosely-related cluster than its
        // own tight one. Averaging correctly gives the other cluster 0.3 < 0.9.
        let clusters = vec![vec![0, 1], vec![2, 3, 4, 5, 6]];
        let n = 7;
        let mut sim = vec![vec![0.0; n]; n];
        sim[0][1] = 0.9;
        sim[1][0] = 0.9;
        for &j in &[2, 3, 4, 5, 6] {
            sim[0][j] = 0.3;
            sim[j][0] = 0.3;
        }

        assert!(is_well_placed(0, 0, &clusters, &sim));
    }

    #[test]
    fn entity_placement_score_is_perfect_when_a_small_tight_cluster_beats_a_larger_loose_one() {
        // Same asymmetry as above, but through the full entity_placement_score path: every
        // member of both clusters is well-placed once similarity is averaged, not summed —
        // under the old sum-based scoring, method 0 (and by symmetry method 1) would have
        // flipped to "better placed in the big cluster," dragging the score below 1.0.
        let clusters = vec![vec![0, 1], vec![2, 3, 4, 5, 6]];
        let n = 7;
        let mut sim = vec![vec![0.0; n]; n];
        sim[0][1] = 0.9;
        sim[1][0] = 0.9;
        for &i in &[0, 1] {
            for &j in &[2, 3, 4, 5, 6] {
                sim[i][j] = 0.3;
                sim[j][i] = 0.3;
            }
        }
        for &i in &[2, 3, 4, 5, 6] {
            for &j in &[2, 3, 4, 5, 6] {
                if i != j {
                    sim[i][j] = 0.5;
                }
            }
        }

        assert_eq!(entity_placement_score(&clusters, &sim), 1.0);
    }

    #[test]
    fn separates_meaningfully_is_false_for_uniform_similarity() {
        let clusters = vec![vec![0, 1], vec![2, 3]];
        let sim = vec![vec![1.0; 4]; 4];
        assert!(!separates_meaningfully(&clusters, &sim));
    }

    #[test]
    fn separates_meaningfully_is_true_when_intra_beats_inter() {
        let clusters = vec![vec![0, 1], vec![2, 3]];
        let mut sim = vec![vec![0.0; 4]; 4];
        sim[0][1] = 1.0;
        sim[1][0] = 1.0;
        sim[2][3] = 1.0;
        sim[3][2] = 1.0;
        assert!(separates_meaningfully(&clusters, &sim));
    }

    // --- candidate_splits (#7: return the closed set of tied cuts, not a single pick) ---

    #[test]
    fn candidate_splits_returns_every_partition_tied_for_the_best_score() {
        // Three tight pairs (sim=1.0 each) with uniform 0.4 cross-cluster similarity. Both
        // the 3-cluster cut ({0,1},{2,3},{4,5}) and the 2-cluster cut ({0,1,2,3},{4,5})
        // score a perfect 1.0 — every method is at least as similar, on average, to its own
        // group as to the best alternative under either cut. Per issue #38's "closed set
        // of graph-legal moves, no ranking beyond genuine ties" design, both must come
        // back — a single-winner design would arbitrarily drop one.
        let n = 6;
        let mut sim = vec![vec![0.4; n]; n];
        for (i, row) in sim.iter_mut().enumerate() {
            row[i] = 0.0;
        }
        for &(a, b) in &[(0, 1), (2, 3), (4, 5)] {
            sim[a][b] = 1.0;
            sim[b][a] = 1.0;
        }
        let ids = vec!["m0", "m1", "m2", "m3", "m4", "m5"];

        let splits = candidate_splits(&ids, &sim);
        let mut cluster_counts: Vec<usize> =
            splits.iter().map(|(clusters, _)| clusters.len()).collect();
        cluster_counts.sort_unstable();
        assert_eq!(cluster_counts, vec![2, 3], "got: {splits:?}");
        for (_, score) in &splits {
            assert_eq!(*score, 1.0, "got: {splits:?}");
        }
    }
}
