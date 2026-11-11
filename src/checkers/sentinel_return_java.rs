use super::sentinel_return::{LangSpec, sentinel_checker};
use crate::checker::Language;

sentinel_checker!(
    "sentinel-return-java",
    Language::Java,
    &["**/*.java"],
    LangSpec {
        check_name: "sentinel-return-java",
        catch_kinds: &["catch_clause"],
        function_kinds: &[
            "method_declaration",
            "constructor_declaration",
            "lambda_expression"
        ],
        return_kinds: &["return_statement"],
        nullable_type_markers: &[],
    },
    "flags `return -1`/`return null` inside a catch block (Replace Error Code with Exception)"
);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<crate::checker::Finding> {
        crate::test_support::check_java_source(&SentinelReturnChecker, src).unwrap()
    }

    #[test]
    fn flags_negative_return_in_catch() {
        assert_eq!(
            run(
                "class A { int f() { try { g(); } catch (Exception e) { return -1; } return 0; } }"
            )
            .len(),
            1
        );
    }

    #[test]
    fn flags_null_return_in_catch() {
        assert_eq!(run("class A { String f() { try { g(); } catch (Exception e) { return null; } return \"\"; } }").len(), 1);
    }

    #[test]
    fn allows_throw_in_catch() {
        assert!(run("class A { int f() { try { g(); } catch (Exception e) { throw new RuntimeException(e); } return 0; } }").is_empty());
    }

    #[test]
    fn ignores_sentinel_outside_catch() {
        assert!(run("class A { int f() { if (x) { return -1; } return 0; } }").is_empty());
    }

    #[test]
    fn ignores_lambda_inside_catch() {
        assert!(run("class A { void f() { try { g(); } catch (Exception e) { Supplier<Integer> s = () -> { return -1; }; } } }").is_empty());
    }

    #[test]
    fn suppresses_search_named_methods() {
        assert!(run("class A { int indexOfThing() { try { g(); } catch (Exception e) { return -1; } return 0; } }").is_empty());
    }

    #[test]
    fn suppresses_factory_null() {
        assert!(run("class A { A createA() { try { g(); } catch (Exception e) { return null; } return new A(); } }").is_empty());
    }

    #[test]
    fn suppresses_try_prefixed_and_read_methods() {
        assert!(run("class A { A tryParse() { try { g(); } catch (Exception e) { return null; } return null; } }").is_empty());
        assert!(run("class A { int read() { try { g(); } catch (Exception e) { return -1; } return 0; } }").is_empty());
    }
}
