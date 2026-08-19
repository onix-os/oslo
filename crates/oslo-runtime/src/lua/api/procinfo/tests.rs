//! The `/proc` parsing, against the shapes the kernel actually writes, and against this process —
//! which is the one process a test can be sure exists and can be sure of the answers for.

use super::super::util::probe;
use super::*;

/// A trimmed `/proc/<pid>/status`, tabbed as the kernel writes it.
const STATUS: &str = "Name:\tbash\n\
                      Umask:\t0022\n\
                      State:\tS (sleeping)\n\
                      Tgid:\t4242\n\
                      Pid:\t4242\n\
                      PPid:\t4200\n\
                      Uid:\t1000\t1000\t1000\t1000\n\
                      VmSize:\t   21360 kB\n\
                      VmRSS:\t    5544 kB\n\
                      Threads:\t1\n";

#[test]
fn a_status_field_is_read_by_its_exact_name() {
    assert_eq!(field(STATUS, "Name"), Some("bash"));
    assert_eq!(field(STATUS, "PPid"), Some("4200"));
    // A prefix of a real name must not match it — `Pid` and `PPid` are different fields, and
    // `Tgid` sits between them.
    assert_eq!(field(STATUS, "Pid"), Some("4242"));
    assert_eq!(field(STATUS, "Nothing"), None);
}

/// **Kibibytes in, bytes out**, so `if p.rss > 1e9` is written in the unit anybody would write.
#[test]
fn memory_fields_are_bytes() {
    assert!(
        matches!(bytes_field(STATUS, "VmRSS"), Value::Number(n) if n.as_int() == Some(5544 * 1024))
    );
    assert!(
        matches!(bytes_field(STATUS, "VmSize"), Value::Number(n) if n.as_int() == Some(21360 * 1024))
    );
    // A field the kernel omits — a kernel thread has no `VmRSS` — is nil, not zero.
    assert!(matches!(bytes_field(STATUS, "VmPeak"), Value::Nil));
}

/// The `Uid:` line carries four ids; the first is the real one, which is who owns the process.
#[test]
fn the_owner_is_the_first_of_the_four_ids() {
    assert!(matches!(first_number(STATUS, "Uid"), Value::Number(n) if n.as_int() == Some(1000)));
}

/// **A letter means nothing at a call site.** `p.state == "zombie"` reads; `p.state == "Z"` sends
/// the reader to `proc(5)`.
#[test]
fn the_state_letter_becomes_a_word() {
    for (letter, word) in [
        ("R (running)", "running"),
        ("S (sleeping)", "sleeping"),
        ("D (disk sleep)", "waiting"),
        ("Z (zombie)", "zombie"),
        ("T (stopped)", "stopped"),
        ("t (tracing stop)", "traced"),
        ("I (idle)", "idle"),
        ("Q (nonsense)", "other"),
    ] {
        let status = format!("State:\t{letter}\n");
        match state_of(&status) {
            Value::Str(name) => assert_eq!(name.as_ref(), word, "for {letter}"),
            other => panic!("{letter} gave {}", other.type_name()),
        }
    }
}

/// **This process, which is the one a test knows the answers for.**
#[test]
fn reading_this_process_answers_what_is_true_of_it() {
    let mut proc = Table::new();
    install(&mut proc);
    let proc = Value::table(proc);

    let me = std::process::id() as i64;
    let info = probe::first(&probe::field(&proc, "info"), vec![Value::int(me)]);
    let Value::Table(info) = info else {
        panic!("info did not answer a table")
    };
    let info = info.borrow();

    assert!(matches!(info.get_str("pid"), Value::Number(n) if n.as_int() == Some(me)));
    assert!(matches!(info.get_str("name"), Value::Str(_)));
    assert!(matches!(info.get_str("state"), Value::Str(_)));
    // A test binary has a parent, at least one thread, and some resident memory.
    assert!(matches!(info.get_str("ppid"), Value::Number(n) if n.as_int().unwrap_or(0) > 0));
    assert!(matches!(info.get_str("threads"), Value::Number(n) if n.as_int().unwrap_or(0) >= 1));
    assert!(matches!(info.get_str("rss"), Value::Number(n) if n.as_int().unwrap_or(0) > 0));
    // Ours, so the links are readable.
    assert!(matches!(info.get_str("exe"), Value::Str(_)));
    assert!(matches!(info.get_str("cwd"), Value::Str(_)));
    // `argv` is a list, and `command` is it joined — the test binary was run with at least its own
    // name.
    let Value::Table(argv) = info.get_str("argv") else {
        panic!("argv is not a table")
    };
    assert!(!argv.borrow().sequence().is_empty());
}

/// A process that is not there is a message, not a raise — a pid is a thing that stops existing
/// between when you read it and when you ask about it.
#[test]
fn a_process_that_is_gone_is_a_message() {
    let mut proc = Table::new();
    install(&mut proc);
    let proc = Value::table(proc);

    // Above `pid_max`'s default ceiling, so it cannot be a live process.
    let answered = probe::call(&probe::field(&proc, "info"), vec![Value::int(9_999_999)])
        .expect("should answer rather than raise");
    assert!(matches!(answered.first(), Some(Value::Nil)));
    assert!(answered.len() > 1, "no message beside the nil");
}

#[test]
fn something_that_is_not_a_pid_is_refused() {
    let mut proc = Table::new();
    install(&mut proc);
    let proc = Value::table(proc);
    for bad in [
        Value::str("init"),
        Value::int(0),
        Value::int(-1),
        Value::Nil,
    ] {
        let refused = probe::call(&probe::field(&proc, "info"), vec![bad.clone()])
            .expect_err("should refuse");
        assert!(refused.to_string().contains("process id"), "{refused}");
    }
}

/// **Children are found by scanning**, because the kernel records only the edge upwards. The child
/// this spawns is one of ours, so it is certain to be in the answer.
#[test]
fn a_spawned_child_is_found_under_this_process() {
    let mut proc = Table::new();
    install(&mut proc);
    let proc = Value::table(proc);

    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");

    let me = std::process::id() as i64;
    let found = probe::first(&probe::field(&proc, "children"), vec![Value::int(me)]);
    let Value::Table(found) = found else {
        panic!("children did not answer a table")
    };
    let mine: Vec<i64> = found
        .borrow()
        .sequence()
        .iter()
        .filter_map(|entry| {
            let Value::Table(entry) = entry else {
                return None;
            };
            let pid = entry.borrow().get_str("pid");
            pid.as_number()?.as_int()
        })
        .collect();

    let _ = child.kill();
    let _ = child.wait();
    assert!(
        mine.contains(&(child.id() as i64)),
        "the child this test started was not among {mine:?}"
    );
}
