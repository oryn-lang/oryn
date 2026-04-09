mod common;
use common::run;

// --- If / else / elif ---

#[test]
fn if_true_runs_body() {
    assert_eq!(run("if true { print(1) }"), "1\n");
}

#[test]
fn if_false_skips_body() {
    assert_eq!(run("if false { print(1) }\nprint(2)"), "2\n");
}

#[test]
fn if_else_takes_else_when_false() {
    assert_eq!(run("if false { print(1) } else { print(2) }"), "2\n");
}

#[test]
fn if_else_takes_if_when_true() {
    assert_eq!(run("if true { print(1) } else { print(2) }"), "1\n");
}

#[test]
fn elif_takes_first_true_branch() {
    assert_eq!(
        run("if false { print(1) } elif true { print(2) } else { print(3) }"),
        "2\n",
    );
}

#[test]
fn elif_falls_through_to_else() {
    assert_eq!(
        run("if false { print(1) } elif false { print(2) } else { print(3) }"),
        "3\n",
    );
}

#[test]
fn if_with_condition_expression() {
    assert_eq!(
        run("let x = 5\nif x > 3 { print(1) } else { print(2) }"),
        "1\n"
    );
}

#[test]
fn if_block_runs_multiple_statements() {
    assert_eq!(
        run("let x = 1\nif true {\nx = x + 1\nx = x + 1\nprint(x)\n}"),
        "3\n",
    );
}

#[test]
fn else_block_runs_multiple_statements() {
    assert_eq!(
        run("let x = 1\nif false {\nprint(0)\n} else {\nx = x + 10\nx = x + 5\nprint(x)\n}"),
        "16\n",
    );
}

#[test]
fn elif_block_runs_multiple_statements() {
    assert_eq!(
        run(
            "let x = 0\nif false {\nprint(1)\n} elif true {\nx = x + 7\nx = x + 3\nprint(x)\n} else {\nprint(99)\n}"
        ),
        "10\n",
    );
}

#[test]
fn elif_chain() {
    assert_eq!(
        run(
            "let x = 3\nif x == 1 { print(1) } elif x == 2 { print(2) } elif x == 3 { print(3) } else { print(4) }"
        ),
        "3\n",
    );
}

// --- While loops ---

#[test]
fn while_loop() {
    assert_eq!(
        run("let x = 0\nwhile x < 3 {\nx = x + 1\n}\nprint(x)"),
        "3\n",
    );
}

#[test]
fn while_false_skips_body() {
    assert_eq!(run("while false {\nprint(1)\n}\nprint(2)"), "2\n");
}

#[test]
fn while_with_print_each_iteration() {
    assert_eq!(
        run("let i = 0\nwhile i < 3 {\nprint(i)\ni = i + 1\n}"),
        "0\n1\n2\n",
    );
}

#[test]
fn break_exits_loop() {
    assert_eq!(
        run("let i = 0\nwhile true {\nif i == 3 { break }\ni = i + 1\n}\nprint(i)"),
        "3\n",
    );
}

#[test]
fn continue_skips_rest_of_body() {
    // Print only odd numbers: skip even ones with continue.
    assert_eq!(
        run(
            "let i = 0\nwhile i < 5 {\ni = i + 1\nif i == 2 { continue }\nif i == 4 { continue }\nprint(i)\n}"
        ),
        "1\n3\n5\n",
    );
}

#[test]
fn break_in_nested_if() {
    // break inside an if inside a while should exit the while.
    assert_eq!(
        run("let x = 0\nwhile true {\nx = x + 1\nif x > 5 {\nbreak\n}\n}\nprint(x)"),
        "6\n",
    );
}
