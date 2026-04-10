mod comments;
mod printer;
mod session;

pub use session::format_source;

#[cfg(test)]
mod tests {
    use super::format_source;

    #[test]
    fn preserves_leading_comments() {
        let source = "// This is a greeting\nlet x = 5";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "// This is a greeting\nlet x = 5\n");
    }

    #[test]
    fn preserves_trailing_comments() {
        let source = "let x = 5 // important value";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "let x = 5  // important value\n");
    }

    #[test]
    fn preserves_section_comments() {
        let source = "// --- section 1 ---\nlet x = 5\n\n// --- section 2 ---\nlet y = 10";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "// --- section 1 ---\nlet x = 5\n\n// --- section 2 ---\nlet y = 10\n"
        );
    }

    #[test]
    fn preserves_comments_in_blocks() {
        let source = "fn foo() {\n    // inside\n    let x = 5\n}";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "fn foo() {\n    // inside\n    let x = 5\n}\n");
    }

    #[test]
    fn preserves_comments_before_object_members() {
        let source = "obj Foo {\n    // docs\n    x: int\n\n    // impl\n    fn bar() {}\n}";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "obj Foo {\n    // docs\n    x: int\n\n    // impl\n    fn bar() {\n    }\n}\n"
        );
    }

    #[test]
    fn formats_function_and_if() {
        let source = "fn foo(x:int,y:int)->int{if x<y { rn x } else { rn y }}";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "fn foo(x: int, y: int) -> int {\n    if x < y {\n        rn x\n    } else {\n        rn y\n    }\n}\n"
        );
    }

    #[test]
    fn formats_objects_and_static_methods() {
        let source = "obj Vec2{x:int y:int fn len()->int{rn x*x+y*y}}\nmath.sqrt(4)";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "obj Vec2 {\n    x: int\n    y: int\n\n    fn len() -> int {\n        rn x * x + y * y\n    }\n}\n\nmath.sqrt(4)\n"
        );
    }

    #[test]
    fn formats_for_and_ranges() {
        let source = "for i in 0..=3 {print(i)}";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "for i in 0..=3 {\n    print(i)\n}\n");
    }

    #[test]
    fn formats_if_let() {
        let source = "if let x=maybe_val(){print(x)}";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "if let x = maybe_val() {\n    print(x)\n}\n");
    }

    #[test]
    fn formats_if_let_with_else() {
        let source = "if let x=maybe_val(){print(x)} else {print(0)}";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "if let x = maybe_val() {\n    print(x)\n} else {\n    print(0)\n}\n"
        );
    }

    #[test]
    fn preserves_elif_syntax() {
        let source = "if x { print(1) } else { if y { print(2) } else { print(3) } }";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "if x {\n    print(1)\n} elif y {\n    print(2)\n} else {\n    print(3)\n}\n"
        );
    }

    #[test]
    fn formats_unless() {
        let source = "unless ready {print(0)}";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "unless ready {\n    print(0)\n}\n");
    }

    #[test]
    fn formats_unless_with_else() {
        let source = "unless ready {print(0)} else {print(1)}";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "unless ready {\n    print(0)\n} else {\n    print(1)\n}\n"
        );
    }
}
