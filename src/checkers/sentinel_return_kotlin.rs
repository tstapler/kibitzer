use super::sentinel_return::{LangSpec, sentinel_checker};
use crate::checker::Language;

sentinel_checker!(
    "sentinel-return-kotlin",
    Language::Kotlin,
    &["**/*.kt", "**/*.kts"],
    LangSpec {
        check_name: "sentinel-return-kotlin",
        catch_kinds: &["catch_block"],
        function_kinds: &[
            "function_declaration",
            "lambda_literal",
            "anonymous_function"
        ],
        return_kinds: &["return_expression"],
        nullable_type_markers: &["?"],
    },
    "flags `return -1`/`return null` inside a catch block (Replace Error Code with Exception)"
);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<crate::checker::Finding> {
        crate::test_support::check_kotlin_source(&SentinelReturnChecker, src).unwrap()
    }

    #[test]
    fn flags_negative_return_in_catch() {
        assert_eq!(
            run("fun f(): Int { try { g() } catch (e: Exception) { return -1 }; return 0 }").len(),
            1
        );
    }

    #[test]
    fn allows_throw_in_catch() {
        assert!(run("fun f(): Int { try { g() } catch (e: Exception) { throw IllegalStateException(e) }; return 0 }").is_empty());
    }

    #[test]
    fn allows_null_when_return_type_is_nullable() {
        assert!(
            run(
                "fun f(): String? { try { g() } catch (e: Exception) { return null }; return \"\" }"
            )
            .is_empty()
        );
    }
}
