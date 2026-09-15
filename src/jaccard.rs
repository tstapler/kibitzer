//! Jaccard similarity over `&str` sets — shared by `extract_class` (method field-access
//! overlap) and `root_cause_clusters` (bug-fix-commit touched-file overlap).

use std::collections::HashSet;

/// `|a ∩ b| / |a ∪ b|`, `0.0` when both sets are empty (an empty union would otherwise
/// divide by zero).
pub fn jaccard(a: &HashSet<&str>, b: &HashSet<&str>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(b).count() as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_of_two_empty_sets_is_zero() {
        let empty: HashSet<&str> = HashSet::new();
        assert_eq!(jaccard(&empty, &empty), 0.0);
    }

    #[test]
    fn jaccard_of_identical_sets_is_one() {
        let a: HashSet<&str> = ["x", "y"].into_iter().collect();
        assert_eq!(jaccard(&a, &a), 1.0);
    }
}
