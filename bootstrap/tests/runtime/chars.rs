use crate::harness::exec;

#[test]
fn char_literal_basic() {
    exec(
        r#"
let main = fn () -> unit do
    let _c = 'a'
    return ()
end
"#,
    );
}

#[test]
fn char_literal_escape_sequences() {
    exec(
        r#"
let main = fn () -> unit do
    let _newline = '\n'
    let _tab = '\t'
    let _null = '\0'
    let _backslash = '\\'
    let _quote = '\''
    return ()
end
"#,
    );
}

#[test]
fn char_eq() {
    exec(
        r#"
let main = fn () -> unit do
    if 'a' == 'a' then () else raise RuntimeError("equal chars should be ==") end
    if 'a' == 'b' then raise RuntimeError("different chars should not be ==") else () end
    return ()
end
"#,
    );
}

#[test]
fn char_ne() {
    exec(
        r#"
let main = fn () -> unit do
    if 'a' != 'b' then () else raise RuntimeError("different chars should be !=") end
    if 'a' != 'a' then raise RuntimeError("equal chars should not be !=") else () end
    return ()
end
"#,
    );
}

#[test]
fn char_comparison() {
    exec(
        r#"
let main = fn () -> unit do
    if 'a' < 'b' then () else raise RuntimeError("'a' should be < 'b'") end
    if 'b' > 'a' then () else raise RuntimeError("'b' should be > 'a'") end
    if 'a' <= 'a' then () else raise RuntimeError("'a' should be <= 'a'") end
    if 'a' >= 'a' then () else raise RuntimeError("'a' should be >= 'a'") end
    return ()
end
"#,
    );
}

#[test]
fn char_in_function_param() {
    exec(
        r#"
let is_a = fn (c: char) -> bool do
    return c == 'a'
end

let main = fn () -> unit do
    if is_a(c: 'a') then () else raise RuntimeError("is_a('a') should be true") end
    if is_a(c: 'b') then raise RuntimeError("is_a('b') should be false") else () end
    return ()
end
"#,
    );
}

#[test]
fn char_match() {
    exec(
        r#"
let main = fn () -> unit do
    let c = 'x'
    match c do
        | 'a' -> raise RuntimeError("should not match 'a'")
        | 'x' -> ()
        | _ -> raise RuntimeError("should not match wildcard")
    end
    return ()
end
"#,
    );
}

#[test]
fn char_unicode() {
    exec(
        r#"
let main = fn () -> unit do
    let _c = '\u{1F600}'
    return ()
end
"#,
    );
}
