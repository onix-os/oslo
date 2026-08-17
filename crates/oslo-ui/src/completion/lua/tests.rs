//! Reading a Lua line backwards from the cursor, which is the half of completion that is text.

use super::{Typed, candidates, set_name_source, typed_at};

#[track_caller]
fn reads(line: &str, path: &[&str], stem: &str, method: bool) {
    let got = typed_at(line, line.len()).unwrap_or_else(|| panic!("{line:?} read as nothing"));
    let wanted = Typed {
        path: path.iter().map(|s| (*s).to_string()).collect(),
        stem: stem.to_string(),
        at: line.len() - stem.len(),
        method,
    };
    assert_eq!(got, wanted, "for {line:?}");
}

/// A bare word is a global, and a dotted one names the table it comes from.
#[test]
fn a_name_carries_the_table_it_belongs_to() {
    reads("pri", &[], "pri", false);
    reads("oslo.", &["oslo"], "", false);
    reads("oslo.ma", &["oslo"], "ma", false);
    reads("oslo.math.ev", &["oslo", "math"], "ev", false);
    reads("print(oslo.js", &["oslo"], "js", false);
    reads("local x = fs.re", &["fs"], "re", false);
}

/// **A colon is a method call**, and only the first separator may be one.
#[test]
fn a_colon_asks_for_a_method() {
    reads("s:ev", &["s"], "ev", true);
    reads("s:", &["s"], "", true);
    // `a:b:c` is not callable Lua, so it is not completed as though it were.
    assert_eq!(typed_at("a:b:c", 5), None);
}

/// **Nothing is completed inside text**, because a name there is not a name.
#[test]
fn a_string_or_a_comment_is_not_a_name() {
    assert_eq!(typed_at("print(\"pri", 10), None);
    assert_eq!(typed_at("print('os", 9), None);
    assert_eq!(typed_at("-- pri", 6), None);
    assert_eq!(typed_at("x = [[ os", 9), None);
    // A closed string leaves the cursor back in code, where a name is a name again.
    reads("print(\"hi\") ; pri", &[], "pri", false);
    // An escaped quote does not close the string it is in.
    assert_eq!(typed_at("x = \"a\\\" pri", 12), None);
}

/// A number is not the start of a name, so `2x` offers nothing.
#[test]
fn a_name_cannot_begin_with_a_digit() {
    assert_eq!(typed_at("2x", 2), None);
    assert_eq!(
        typed_at("x2", 2),
        Some(Typed {
            path: vec![],
            stem: "x2".to_string(),
            at: 0,
            method: false,
        })
    );
}

/// **Keywords complete, and only where one could be written.** After a dot, `local` is not a field
/// of anything, so offering it would be offering a syntax error.
#[test]
fn keywords_complete_only_in_the_open() {
    set_name_source(None);
    let (_, open) = candidates("fun", 3).expect("a name");
    assert!(
        open.iter().any(|c| c.display == "function"),
        "{:?}",
        open.iter().map(|c| &c.display).collect::<Vec<_>>()
    );

    let (_, after_dot) = candidates("oslo.fun", 8).expect("a name");
    assert!(
        after_dot.is_empty(),
        "no keyword is a field: {:?}",
        after_dot.iter().map(|c| &c.display).collect::<Vec<_>>()
    );

    // An empty stem does not pour the whole keyword list onto the screen.
    let (_, nothing) = candidates("", 0).expect("a name");
    assert!(nothing.is_empty(), "{nothing:?}");
}

/// The names the runtime supplies are filtered by what has been typed, and marked by what they are.
#[test]
fn supplied_names_are_filtered_and_labelled() {
    set_name_source(Some(std::rc::Rc::new(|path: &[String]| {
        if path == ["oslo"] {
            vec![("math".into(), false), ("run".into(), true)]
        } else {
            vec![("print".into(), true), ("pairs".into(), true)]
        }
    })));

    let (at, out) = candidates("oslo.r", 6).expect("a name");
    assert_eq!(at, 5, "replace from after the dot");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].display, "run");
    assert_eq!(out[0].kind.as_deref(), Some("function"));

    let (_, globals) = candidates("pri", 3).expect("a name");
    assert!(globals.iter().any(|c| c.display == "print"));
    assert!(!globals.iter().any(|c| c.display == "pairs"), "filtered");

    // A method must be callable: `oslo:math` is not a thing that can be called.
    let (_, methods) = candidates("oslo:", 5).expect("a name");
    assert_eq!(
        methods.iter().map(|c| &c.display).collect::<Vec<_>>(),
        vec!["run"],
        "only the callable one"
    );
    set_name_source(None);
}
