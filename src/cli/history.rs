pub mod help;

use crate::cli::help::Paint;
use oslo::track::{
    EventId, HistoryCompletion, HistoryEvent, HistoryFilter, HistoryMatch, HistorySegment,
    HistoryStatus, Track, status_file, sync_files, verify_file,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::str::FromStr;

/// The one complaint that is about the *choice* of subcommand rather than about its arguments, so
/// that it can be said in the words every other tool says it in.
const NO_SUCH: &str = "no such subcommand";

pub fn run(args: &[String]) -> i32 {
    match execute(args) {
        Ok(()) => 0,
        // **A mistake about the words gets the page for the subcommand it was made in**, then the
        // complaint — which is the shape every other tool's mistakes have. Anything else is a
        // failure at the work rather than at the asking, and a help page would be noise on top of
        // it.
        Err(error) => match error.strip_prefix("usage:") {
            Some(complaint) => match (args.first(), complaint.trim()) {
                (Some(word), NO_SUCH) => help::MENU.unknown(word),
                (Some(command), complaint) => help::MENU.wrong(command, complaint),
                (None, complaint) => help::MENU.missing(complaint),
            },
            None => {
                eprintln!("oslo history: {error}");
                1
            }
        },
    }
}

fn execute(args: &[String]) -> Result<(), String> {
    // Asked and answered before any subcommand parses its own words, so that `--help` cannot be
    // eaten by an argument parser that saw an option it did not know.
    if let Some(page) = help::MENU.asked(args, Paint::detect()) {
        print!("{page}");
        return Ok(());
    }
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "path" => path_command(rest),
        "status" => status_command(rest),
        "list" => query_command(rest, false),
        "search" => query_command(rest, true),
        "show" => show_command(rest),
        "stats" => stats_command(rest),
        "verify" => verify_command(rest),
        "sync" => sync_command(rest),
        "delete" => delete_command(rest),
        "clear" => clear_command(rest),
        "prune" => prune_command(rest),
        "export" => export_command(rest),
        "import" => import_command(rest),
        "backup" => backup_command(rest),
        _ => Err(format!("usage: {NO_SUCH}")),
    }
}

fn current_path() -> Result<PathBuf, String> {
    oslo::track::default_path(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .ok_or_else(|| "neither XDG_DATA_HOME nor HOME names the history directory".to_string())
}

fn open_current(read_only: bool) -> Result<Track, String> {
    open_verified(&current_path()?, read_only)
}

fn open_verified(path: &std::path::Path, read_only: bool) -> Result<Track, String> {
    verify_file(path)?;
    Track::open_existing(path, read_only)
}

fn path_command(args: &[String]) -> Result<(), String> {
    expect_empty(args, "path")?;
    println!("{}", current_path()?.display());
    Ok(())
}

fn status_command(args: &[String]) -> Result<(), String> {
    let (path, json_output) = optional_path_and_json(args, "status")?;
    print_status(&status_file(&path)?, json_output);
    Ok(())
}

fn print_status(status: &HistoryStatus, json_output: bool) {
    if json_output {
        println!(
            "{}",
            json!({
                "path": status.path,
                "schema": status.schema,
                "file_size": status.file_size,
                "events": status.events,
                "visible": status.visible,
                "tombstones": status.tombstones,
                "projections": status.projections,
                "pending_projections": status.pending_projections,
                "page_size": status.page_size,
                "allocated_pages": status.allocated_pages,
                "free_pages": status.free_pages,
                "pending_pages": status.pending_pages,
                "active_readers": status.active_readers,
            })
        );
        return;
    }
    println!("path\t{}", status.path);
    println!("schema\t{}", status.schema);
    println!("size\t{}", status.file_size);
    println!("events\t{}", status.events);
    println!("visible\t{}", status.visible);
    println!("tombstones\t{}", status.tombstones);
    println!("projections\t{}", status.projections);
    println!("pending-projections\t{}", status.pending_projections);
    println!("pages\t{}", status.allocated_pages);
    println!("free-pages\t{}", status.free_pages);
    println!("active-readers\t{}", status.active_readers);
}

#[derive(Default)]
struct QueryOptions {
    filter: HistoryFilter,
    json: bool,
    null: bool,
}

fn query_command(args: &[String], require_query: bool) -> Result<(), String> {
    let options = parse_query(args, require_query)?;
    let track = open_current(true)?;
    let events = track.events(&options.filter);
    print_events(&events, options.json, options.null)
}

fn parse_query(args: &[String], require_query: bool) -> Result<QueryOptions, String> {
    let mut options = QueryOptions::default();
    let mut matching_set = false;
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--json" => options.json = true,
            "--null" => options.null = true,
            "--oldest" => options.filter.oldest_first = true,
            "--exact" | "--prefix" | "--contains" => {
                if matching_set {
                    return Err("usage: choose one of --exact, --prefix, or --contains".to_string());
                }
                matching_set = true;
                options.filter.matching = match args[at].as_str() {
                    "--exact" => HistoryMatch::Exact,
                    "--prefix" => HistoryMatch::Prefix,
                    _ => HistoryMatch::Contains,
                };
            }
            "-n" | "--limit" => {
                at += 1;
                options.filter.limit = Some(parse_value(args, at, "limit")?);
            }
            "--host" => {
                at += 1;
                options.filter.host = Some(value(args, at, "host")?.to_string());
            }
            "--cwd" => {
                at += 1;
                options.filter.cwd = Some(value(args, at, "cwd")?.to_string());
            }
            "--status" => {
                at += 1;
                options.filter.status = Some(parse_value(args, at, "status")?);
            }
            "--since" => {
                at += 1;
                options.filter.since = Some(since(value(args, at, "duration")?)?);
            }
            "--before" => {
                at += 1;
                options.filter.before = Some(parse_time(value(args, at, "timestamp")?)?);
            }
            argument if argument.starts_with('-') => {
                return Err(format!("usage: unknown history query option {argument:?}"));
            }
            query => {
                if options.filter.query.is_some() {
                    return Err("usage: history query accepts one search string".to_string());
                }
                options.filter.query = Some(query.to_string());
            }
        }
        at += 1;
    }
    if require_query && options.filter.query.is_none() {
        return Err("usage: needs something to search for".to_string());
    }
    if options.json && options.null {
        return Err("usage: --json and --null cannot be combined".to_string());
    }
    Ok(options)
}

fn print_events(events: &[HistoryEvent], json_output: bool, null: bool) -> Result<(), String> {
    if json_output {
        let rows: Vec<Value> = events.iter().map(event_json).collect();
        println!("{}", Value::Array(rows));
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for event in events {
        if null {
            out.write_all(event.line.as_bytes())
                .and_then(|()| out.write_all(&[0]))
                .map_err(|error| error.to_string())?;
        } else {
            let status = event
                .completion
                .as_ref()
                .and_then(|done| done.status)
                .map(|status| status.to_string())
                .unwrap_or_else(|| "-".to_string());
            let cwd = event
                .completion
                .as_ref()
                .map(|done| escaped(&done.cwd))
                .unwrap_or_default();
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}\t{}",
                event.id,
                event.recorded_at,
                event.mode,
                status,
                cwd,
                escaped(&event.line)
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn event_json(event: &HistoryEvent) -> Value {
    let completion = event.completion.as_ref().map(|done| {
        json!({
            "cwd": done.cwd,
            "root": done.root,
            "status": done.status,
            "duration_ms": done.duration_ms,
            "segments": done.segments.iter().map(|segment| json!({
                "segment": segment.segment,
                "join": segment.join,
                "text": segment.text,
                "status": segment.status,
                "duration_ms": segment.duration_ms,
            })).collect::<Vec<_>>(),
        })
    });
    json!({
        "format": 1,
        "id": event.id.to_string(),
        "revision": event.revision,
        "deleted": event.deleted,
        "tie_breaker": hex(&event.tie_breaker),
        "line": event.line,
        "mode": event.mode,
        "recorded_at": event.recorded_at,
        "host": event.host,
        "session": event.session,
        "seq": event.seq,
        "rewritten": event.rewritten,
        "completion": completion,
    })
}

fn show_command(args: &[String]) -> Result<(), String> {
    let mut json_output = false;
    let mut id_text = None;
    for argument in args {
        match argument.as_str() {
            "--json" => json_output = true,
            flag if flag.starts_with('-') => {
                return Err(format!("usage: unknown show option {flag:?}"));
            }
            id if id_text.is_none() => id_text = Some(id),
            _ => return Err("usage: shows one event, so it takes one ID".to_string()),
        }
    }
    let id_text = id_text.ok_or_else(|| "usage: needs the event ID to show".to_string())?;
    let id = EventId::from_str(id_text).map_err(|error| format!("usage: {error}"))?;
    let event = open_current(true)?
        .event(id)
        .ok_or_else(|| format!("event {id} was not found"))?;
    if json_output {
        println!("{}", event_json(&event));
    } else {
        println!("id\t{}", event.id);
        println!("revision\t{}", event.revision);
        println!("deleted\t{}", event.deleted);
        println!("time\t{}", event.recorded_at);
        println!("mode\t{}", event.mode);
        println!("host\t{}", event.host);
        println!("session\t{}", event.session);
        if let Some(done) = &event.completion {
            println!("cwd\t{}", escaped(&done.cwd));
            println!(
                "status\t{}",
                done.status.map_or("-".to_string(), |s| s.to_string())
            );
            println!("duration-ms\t{}", done.duration_ms);
        }
        println!("line\t{}", escaped(&event.line));
    }
    Ok(())
}

mod admin;
mod interchange;

use admin::{
    clear_command, delete_command, prune_command, stats_command, sync_command, verify_command,
};
use interchange::{backup_command, export_command, hex, import_command};
#[cfg(test)]
use interchange::{parse_event_json, write_output};

fn optional_path_and_json(args: &[String], command: &str) -> Result<(PathBuf, bool), String> {
    let mut path = None;
    let mut json_output = false;
    for argument in args {
        if argument == "--json" {
            json_output = true;
        } else if argument.starts_with('-') || path.is_some() {
            return Err(format!("usage: {command} takes one file at most"));
        } else {
            path = Some(PathBuf::from(argument));
        }
    }
    Ok((path.unwrap_or(current_path()?), json_output))
}

fn confirm(question: &str) -> Result<bool, String> {
    eprint!("{question} [y/N] ");
    std::io::stderr()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn value<'a>(args: &'a [String], at: usize, name: &str) -> Result<&'a str, String> {
    args.get(at)
        .map(String::as_str)
        .ok_or_else(|| format!("usage: {name} requires a value"))
}

fn parse_value<T: FromStr>(args: &[String], at: usize, name: &str) -> Result<T, String> {
    value(args, at, name)?
        .parse()
        .map_err(|_| format!("usage: invalid {name}"))
}

fn since(text: &str) -> Result<u64, String> {
    let duration = duration_seconds(text)?;
    Ok(now().saturating_sub(duration))
}

fn parse_time(text: &str) -> Result<u64, String> {
    text.parse()
        .or_else(|_| since(text))
        .map_err(|_| "usage: timestamp must be epoch seconds or a duration".to_string())
}

fn duration_seconds(text: &str) -> Result<u64, String> {
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let amount: u64 = text[..split]
        .parse()
        .map_err(|_| "usage: invalid duration".to_string())?;
    let scale = match &text[split..] {
        "s" | "" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        _ => return Err("usage: duration suffix must be s, m, h, d, or w".to_string()),
    };
    Ok(amount.saturating_mul(scale))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn escaped(text: &str) -> String {
    text.chars()
        .flat_map(|character| character.escape_default())
        .collect()
}

fn expect_empty(args: &[String], command: &str) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("usage: {command} takes no arguments"))
    }
}

fn print_help() {
    print!("{}", help::text(Paint::detect()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_use_small_explicit_units() {
        assert_eq!(duration_seconds("2h"), Ok(7200));
        assert_eq!(duration_seconds("3d"), Ok(259200));
        assert!(duration_seconds("1month").is_err());
    }

    #[test]
    fn query_flags_build_one_filter() {
        let args = [
            "cargo".to_string(),
            "--prefix".to_string(),
            "--host".to_string(),
            "work".to_string(),
            "-n".to_string(),
            "7".to_string(),
        ];
        let parsed = parse_query(&args, true).expect("query");
        assert_eq!(parsed.filter.query.as_deref(), Some("cargo"));
        assert_eq!(parsed.filter.matching, HistoryMatch::Prefix);
        assert_eq!(parsed.filter.host.as_deref(), Some("work"));
        assert_eq!(parsed.filter.limit, Some(7));
        assert!(
            parse_query(
                &[
                    "cargo".to_string(),
                    "--exact".to_string(),
                    "--prefix".to_string(),
                ],
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn portable_json_preserves_event_identity_and_completion() {
        let event = HistoryEvent {
            id: EventId([7; 32]),
            revision: 3,
            deleted: false,
            tie_breaker: [9; 16],
            line: "echo a\n\0b".to_string(),
            mode: "sh".to_string(),
            recorded_at: 17,
            host: "host".to_string(),
            session: "session".to_string(),
            seq: 4,
            rewritten: true,
            completion: Some(HistoryCompletion {
                cwd: "/w".to_string(),
                root: Some("/".to_string()),
                status: Some(0),
                duration_ms: 12,
                segments: vec![HistorySegment {
                    segment: 1,
                    join: "&&".to_string(),
                    text: "true".to_string(),
                    status: Some(0),
                    duration_ms: 1,
                }],
            }),
        };
        let mut encoded = event_json(&event);
        assert_eq!(parse_event_json(&encoded), Ok(event));
        encoded["revision"] = Value::from(0);
        assert!(parse_event_json(&encoded).is_err());
    }

    #[test]
    fn file_output_is_private_and_refuses_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.jsonl");
        write_output(path.to_str().expect("path"), |out| {
            out.write_all(b"one\n").map_err(|error| error.to_string())
        })
        .expect("output");
        assert_eq!(std::fs::read(&path).expect("read"), b"one\n");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(write_output(path.to_str().expect("path"), |_| Ok(())).is_err());
    }

    #[test]
    fn file_output_never_replaces_a_racing_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.jsonl");
        let result = write_output(path.to_str().expect("path"), |out| {
            out.write_all(b"export\n")
                .map_err(|error| error.to_string())?;
            std::fs::write(&path, b"racer\n").map_err(|error| error.to_string())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).expect("destination"), b"racer\n");
    }

    #[test]
    fn malformed_subcommand_flags_are_usage_errors() {
        assert!(
            show_command(&["--unknown".to_string()])
                .expect_err("show error")
                .starts_with("usage:")
        );
        assert!(
            delete_command(&["--unknown".to_string()])
                .expect_err("delete error")
                .starts_with("usage:")
        );
        assert!(
            import_command(&["--unknown".to_string()])
                .expect_err("import error")
                .starts_with("usage:")
        );
        assert!(
            show_command(&["not-an-event-id".to_string()])
                .expect_err("show id error")
                .starts_with("usage:")
        );
        assert!(
            delete_command(&["not-an-event-id".to_string()])
                .expect_err("delete id error")
                .starts_with("usage:")
        );
        assert!(
            backup_command(&["--unknown".to_string()])
                .expect_err("backup error")
                .starts_with("usage:")
        );
    }
}
