use super::sentinel_return::{LangSpec, sentinel_checker};
use crate::checker::Language;

sentinel_checker!(
    "sentinel-return-typescript",
    Language::TypeScript,
    &["**/*.ts"],
    LangSpec {
        check_name: "sentinel-return-typescript",
        catch_kinds: &["catch_clause"],
        function_kinds: &[
            "function_declaration",
            "function_expression",
            "function",
            "arrow_function",
            "method_definition",
            "generator_function_declaration",
            "generator_function",
        ],
        return_kinds: &["return_statement"],
        nullable_type_markers: &["null", "undefined"],
    },
    "flags `return -1`/`return null` inside a catch block (Replace Error Code with Exception)"
);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<crate::checker::Finding> {
        crate::test_support::check_typescript_source(&SentinelReturnChecker, src).unwrap()
    }

    #[test]
    fn flags_negative_return_in_catch() {
        assert_eq!(
            run("function f() { try { g(); } catch (e) { return -1; } return 0; }").len(),
            1
        );
    }

    #[test]
    fn flags_null_return_in_catch() {
        assert_eq!(
            run("function f() { try { g(); } catch (e) { return null; } return 1; }").len(),
            1
        );
    }

    #[test]
    fn allows_throw_in_catch() {
        assert!(run("function f() { try { g(); } catch (e) { throw new Error('x', { cause: e }); } return 0; }").is_empty());
    }

    #[test]
    fn suppresses_find_named_function() {
        assert!(
            run("function findThing() { try { g(); } catch (e) { return -1; } return 0; }")
                .is_empty()
        );
    }
}
