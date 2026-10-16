//! Pure analyses over a `TestMatrix` (Story 2.1.2, ADR-005): unique coverage and kills, greedy cover, joint
//! loss, the joint-safe subset and dynamic subsumption. Every tie-break is test-id order, so output is reproducible.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

use super::matrix::{
    CoverageDimension, CoverageUnit, Dimension, DimensionMeta, KillDimension, MutantId, TestId,
    TestMatrix,
};

/// Units covered by (or mutants killed by) each test and by no other; every test appears, possibly with `{}`.
pub fn unique_units<U: Ord + Clone, M: DimensionMeta>(
    d: &Dimension<U, M>,
) -> BTreeMap<TestId, BTreeSet<U>> {
    let mut out: BTreeMap<TestId, BTreeSet<U>> = d
        .tests
        .iter()
        .map(|t| (t.clone(), BTreeSet::new()))
        .collect();
    for (unit, owner) in d.singleton_units() {
        out.entry(d.test(owner).clone())
            .or_default()
            .insert(unit.clone());
    }
    out
}

pub fn unique_coverage(d: &CoverageDimension) -> BTreeMap<TestId, BTreeSet<CoverageUnit>> {
    unique_units(d)
}

pub fn unique_kills(d: &KillDimension) -> BTreeMap<TestId, BTreeSet<MutantId>> {
    unique_units(d)
}

/// Unit index lists per test ordinal (the transpose of the posting lists).
fn units_by_test<U, M>(d: &Dimension<U, M>) -> Vec<Vec<u32>> {
    let mut by_test = vec![Vec::new(); d.tests.len()];
    for (unit_ix, posting) in d.index.values().enumerate() {
        for &t in &posting.tests {
            by_test[t as usize].push(unit_ix as u32);
        }
    }
    by_test
}

/// Greedy set cover: repeatedly take the test adding most uncovered units (ties: lowest test id), then drop
/// any pick the others make redundant, so the result preserves the union and is subset-minimal.
pub fn greedy_reduction<U: Ord, M: DimensionMeta>(d: &Dimension<U, M>) -> BTreeSet<TestId> {
    let by_test = units_by_test(d);
    let mut covered = vec![false; d.index.len()];
    let mut heap: BinaryHeap<(usize, Reverse<u32>)> = by_test
        .iter()
        .enumerate()
        .filter(|(_, units)| !units.is_empty())
        .map(|(t, units)| (units.len(), Reverse(t as u32)))
        .collect();
    let mut picks: Vec<u32> = Vec::new();
    while let Some((stored, Reverse(t))) = heap.pop() {
        let gain = by_test[t as usize]
            .iter()
            .filter(|&&u| !covered[u as usize])
            .count();
        if gain == 0 {
            continue;
        }
        if gain < stored {
            heap.push((gain, Reverse(t)));
            continue;
        }
        for &u in &by_test[t as usize] {
            covered[u as usize] = true;
        }
        picks.push(t);
    }
    let mut cover_count = vec![0u32; d.index.len()];
    for &t in &picks {
        for &u in &by_test[t as usize] {
            cover_count[u as usize] += 1;
        }
    }
    let mut kept: BTreeSet<u32> = picks.iter().copied().collect();
    for &t in picks.iter().rev() {
        if by_test[t as usize]
            .iter()
            .all(|&u| cover_count[u as usize] > 1)
        {
            for &u in &by_test[t as usize] {
                cover_count[u as usize] -= 1;
            }
            kept.remove(&t);
        }
    }
    kept.into_iter().map(|t| d.test(t).clone()).collect()
}

/// Units that lose every covering test if the whole `set` is removed together (never per test).
pub fn joint_loss<U: Ord + Clone, M: DimensionMeta>(
    d: &Dimension<U, M>,
    set: &BTreeSet<TestId>,
) -> BTreeSet<U> {
    let ordinals: BTreeSet<u32> = set.iter().filter_map(|t| d.ordinal_of(t)).collect();
    d.index
        .iter()
        .filter(|(_, p)| p.tests.iter().all(|t| ordinals.contains(t)))
        .map(|(u, _)| u.clone())
        .collect()
}

/// A flagged test left out of the joint-safe set, with the members of the set it is only redundant with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub test: TestId,
    pub redundant_only_with: Vec<TestId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JointSafe {
    pub safe: Vec<TestId>,
    pub rejected: Vec<Rejected>,
}

/// Tracks, per unit, how many covering tests are still outside the growing deletion set.
struct Tracker {
    tests: Vec<TestId>,
    postings: Vec<Vec<u32>>,
    by_test: Vec<Vec<u32>>,
    remaining: Vec<u32>,
    added: Vec<bool>,
}

impl Tracker {
    fn new<U: Ord, M: DimensionMeta>(d: &Dimension<U, M>) -> Self {
        let postings: Vec<Vec<u32>> = d.index.values().map(|p| p.tests.clone()).collect();
        Self {
            tests: d.tests.clone(),
            remaining: postings.iter().map(|p| p.len() as u32).collect(),
            by_test: units_by_test(d),
            added: vec![false; d.tests.len()],
            postings,
        }
    }

    fn ordinal(&self, test: &TestId) -> Option<usize> {
        self.tests.binary_search(test).ok()
    }

    /// Units whose last covering test outside the set is `test`.
    fn blocked_units(&self, test: &TestId) -> Vec<u32> {
        self.ordinal(test)
            .map(|t| {
                self.by_test[t]
                    .iter()
                    .copied()
                    .filter(|&u| self.remaining[u as usize] == 1)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn partners(&self, units: &[u32]) -> impl Iterator<Item = &TestId> {
        units
            .iter()
            .flat_map(|&u| &self.postings[u as usize])
            .filter(|&&t| self.added[t as usize])
            .map(|&t| &self.tests[t as usize])
    }

    fn add(&mut self, test: &TestId) {
        if let Some(t) = self.ordinal(test) {
            self.added[t] = true;
            for &u in &self.by_test[t] {
                self.remaining[u as usize] -= 1;
            }
        }
    }
}

/// Greedy in test-id order: a flagged test joins the deletion set only if the set's joint loss stays empty in
/// every supplied dimension (coverage units and, where present, killed mutants).
pub fn joint_safe_subset(matrix: &TestMatrix, flagged: &[TestId]) -> JointSafe {
    let mut trackers: Vec<Tracker> = Vec::new();
    trackers.extend(matrix.coverage.as_ref().map(Tracker::new));
    trackers.extend(matrix.kills.as_ref().map(Tracker::new));
    let order: BTreeSet<&TestId> = flagged.iter().collect();
    let mut out = JointSafe::default();
    for test in order {
        let mut blocked = false;
        let mut partners: BTreeSet<TestId> = BTreeSet::new();
        for tr in &trackers {
            let units = tr.blocked_units(test);
            if !units.is_empty() {
                blocked = true;
                partners.extend(tr.partners(&units).cloned());
            }
        }
        if blocked {
            out.rejected.push(Rejected {
                test: test.clone(),
                redundant_only_with: partners.into_iter().collect(),
            });
        } else {
            trackers.iter_mut().for_each(|tr| tr.add(test));
            out.safe.push(test.clone());
        }
    }
    out
}

/// Label every subsumption claim carries: no equivalence claims beyond the supplied suite.
pub const SUBSUMPTION_LABEL: &str = "relative to supplied suite";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subsumption {
    pub label: &'static str,
    /// Mutants not subsumed by any other.
    pub dominators: Vec<MutantId>,
    /// `(dominator, subsumed)`: every test killing the dominator also kills the subsumed mutant.
    pub subsumes: Vec<(MutantId, MutantId)>,
}

/// `m2` subsumes `m1` when `killers(m2)` is a subset of `killers(m1)`; identical killer sets keep the lowest id.
pub fn dominators(d: &KillDimension) -> Subsumption {
    let keys: Vec<&MutantId> = d.index.keys().collect();
    let killers: Vec<&[u32]> = d.index.values().map(|p| p.tests.as_slice()).collect();
    let mut mutants_of_test: Vec<Vec<usize>> = vec![Vec::new(); d.tests.len()];
    for (m, ks) in killers.iter().enumerate() {
        for &t in *ks {
            mutants_of_test[t as usize].push(m);
        }
    }
    let mut subsumes = Vec::new();
    let mut is_subsumed = vec![false; keys.len()];
    for (m2, ks2) in killers.iter().enumerate() {
        let Some(&first) = ks2.first() else { continue };
        for &m1 in &mutants_of_test[first as usize] {
            let strict = killers[m1].len() > ks2.len();
            if m1 != m2 && (strict || m2 < m1) && is_subset(ks2, killers[m1]) {
                subsumes.push((keys[m2].clone(), keys[m1].clone()));
                is_subsumed[m1] = true;
            }
        }
    }
    subsumes.sort();
    Subsumption {
        label: SUBSUMPTION_LABEL,
        dominators: keys
            .iter()
            .zip(&is_subsumed)
            .filter(|(_, s)| !**s)
            .map(|(k, _)| (*k).clone())
            .collect(),
        subsumes,
    }
}

/// Both slices sorted ascending.
fn is_subset(small: &[u32], big: &[u32]) -> bool {
    let mut it = big.iter();
    small.iter().all(|s| it.any(|b| b == s))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::super::matrix::test_support::*;
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<TestId> {
        names.iter().map(|n| tid(n)).collect()
    }

    fn units(lines: &[u32]) -> BTreeSet<CoverageUnit> {
        lines
            .iter()
            .map(|&l| CoverageUnit::new("p.go", l))
            .collect()
    }

    fn t1t2t3() -> CoverageDimension {
        cov(&[("T1", &[1, 2, 3]), ("T2", &[2, 3]), ("T3", &[3, 4])])
    }

    fn matrix(c: Option<CoverageDimension>, k: Option<KillDimension>) -> TestMatrix {
        TestMatrix {
            coverage: c,
            kills: k,
        }
    }

    #[test]
    fn unique_coverage_should_ReturnSetDifference_When_T1T2T3Example() {
        let u = unique_coverage(&t1t2t3());
        assert_eq!(u[&tid("T1")], units(&[1]));
        assert_eq!(u[&tid("T2")], units(&[]));
        assert_eq!(u[&tid("T3")], units(&[4]));
    }

    #[test]
    fn unique_kills_should_ReturnMutantsKilledByOnlyThisTest_When_KillMatrixGiven() {
        let d = kills(&[("T1", &["m1", "m2"]), ("T2", &["m2"])]);
        let u = unique_kills(&d);
        assert_eq!(u[&tid("T1")], BTreeSet::from([MutantId("m1".into())]));
        assert!(u[&tid("T2")].is_empty());
    }

    #[test]
    fn unique_coverage_should_ReturnEmpty_When_NoTests() {
        assert!(unique_coverage(&cov(&[])).is_empty());
        assert!(greedy_reduction(&cov(&[])).is_empty());
    }

    #[test]
    fn greedy_reduction_should_ReturnT1T3_When_RunTwice() {
        let d = t1t2t3();
        let (a, b) = (greedy_reduction(&d), greedy_reduction(&d));
        assert_eq!(a, set(&["T1", "T3"]));
        assert_eq!(a, b);
        let union: BTreeSet<u32> = (1..=4).collect();
        let covered: BTreeSet<u32> = d
            .index
            .iter()
            .filter(|(_, p)| p.tests.iter().any(|&t| a.contains(d.test(t))))
            .map(|(u, _)| u.line)
            .collect();
        assert_eq!(covered, union);
    }

    #[test]
    fn joint_loss_should_ReturnUnit1_When_ABBothOnlyCoverOne() {
        let d = cov(&[("A", &[1]), ("B", &[1])]);
        assert!(joint_loss(&d, &set(&["A"])).is_empty());
        assert!(joint_loss(&d, &set(&["B"])).is_empty());
        assert_eq!(joint_loss(&d, &set(&["A", "B"])), units(&[1]));
    }

    #[test]
    fn joint_safe_subset_should_ReturnAC_When_ABCFlaggedAndBRejected() {
        let d = cov(&[("A", &[1]), ("B", &[1]), ("C", &[2]), ("D", &[2])]);
        let r = joint_safe_subset(&matrix(Some(d), None), &[tid("C"), tid("B"), tid("A")]);
        assert_eq!(r.safe, vec![tid("A"), tid("C")]);
        assert_eq!(
            r.rejected,
            vec![Rejected {
                test: tid("B"),
                redundant_only_with: vec![tid("A")]
            }]
        );
    }

    #[test]
    fn joint_safe_subset_should_RejectTest_When_MutantWouldLoseAllKillers() {
        let c = cov(&[("A", &[1]), ("B", &[1])]);
        let k = kills(&[("A", &["m"]), ("B", &["m"])]);
        let both = joint_safe_subset(&matrix(Some(c.clone()), Some(k)), &[tid("A"), tid("B")]);
        assert_eq!(both.safe, vec![tid("A")]);
        // Coverage alone is also rejected here; make coverage permissive so only kills decide.
        let c = cov(&[("A", &[1]), ("B", &[1]), ("X", &[1])]);
        let k = kills(&[("A", &["m"]), ("B", &["m"])]);
        let r = joint_safe_subset(&matrix(Some(c), Some(k)), &[tid("A"), tid("B")]);
        assert_eq!(r.safe, vec![tid("A")]);
        assert_eq!(r.rejected[0].test, tid("B"));
        assert_eq!(r.rejected[0].redundant_only_with, vec![tid("A")]);
    }

    #[test]
    fn dominators_should_LabelRelativeToSuppliedSuite_When_M2SubsumesM1() {
        let d = kills(&[("T1", &["m1", "m2"]), ("T2", &["m1"])]);
        let s = dominators(&d);
        assert_eq!(s.label, "relative to supplied suite");
        assert_eq!(s.dominators, vec![MutantId("m2".into())]);
        assert_eq!(
            s.subsumes,
            vec![(MutantId("m2".into()), MutantId("m1".into()))]
        );
    }

    #[test]
    fn dominators_should_KeepLowestId_When_KillerSetsIdentical() {
        let d = kills(&[("T1", &["a", "b"])]);
        let s = dominators(&d);
        assert_eq!(s.dominators, vec![MutantId("a".into())]);
    }

    fn random_cov(rng: &mut Rng) -> CoverageDimension {
        let tests: Vec<String> = (0..1 + rng.below(8)).map(|i| format!("T{i}")).collect();
        let mut rows: Vec<(String, Vec<u32>)> = Vec::new();
        for t in &tests {
            let us: Vec<u32> = (0..rng.below(6)).map(|_| rng.below(10) as u32).collect();
            rows.push((t.clone(), us));
        }
        let borrowed: Vec<(&str, &[u32])> = rows
            .iter()
            .map(|(t, u)| (t.as_str(), u.as_slice()))
            .collect();
        cov(&borrowed)
    }

    fn random_kills(rng: &mut Rng) -> KillDimension {
        let names: Vec<String> = (0..1 + rng.below(8)).map(|i| format!("T{i}")).collect();
        let mut rows: Vec<(String, Vec<String>)> = Vec::new();
        for t in &names {
            let ms = (0..rng.below(5))
                .map(|_| format!("m{}", rng.below(6)))
                .collect();
            rows.push((t.clone(), ms));
        }
        let owned: Vec<(&str, Vec<&str>)> = rows
            .iter()
            .map(|(t, ms)| (t.as_str(), ms.iter().map(String::as_str).collect()))
            .collect();
        let borrowed: Vec<(&str, &[&str])> =
            owned.iter().map(|(t, m)| (*t, m.as_slice())).collect();
        kills(&borrowed)
    }

    #[test]
    fn greedy_reduction_should_PreserveUnion_When_RandomMatrices() {
        let mut rng = Rng(0xDEAD_BEEF_1234_5678);
        for _ in 0..300 {
            let d = random_cov(&mut rng);
            let picked = greedy_reduction(&d);
            let all: BTreeSet<TestId> = d.tests.iter().cloned().collect();
            let removed: BTreeSet<TestId> = all.difference(&picked).cloned().collect();
            assert!(joint_loss(&d, &removed).is_empty(), "union lost");
            // Subset-minimal: dropping any one pick loses a unit.
            for p in &picked {
                let mut r = removed.clone();
                r.insert(p.clone());
                assert!(!joint_loss(&d, &r).is_empty(), "{p} is redundant");
            }
            assert_eq!(picked, greedy_reduction(&d));
        }
    }

    fn random_flagged(rng: &mut Rng, ntests: usize) -> Vec<TestId> {
        (0..ntests)
            .filter(|_| rng.below(2) == 0)
            .map(|i| tid(&format!("T{i}")))
            .collect()
    }

    #[test]
    fn joint_safe_subset_should_BeSubsetOfIndividuallyFlagged_When_RandomMatrices() {
        let mut rng = Rng(0x1357_9BDF_2468_ACE1);
        for _ in 0..300 {
            let m = matrix(Some(random_cov(&mut rng)), Some(random_kills(&mut rng)));
            let flagged = random_flagged(&mut rng, 8);
            let r = joint_safe_subset(&m, &flagged);
            assert!(r.safe.iter().all(|t| flagged.contains(t)));
            assert_eq!(r.safe.len() + r.rejected.len(), flagged.len());
            assert!(r.safe.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn joint_safe_subset_should_HaveZeroJointLoss_When_RandomMatrices() {
        let mut rng = Rng(0x0F0F_1234_ABCD_9876);
        for _ in 0..300 {
            let (c, k) = (random_cov(&mut rng), random_kills(&mut rng));
            let flagged = random_flagged(&mut rng, 8);
            let r = joint_safe_subset(&matrix(Some(c.clone()), Some(k.clone())), &flagged);
            let safe: BTreeSet<TestId> = r.safe.iter().cloned().collect();
            assert!(joint_loss(&c, &safe).is_empty());
            assert!(joint_loss(&k, &safe).is_empty());
            // Maximal in order: each rejected test would break the invariant on top of the safe tests before it.
            for rej in &r.rejected {
                let mut s: BTreeSet<TestId> =
                    r.safe.iter().filter(|t| **t < rej.test).cloned().collect();
                s.insert(rej.test.clone());
                assert!(!joint_loss(&c, &s).is_empty() || !joint_loss(&k, &s).is_empty());
            }
        }
    }

    #[test]
    fn dominators_should_AgreeWithBruteForce_When_RandomMatrices() {
        let mut rng = Rng(0x5555_AAAA_3333_CCCC);
        for _ in 0..200 {
            let d = random_kills(&mut rng);
            let s = dominators(&d);
            let ks: Vec<(&MutantId, &Vec<u32>)> =
                d.index.iter().map(|(m, p)| (m, &p.tests)).collect();
            for (i, (m1, k1)) in ks.iter().enumerate() {
                let subsumed = ks.iter().enumerate().any(|(j, (_, k2))| {
                    j != i && k2.iter().all(|t| k1.contains(t)) && (k2.len() < k1.len() || j < i)
                });
                assert_eq!(s.dominators.contains(m1), !subsumed);
            }
        }
    }
}
