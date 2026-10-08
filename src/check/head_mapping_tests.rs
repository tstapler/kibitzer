use super::*;

fn hunk(old_start: usize, old_count: usize, new_start: usize, new_count: usize) -> DiffHunk {
    DiffHunk {
        old_start,
        old_count,
        new_start,
        new_count,
    }
}

#[test]
fn parses_unified_diff_hunk_headers() {
    let diff = "diff --git a/foo.go b/foo.go\n\
                 --- a/foo.go\n\
                 +++ b/foo.go\n\
                 @@ -10,2 +10,5 @@ func Foo() {\n\
                 -old line\n\
                 +new line 1\n";
    let hunks = parse_diff_hunks(diff).unwrap();
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].old_start, 10);
    assert_eq!(hunks[0].old_count, 2);
    assert_eq!(hunks[0].new_start, 10);
    assert_eq!(hunks[0].new_count, 5);
}

#[test]
fn range_before_any_hunk_is_unshifted() {
    let hunks = vec![hunk(20, 1, 20, 6)];
    let mapped = map_ranges_through_hunks(&[(1, 3)], &hunks);
    assert_eq!(mapped, vec![(1, 3)]);
}

#[test]
fn range_after_a_growing_hunk_is_shifted_back() {
    // A 1-line -> 6-line edit at old line 20 pushes everything after it down by 5 in
    // the current file. A changed_lines range of (30, 30) in current-file coordinates
    // must map back to (25, 25) in HEAD.
    let hunks = vec![hunk(20, 1, 20, 6)];
    let mapped = map_ranges_through_hunks(&[(30, 30)], &hunks);
    assert_eq!(mapped, vec![(25, 25)]);
}

#[test]
fn range_inside_the_edited_hunk_maps_to_its_old_span() {
    let hunks = vec![hunk(20, 1, 20, 6)];
    let mapped = map_ranges_through_hunks(&[(21, 23)], &hunks);
    assert_eq!(mapped, vec![(20, 20)]);
}

#[test]
fn range_inside_a_pure_insertion_has_no_head_counterpart() {
    let hunks = vec![hunk(20, 0, 21, 4)];
    let mapped = map_ranges_through_hunks(&[(21, 24)], &hunks);
    assert!(mapped.is_empty());
}
