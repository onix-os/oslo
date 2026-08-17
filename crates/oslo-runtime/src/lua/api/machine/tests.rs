//! The parsing, against the shapes `/proc` actually has.
//!
//! The values themselves are the kernel's and change between reads, so what is asserted is that a
//! real `/proc/meminfo` line is read as the right *number* — the unit being the part everybody gets
//! wrong once.

use super::super::util::probe;
use super::*;

/// A trimmed-down `/proc/meminfo`, spaced as the kernel writes it.
const MEMINFO: &str = "MemTotal:       61201640 kB\n\
                       MemFree:         1849284 kB\n\
                       MemAvailable:   41411216 kB\n\
                       Buffers:          123456 kB\n\
                       SwapTotal:      75497464 kB\n\
                       SwapFree:       54408412 kB\n";

/// **Kibibytes in, bytes out.** `/proc/meminfo` says `kB` on every line and means KiB; a caller
/// comparing against a threshold in bytes would be out by 1024 without this.
#[test]
fn meminfo_is_read_as_bytes() {
    assert_eq!(kilobytes(MEMINFO, "MemTotal"), Some(61_201_640 * 1024));
    assert_eq!(kilobytes(MEMINFO, "MemAvailable"), Some(41_411_216 * 1024));
    assert_eq!(kilobytes(MEMINFO, "SwapFree"), Some(54_408_412 * 1024));
}

/// A field that is not there is `None`, not zero — a machine with no swap and a kernel that does
/// not report swap are different states, and zero would say the first about both.
#[test]
fn an_absent_field_is_not_zero() {
    assert_eq!(kilobytes(MEMINFO, "HugePages_Total"), None);
    assert_eq!(kilobytes("", "MemTotal"), None);
    // A prefix of a real name must not match it.
    assert_eq!(kilobytes(MEMINFO, "Mem"), None);
}

#[test]
fn a_missing_file_answers_nil_rather_than_raising() {
    assert!(matches!(first_line("/proc/nothing-here"), Value::Nil));
    assert_eq!(read_at("/proc/nothing-here", 0), None);
}

/// Everything answers on this machine, and answers the right *kind* of thing.
#[test]
fn every_fact_is_readable_here() {
    let mut sys = Table::new();
    install(&mut sys);
    let sys = Value::table(sys);

    assert!(matches!(
        probe::first(&probe::field(&sys, "cpus"), Vec::new()),
        Value::Number(_)
    ));
    assert!(matches!(
        probe::first(&probe::field(&sys, "arch"), Vec::new()),
        Value::Str(_)
    ));
    // These read `/proc`, which is there on Linux and is the only platform oslo builds for.
    for name in ["kernel", "uptime", "loadavg", "memory"] {
        let answered = probe::first(&probe::field(&sys, name), Vec::new());
        assert!(
            !matches!(answered, Value::Nil),
            "{name} answered nil on a machine with /proc"
        );
    }

    // The load average is three numbers, newest window first.
    let Value::Table(load) = probe::first(&probe::field(&sys, "loadavg"), Vec::new()) else {
        panic!("loadavg is not a table")
    };
    assert_eq!(load.borrow().sequence().len(), 3);

    // Memory is bytes, and a machine has more than a megabyte of it.
    let memory = probe::first(&probe::field(&sys, "memory"), Vec::new());
    let Value::Table(memory) = memory else {
        panic!("memory is not a table")
    };
    match memory.borrow().get_str("total") {
        Value::Number(n) => assert!(
            n.as_int().unwrap_or(0) > 1024 * 1024,
            "total looks like kibibytes rather than bytes"
        ),
        other => panic!("total is {}", other.type_name()),
    }
}
