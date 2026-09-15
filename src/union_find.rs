//! A minimal union-find (disjoint-set) over a dense `0..n` index space, backed by a plain
//! `parent: Vec<usize>` the caller owns and initializes to `(0..n).collect()`. No union-by-
//! rank/size and no path compression beyond the recursive lookup in [`root`] — every caller
//! so far (`architecture_checks::LcomChecker`'s method-connectivity graph,
//! `root_cause_clusters`'s bug-fix-commit clustering) works over at most a few dozen
//! elements, where the asymptotic difference doesn't matter.

/// Finds `x`'s set representative, compressing the path as it walks up.
pub fn root(parent: &mut [usize], x: usize) -> usize {
    if parent[x] != x {
        parent[x] = root(parent, parent[x]);
    }
    parent[x]
}

/// Merges the sets containing `a` and `b`, if they're not already the same set.
pub fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = root(parent, a);
    let rb = root(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_joins_two_singletons_into_one_set() {
        let mut parent: Vec<usize> = (0..4).collect();
        union(&mut parent, 0, 1);
        assert_eq!(root(&mut parent, 0), root(&mut parent, 1));
        assert_ne!(root(&mut parent, 0), root(&mut parent, 2));
    }

    #[test]
    fn union_is_transitive_across_chained_merges() {
        let mut parent: Vec<usize> = (0..4).collect();
        union(&mut parent, 0, 1);
        union(&mut parent, 1, 2);
        assert_eq!(root(&mut parent, 0), root(&mut parent, 2));
        assert_ne!(root(&mut parent, 0), root(&mut parent, 3));
    }
}
