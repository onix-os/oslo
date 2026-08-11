use super::*;

pub(super) fn stats_command(args: &[String]) -> Result<(), String> {
    let options = parse_stats(args)?;
    let events = open_current(true)?.events(&options.filter);
    let visible: Vec<&HistoryEvent> = events.iter().filter(|event| !event.deleted).collect();
    let failures = visible
        .iter()
        .filter(|event| {
            event
                .completion
                .as_ref()
                .and_then(|done| done.status)
                .is_some_and(|status| status != 0)
        })
        .count();
    let completed = visible
        .iter()
        .filter(|event| event.completion.is_some())
        .count();
    let duration_ms: i64 = visible
        .iter()
        .filter_map(|event| event.completion.as_ref())
        .map(|done| done.duration_ms)
        .fold(0_i64, i64::saturating_add);
    let successes = visible
        .iter()
        .filter(|event| event.completion.as_ref().and_then(|done| done.status) == Some(0))
        .count();
    let first_at = visible.iter().map(|event| event.recorded_at).min();
    let last_at = visible.iter().map(|event| event.recorded_at).max();
    let hosts: BTreeSet<&str> = visible.iter().map(|event| event.host.as_str()).collect();
    let directories: BTreeSet<&str> = visible
        .iter()
        .filter_map(|event| event.completion.as_ref().map(|done| done.cwd.as_str()))
        .collect();
    let commands: BTreeSet<(&str, &str)> = visible
        .iter()
        .map(|event| (event.mode.as_str(), event.line.as_str()))
        .collect();
    let value = json!({
        "events": visible.len(),
        "commands": commands.len(),
        "completed": completed,
        "successes": successes,
        "failures": failures,
        "hosts": hosts.len(),
        "directories": directories.len(),
        "duration_ms": duration_ms,
        "first_at": first_at,
        "last_at": last_at,
    });
    if options.json {
        println!("{value}");
    } else {
        for key in [
            "events",
            "commands",
            "completed",
            "successes",
            "failures",
            "hosts",
            "directories",
            "duration_ms",
            "first_at",
            "last_at",
        ] {
            println!("{key}\t{}", value[key]);
        }
    }
    Ok(())
}

fn parse_stats(args: &[String]) -> Result<QueryOptions, String> {
    let mut options = QueryOptions::default();
    let mut at = 0;
    while at < args.len() {
        match args[at].as_str() {
            "--json" => options.json = true,
            "--host" => {
                at += 1;
                options.filter.host = Some(value(args, at, "host")?.to_string());
            }
            "--since" => {
                at += 1;
                options.filter.since = Some(since(value(args, at, "duration")?)?);
            }
            argument => return Err(format!("usage: unknown stats option {argument:?}")),
        }
        at += 1;
    }
    Ok(options)
}

pub(super) fn verify_command(args: &[String]) -> Result<(), String> {
    let (path, json_output) = optional_path_and_json(args, "verify")?;
    let status = verify_file(&path)?;
    if json_output {
        println!(
            "{}",
            json!({"ok": true, "path": status.path, "schema": status.schema})
        );
    } else {
        println!("ok\t{}", status.path);
    }
    Ok(())
}

pub(super) fn sync_command(args: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    let mut dry_run = false;
    let mut json_output = false;
    for argument in args {
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            "--json" => json_output = true,
            flag if flag.starts_with('-') => {
                return Err(format!("usage: unknown sync option {flag:?}"));
            }
            path => paths.push(PathBuf::from(path)),
        }
    }
    let (left, right) = match paths.as_slice() {
        [other] => (current_path()?, other.clone()),
        [left, right] => (left.clone(), right.clone()),
        _ => {
            return Err(
                "usage: oslo history sync OTHER|FILE1 FILE2 [--dry-run] [--json]".to_string(),
            );
        }
    };
    let report = sync_files(&left, &right, dry_run)?;
    let value = json!({
        "dry_run": dry_run,
        "left": {
            "added": report.added_left,
            "updated": report.updated_left,
            "deleted": report.deleted_left,
            "applied": report.applied_left,
        },
        "right": {
            "added": report.added_right,
            "updated": report.updated_right,
            "deleted": report.deleted_right,
            "applied": report.applied_right,
        },
        "unchanged": report.unchanged,
    });
    if json_output {
        println!("{value}");
    } else {
        println!(
            "left\tadded={} updated={} deleted={} applied={}",
            report.added_left, report.updated_left, report.deleted_left, report.applied_left
        );
        println!(
            "right\tadded={} updated={} deleted={} applied={}",
            report.added_right, report.updated_right, report.deleted_right, report.applied_right
        );
        println!("unchanged\t{}", report.unchanged);
    }
    Ok(())
}

pub(super) fn delete_command(args: &[String]) -> Result<(), String> {
    let mut yes = false;
    let mut ids = Vec::new();
    for argument in args {
        match argument.as_str() {
            "--yes" => yes = true,
            flag if flag.starts_with('-') => {
                return Err(format!("usage: unknown delete option {flag:?}"));
            }
            id => ids.push(EventId::from_str(id).map_err(|error| format!("usage: {error}"))?),
        }
    }
    if ids.is_empty() {
        return Err("usage: oslo history delete EVENT_ID... [--yes]".to_string());
    }
    if !yes && !confirm(&format!("delete {} history event(s)?", ids.len()))? {
        return Ok(());
    }
    println!("deleted\t{}", open_current(false)?.delete_events(&ids)?);
    Ok(())
}

pub(super) fn clear_command(args: &[String]) -> Result<(), String> {
    if args != ["--yes"] {
        return Err("usage: oslo history clear --yes".to_string());
    }
    println!("deleted\t{}", open_current(false)?.clear_events()?);
    // The predictor's snapshot is a distillation of exactly what was just deleted. A shell that
    // kept it would still be able to suggest a line the user asked it to forget, which is the
    // same leak as not clearing at all — only harder to notice.
    #[cfg(feature = "vista")]
    if let Some(path) = oslo_base::predict::default_path(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    ) && let Err(err) = oslo_base::predict::forget_saved(&path)
    {
        return Err(format!("clear model: {err}"));
    }
    Ok(())
}

pub(super) fn prune_command(args: &[String]) -> Result<(), String> {
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let yes = args.iter().any(|arg| arg == "--yes");
    if args.iter().any(|arg| arg != "--dry-run" && arg != "--yes") {
        return Err("usage: oslo history prune [--dry-run] [--yes]".to_string());
    }
    if dry_run {
        let preview = open_current(true)?.sweep_preview();
        println!(
            "dry-run\trun-rows={} missing-directories={} expired-directories={}",
            preview.run_rows, preview.missing_directories, preview.expired_directories
        );
        return Ok(());
    }
    if !yes {
        return Err("usage: mutating prune requires --yes".to_string());
    }
    println!("removed-runs\t{}", open_current(false)?.sweep());
    Ok(())
}
