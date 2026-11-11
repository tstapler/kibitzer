use super::sentinel_return::{LangSpec, sentinel_checker};
use crate::checker::Language;

sentinel_checker!(
    "sentinel-return-python",
    Language::Python,
    &["**/*.py"],
    LangSpec {
        check_name: "sentinel-return-python",
        catch_kinds: &["except_clause", "except_group_clause"],
        function_kinds: &["function_definition", "lambda"],
        return_kinds: &["return_statement"],
        nullable_type_markers: &["Optional", "None"],
    },
    "flags `return -1`/`return None` inside an except block (Replace Error Code with Exception)"
);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<crate::checker::Finding> {
        crate::test_support::check_python_source(&SentinelReturnChecker, src).unwrap()
    }

    #[test]
    fn flags_negative_return_in_except() {
        assert_eq!(run("def f():\n    try:\n        g()\n    except ValueError:\n        return -1\n    return 0\n").len(), 1);
    }

    #[test]
    fn flags_none_return_in_except() {
        assert_eq!(run("def f():\n    try:\n        g()\n    except ValueError:\n        return None\n    return 1\n").len(), 1);
    }

    #[test]
    fn allows_raise_in_except() {
        assert!(run("def f():\n    try:\n        g()\n    except ValueError as e:\n        raise RuntimeError('x') from e\n    return 0\n").is_empty());
    }

    #[test]
    fn suppresses_optional_return_type() {
        assert!(run("def f() -> Optional[int]:\n    try:\n        g()\n    except ValueError:\n        return None\n    return 1\n").is_empty());
    }
}
