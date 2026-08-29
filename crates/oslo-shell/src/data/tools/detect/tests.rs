use super::*;

fn value(row: &Record, name: &str) -> String {
    row.get(name)
        .map(crate::data::render_transport)
        .unwrap_or_else(|| "<absent>".to_string())
}

/// The shape `docker ps` prints: a header name with a space in it, and a status with one too.
///
/// The container IDs are twelve characters, as docker writes them — which is what keeps the gap
/// inside `CONTAINER ID` from looking like a column boundary.
#[test]
fn a_two_space_table_keeps_values_with_spaces_whole() {
    let text = "\
CONTAINER ID   IMAGE          STATUS          NAMES
abc123456789   nginx:latest   Up 3 hours      web
def456789abc   redis:7        Exited (0) 2m   cache";
    let rows = detect(text, Layout::default());
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].columns(),
        ["CONTAINER ID", "IMAGE", "STATUS", "NAMES"]
    );
    assert_eq!(value(&rows[0], "STATUS"), "Up 3 hours");
    assert_eq!(value(&rows[1], "STATUS"), "Exited (0) 2m");
    assert_eq!(value(&rows[1], "NAMES"), "cache");
}

/// **`ps aux` is the case a header-only rule gets wrong.** Its names are one space apart because
/// they are right-aligned into narrow columns, so splitting the header on two-or-more spaces reads
/// `PID %CPU %MEM` as one column and shifts every field after it. The data is aligned even where
/// the header is not, and looking at all the lines together finds the real gaps.
#[test]
fn a_right_aligned_header_does_not_shift_the_columns() {
    let text = "\
USER         PID %CPU %MEM COMMAND
root           1  0.0  0.0 /usr/lib/systemd/systemd --switched-root
bo        123456 12.5  3.1 nvim a b";
    let rows = detect(text, Layout::default());
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].columns(),
        ["USER", "PID", "%CPU", "%MEM", "COMMAND"]
    );
    assert_eq!(rows[0].get("PID"), Some(&Val::Int(1)));
    assert_eq!(rows[1].get("PID"), Some(&Val::Int(123_456)));
    assert_eq!(rows[1].get("%CPU"), Some(&Val::Float(12.5)));
    // The last column keeps its spaces: nothing inside it is whitespace on every row.
    assert_eq!(
        value(&rows[0], "COMMAND"),
        "/usr/lib/systemd/systemd --switched-root"
    );
    assert_eq!(value(&rows[1], "COMMAND"), "nvim a b");
}

/// Separated but not aligned — no position is whitespace on every line, so there is nothing to
/// align to and splitting on whitespace is the only thing left.
#[test]
fn unaligned_output_still_splits() {
    let text = "\
USER PID COMMAND
root 1 /sbin/init splash
bo 42 nvim a b";
    let rows = detect(text, Layout::default());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].columns(), ["USER", "PID", "COMMAND"]);
    assert_eq!(rows[0].get("PID"), Some(&Val::Int(1)));
    assert_eq!(value(&rows[0], "COMMAND"), "/sbin/init splash");
    assert_eq!(value(&rows[1], "COMMAND"), "nvim a b");
}

/// A number is a number wherever it came from, so `sort-by` orders it as one.
#[test]
fn numbers_arrive_as_numbers() {
    let rows = detect("N  SIZE\na  10\nb  9", Layout::default());
    assert_eq!(rows[0].get("SIZE"), Some(&Val::Int(10)));
    assert_eq!(rows[1].get("SIZE"), Some(&Val::Int(9)));
}

/// A short line leaves the missing columns **absent**, not blank, so `compact` can tell them apart.
#[test]
fn a_short_line_leaves_columns_absent() {
    let rows = detect("A   B   C\n1   2", Layout::default());
    assert_eq!(value(&rows[0], "B"), "2");
    assert_eq!(value(&rows[0], "C"), "<absent>");
}

/// Without a header the first line is data and the columns are numbered.
#[test]
fn no_headers_numbers_the_columns() {
    let rows = detect(
        "a   1\nb   2",
        Layout {
            no_headers: true,
            skip: 0,
        },
    );
    assert_eq!(rows.len(), 2, "the first line is data too");
    assert_eq!(rows[0].columns(), ["column0", "column1"]);
    assert_eq!(value(&rows[0], "column0"), "a");
    assert_eq!(value(&rows[1], "column1"), "2");
}

/// A banner before the header is skipped rather than becoming the names.
#[test]
fn skip_drops_lines_before_the_header() {
    let rows = detect(
        "some banner\nNAME   AGE\nweb    3d",
        Layout {
            no_headers: false,
            skip: 1,
        },
    );
    assert_eq!(rows[0].columns(), ["NAME", "AGE"]);
    assert_eq!(value(&rows[0], "NAME"), "web");
}

/// Blank lines are not rows, and nothing at all is no rows rather than a panic.
#[test]
fn blank_lines_and_empty_input_are_quiet() {
    assert!(detect("", Layout::default()).is_empty());
    assert!(detect("   \n\n", Layout::default()).is_empty());
    let rows = detect("A   B\n\n1   2\n\n", Layout::default());
    assert_eq!(rows.len(), 1);
}

/// A header and nothing else is a table of no rows, which is not the same as a failure.
#[test]
fn a_header_alone_is_no_rows() {
    assert!(detect("NAME   AGE", Layout::default()).is_empty());
}
