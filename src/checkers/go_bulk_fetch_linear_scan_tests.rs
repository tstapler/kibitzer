use super::*;

#[test]
fn flags_the_profiled_stapler_squad_shape() {
    // The exact shape from issue #30: bulk List* fetch, range, compare an element
    // field against a parameter, return on match.
    let findings = check_source(
        "package main\n\
         func (s *Store) FindInstanceDataByID(id string) (*Data, error) {\n\
         \tall, err := s.ListInstanceData()\n\
         \tif err != nil {\n\
         \t\treturn nil, err\n\
         \t}\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].message.contains("`all`"));
}

#[test]
fn flags_comparison_order_swapped() {
    let findings = check_source(
        "package main\n\
         func FindX(id string) (*T, error) {\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif id == item.ID {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
}

#[test]
fn flags_get_all_and_find_all_prefixes() {
    for prefix in ["GetAllFoo", "FindAllBar"] {
        let findings = check_source(&format!(
            "package main\n\
             func FindX(id string) (*T, error) {{\n\
             \tall, err := {prefix}()\n\
             \tif err != nil {{ return nil, err }}\n\
             \tfor _, item := range all {{\n\
             \t\tif item.ID == id {{\n\
             \t\t\treturn &item, nil\n\
             \t\t}}\n\
             \t}}\n\
             \treturn nil, ErrNotFound\n\
             }}\n"
        ))
        .unwrap();
        assert_eq!(findings.len(), 1, "prefix {prefix} should flag");
    }
}

#[test]
fn ignores_non_bulk_fetch_name() {
    // Doesn't start with any of List/GetAll/FindAll at all.
    let findings = check_source(
        "package main\n\
         func FindX(id string) (*T, error) {\n\
         \tall, err := loadCandidates()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_list_prefixed_name_without_uppercase_boundary() {
    // The actual uppercase-boundary case `is_bulk_fetch_name` exists to handle:
    // "Listener" starts with "List", but the next char ('e') isn't uppercase, so
    // it's not a List*-shaped bulk fetch.
    let findings = check_source(
        "package main\n\
         func FindX(id string) (*T, error) {\n\
         \tall, err := Listener()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_comparison_against_non_parameter() {
    // Compares against a local, not one of the function's own parameters — not
    // the single-key lookup shape this check targets.
    let findings = check_source(
        "package main\n\
         func FindX(id string) (*T, error) {\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \twant := \"fixed\"\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == want {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_loop_with_no_early_return() {
    // A range that accumulates/aggregates over every element (no per-match early
    // return) isn't the "find one row" shape this check targets.
    let findings = check_source(
        "package main\n\
         func SumX(id string) int {\n\
         \tall, _ := ListAllX()\n\
         \ttotal := 0\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\ttotal += item.Amount\n\
         \t\t}\n\
         \t}\n\
         \treturn total\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_inequality_comparison() {
    let findings = check_source(
        "package main\n\
         func FindX(id string) (*T, error) {\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID != id {\n\
         \t\t\tcontinue\n\
         \t\t}\n\
         \t\treturn &item, nil\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_range_over_unrelated_slice() {
    let findings = check_source(
        "package main\n\
         func FindX(id string, others []T) (*T, error) {\n\
         \t_, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range others {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn ignores_function_with_no_parameters() {
    let findings = check_source(
        "package main\n\
         func FindDefault() (*T, error) {\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == \"default\" {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert!(findings.is_empty());
}

#[test]
fn reports_only_one_finding_per_function() {
    // Deliberate: a function with two independent offending loops still gets one
    // finding, not two — the first is enough to send someone to look at the
    // function, and reporting every occurrence would just be noise.
    let findings = check_source(
        "package main\n\
         func FindXY(id, id2 string) (*T, error) {\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \tall2, err2 := ListAllY()\n\
         \tif err2 != nil { return nil, err2 }\n\
         \tfor _, item2 := range all2 {\n\
         \t\tif item2.ID == id2 {\n\
         \t\t\treturn &item2, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
}

#[test]
fn flags_the_real_loop_after_an_unrelated_validation_loop() {
    // Realistic shape: a setup/validation loop over an unrelated slice ahead of
    // the actual bulk-fetch-and-scan loop, in the same function.
    let findings = check_source(
        "package main\n\
         func FindX(id string, others []T) (*T, error) {\n\
         \tfor _, o := range others {\n\
         \t\t_ = o\n\
         \t}\n\
         \tall, err := ListAllX()\n\
         \tif err != nil { return nil, err }\n\
         \tfor _, item := range all {\n\
         \t\tif item.ID == id {\n\
         \t\t\treturn &item, nil\n\
         \t\t}\n\
         \t}\n\
         \treturn nil, ErrNotFound\n\
         }\n",
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
}
