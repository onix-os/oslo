//! The order the finder lists things in, which is the whole of its behaviour.

use super::*;

fn command(line: &str, runs: i64, last_at: i64, dir: &str) -> Command {
    Command {
        line: line.to_string(),
        mode: "sh".to_string(),
        runs,
        last_at,
        dir: dir.to_string(),
        places: 1,
        worked: true,
        session: String::new(),
        host: String::new(),
        root: None,
    }
}

fn lines(ranked: &[Ranked]) -> Vec<&str> {
    ranked.iter().map(|r| r.command.line.as_str()).collect()
}

/// With nothing typed the finder is a history list, newest first — which is the order the store
/// already returns, so nothing is re-sorted and the thing you just did is at the top.
#[test]
fn an_empty_query_keeps_the_order_it_was_given() {
    let commands = [
        command("newest", 1, 300, "/a"),
        command("middle", 9, 200, "/a"),
        command("oldest", 5, 100, "/a"),
    ];
    let ranked = rank(&commands, "", "/a", Fuzzy::Smart);
    assert_eq!(lines(&ranked), ["newest", "middle", "oldest"]);
}

/// **The query filters; recency orders.** A match typed once an hour ago comes before one typed
/// five hundred times last week, because the thing you most often want is the thing you just did.
#[test]
fn the_most_recent_match_comes_first() {
    let commands = [
        command("git stash pop", 500, 100, "/a"),
        command("gs", 1, 200, "/a"),
    ];
    let ranked = rank(&commands, "gs", "/a", Fuzzy::Smart);
    assert_eq!(ranked[0].command.line, "gs", "{:?}", lines(&ranked));
}

/// Among commands last run at the same moment, the habit wins.
#[test]
fn frequency_breaks_a_tie_between_equally_recent_commands() {
    let commands = [
        command("git stall", 1, 500, "/a"),
        command("git status", 300, 500, "/a"),
    ];
    let ranked = rank(&commands, "git sta", "/a", Fuzzy::Smart);
    assert_eq!(ranked[0].command.line, "git status", "{:?}", lines(&ranked));
}

/// **The score no longer orders anything, and this is the case that made that necessary.**
///
/// Typing `cd` used to put `cd docs/` — run twice, a day ago — above `cd rush`, run twenty-three
/// times two hours ago, because the scorer preferred one string's shape over the other's. Every
/// row in the list already matches by the time it is sorted; "how well" is then a number about
/// strings rather than about what you want next, and the order it produces reads as arbitrary
/// because it is.
#[test]
fn a_stale_match_does_not_outrank_a_fresh_one() {
    let commands = [
        command("cd docs/", 2, 100, "/a"),
        command("cd rush", 23, 500, "/a"),
        command("cd bin", 2, 900, "/a"),
    ];
    let ranked = rank(&commands, "cd", "/a", Fuzzy::Smart);
    assert_eq!(
        lines(&ranked),
        ["cd bin", "cd rush", "cd docs/"],
        "newest first, whatever the scorer thinks of the strings"
    );
}

/// Filtering still filters: recency orders what matched, and cannot promote what did not.
#[test]
fn recency_orders_the_matches_and_nothing_else() {
    let commands = [
        command("npm install", 1, 900, "/a"),
        command("cargo test", 1, 100, "/a"),
    ];
    let ranked = rank(&commands, "cargo", "/a", Fuzzy::Smart);
    assert_eq!(
        lines(&ranked),
        ["cargo test"],
        "the newer one did not match"
    );
}

/// And where you are breaks the tie after that.
#[test]
fn the_working_directory_breaks_the_next_tie() {
    let commands = [
        command("make build", 10, 100, "/elsewhere"),
        command("make build", 10, 100, "/here"),
    ];
    let ranked = rank(&commands, "make", "/here", Fuzzy::Smart);
    assert!(ranked[0].here, "the local one should come first");
    assert_eq!(ranked[0].command.dir, "/here");
}

/// "Here" includes anywhere under the recorded directory: a command run in a repository root is
/// still local when you have stepped into a crate inside it.
#[test]
fn a_parent_directory_counts_as_here() {
    assert!(is_here("/home/me/proj", "/home/me/proj"));
    assert!(is_here("/home/me/proj", "/home/me/proj/crates/api"));
    // Only on a path boundary, or `proj` would claim `project`.
    assert!(!is_here("/home/me/proj", "/home/me/project"));
    assert!(!is_here("/home/me/other", "/home/me/proj"));
    assert!(!is_here("", "/home/me"));
    assert!(!is_here("/home/me", ""));
}

/// Anything the query does not match is not in the list at all. A finder that showed everything
/// and merely reordered it would make the query pointless.
#[test]
fn a_query_filters_rather_than_reorders() {
    let commands = [
        command("cargo build", 1, 100, "/a"),
        command("npm install", 1, 100, "/a"),
    ];
    let ranked = rank(&commands, "cargo", "/a", Fuzzy::Smart);
    assert_eq!(lines(&ranked), ["cargo build"]);
}

/// The order is total: two runs over the same input must list identically, or the selection jumps
/// under the cursor between one opening and the next.
#[test]
fn the_order_is_stable() {
    let commands = [
        command("same", 1, 100, "/a"),
        command("also", 1, 100, "/a"),
        command("more", 1, 100, "/a"),
    ];
    let first = lines(&rank(&commands, "", "/a", Fuzzy::Smart)).join(",");
    for _ in 0..20 {
        assert_eq!(
            lines(&rank(&commands, "", "/a", Fuzzy::Smart)).join(","),
            first
        );
    }
}

/// The age column, at every boundary it crosses.
#[test]
fn ages_read_as_the_shortest_true_thing() {
    let now = 1_000_000_000;
    assert_eq!(ago(now, now), "now");
    assert_eq!(ago(now, now - 59), "now");
    assert_eq!(ago(now, now - 60), "1m");
    assert_eq!(ago(now, now - 3_599), "59m");
    assert_eq!(ago(now, now - 3_600), "1h");
    assert_eq!(ago(now, now - 86_399), "23h");
    assert_eq!(ago(now, now - 86_400), "1d");
    assert_eq!(ago(now, now - 7 * 86_400), "1w");
    assert_eq!(ago(now, now - 30 * 86_400), "1mo");
    assert_eq!(ago(now, now - 365 * 86_400), "1y");
    // A clock that went backwards must not print a negative.
    assert_eq!(ago(now, now + 500), "now");
}
