use super::*;

pub(super) fn export_command(args: &[String]) -> Result<(), String> {
    let mut destination = "-".to_string();
    let mut format = "jsonl";
    let mut at = 0;
    let mut saw_destination = false;
    while at < args.len() {
        match args[at].as_str() {
            "--format" => {
                at += 1;
                format = value(args, at, "format")?;
            }
            flag if flag.starts_with('-') && flag != "-" => {
                return Err(format!("usage: unknown export option {flag:?}"));
            }
            path if !saw_destination => {
                destination = path.to_string();
                saw_destination = true;
            }
            _ => {
                return Err("usage: writes one file, so it takes one".to_string());
            }
        }
        at += 1;
    }
    if !matches!(format, "jsonl" | "text") {
        return Err("usage: export format must be jsonl or text".to_string());
    }
    let mut filter = HistoryFilter {
        oldest_first: true,
        ..HistoryFilter::default()
    };
    filter.include_deleted = format == "jsonl";
    let events = open_current(true)?.events(&filter);
    write_output(&destination, |out| {
        for event in &events {
            if format == "jsonl" {
                writeln!(out, "{}", event_json(event)).map_err(|error| error.to_string())?;
            } else {
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string(&event.line).map_err(|e| e.to_string())?
                )
                .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })
}

pub(super) fn import_command(args: &[String]) -> Result<(), String> {
    let mut dry_run = false;
    let mut file = None;
    for argument in args {
        match argument.as_str() {
            "--dry-run" => dry_run = true,
            flag if flag.starts_with('-') && flag != "-" => {
                return Err(format!("usage: unknown import option {flag:?}"));
            }
            path if file.is_none() => file = Some(path),
            _ => return Err("usage: reads one file, so it takes one".to_string()),
        }
    }
    let file = file.ok_or_else(|| "usage: needs the file to read".to_string())?;
    let input = std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))?;
    let first = input.lines().find(|line| !line.trim().is_empty());
    if first.is_some_and(|line| line.trim_start().starts_with('{')) {
        let mut events = Vec::new();
        for (number, line) in input.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)
                .map_err(|error| format!("{file}:{}: {error}", number + 1))?;
            events.push(
                parse_event_json(&value)
                    .map_err(|error| format!("{file}:{}: {error}", number + 1))?,
            );
        }
        let current = current_path()?;
        if dry_run && !current.exists() {
            let mut unique: BTreeMap<EventId, HistoryEvent> = BTreeMap::new();
            for event in events {
                unique
                    .entry(event.id)
                    .and_modify(|current| *current = current.preferred(&event).clone())
                    .or_insert(event);
            }
            let deleted = unique.values().filter(|event| event.deleted).count();
            println!(
                "added={} updated=0 deleted={} unchanged=0 applied=0",
                unique.len().saturating_sub(deleted),
                deleted
            );
            return Ok(());
        }
        let track = if current.exists() {
            open_verified(&current, dry_run)?
        } else {
            Track::open(&current)
                .ok_or_else(|| format!("{}: cannot create database", current.display()))?
        };
        let report = track.import_events(&events, dry_run)?;
        println!(
            "added={} updated={} deleted={} unchanged={} applied={}",
            report.added, report.updated, report.deleted, report.unchanged, report.applied
        );
        return Ok(());
    }
    let lines: Result<Vec<String>, String> = input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            if line.starts_with('"') {
                serde_json::from_str(line).map_err(|error| error.to_string())
            } else {
                Ok(line.to_string())
            }
        })
        .collect();
    let lines = lines?;
    if !dry_run {
        let path = current_path()?;
        let track = if path.exists() {
            open_verified(&path, false)?
        } else {
            Track::open(&path)
                .ok_or_else(|| format!("{}: cannot create database", path.display()))?
        };
        for line in &lines {
            track
                .append(line, oslo::track::log::MODE_SHELL)
                .ok_or_else(|| "cannot import a history line".to_string())?;
        }
    }
    println!("added={}", lines.len());
    Ok(())
}

pub(super) fn backup_command(args: &[String]) -> Result<(), String> {
    let [destination] = args else {
        return Err("usage: needs one file to write the copy to".to_string());
    };
    if destination.starts_with('-') {
        return Err(format!("usage: unknown backup option {destination:?}"));
    }
    open_current(true)?.backup_to(PathBuf::from(destination).as_path())?;
    println!("{}", destination);
    Ok(())
}

pub(super) fn parse_event_json(value: &Value) -> Result<HistoryEvent, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "history event must be an object".to_string())?;
    if object.get("format").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported history export format".to_string());
    }
    let id = EventId::from_str(string_field(value, "id")?).map_err(str::to_string)?;
    let tie = decode_hex::<16>(string_field(value, "tie_breaker")?)?;
    let completion = match object.get("completion") {
        None | Some(Value::Null) => None,
        Some(done) => {
            let segments = done
                .get("segments")
                .and_then(Value::as_array)
                .ok_or_else(|| "completion.segments must be an array".to_string())?
                .iter()
                .map(|segment| {
                    Ok(HistorySegment {
                        segment: u32::try_from(integer_field(segment, "segment")?)
                            .map_err(|_| "segment is too large".to_string())?,
                        join: string_field(segment, "join")?.to_string(),
                        text: string_field(segment, "text")?.to_string(),
                        status: optional_i32(segment.get("status"))?,
                        duration_ms: signed_field(segment, "duration_ms")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Some(HistoryCompletion {
                cwd: string_field(done, "cwd")?.to_string(),
                root: optional_string(done.get("root"))?,
                status: optional_i32(done.get("status"))?,
                duration_ms: signed_field(done, "duration_ms")?,
                segments,
            })
        }
    };
    let revision = integer_field(value, "revision")?;
    if revision == 0 {
        return Err("revision must be greater than zero".to_string());
    }
    Ok(HistoryEvent {
        id,
        revision,
        deleted: bool_field(value, "deleted")?,
        tie_breaker: tie,
        line: string_field(value, "line")?.to_string(),
        mode: string_field(value, "mode")?.to_string(),
        recorded_at: integer_field(value, "recorded_at")?,
        host: string_field(value, "host")?.to_string(),
        session: string_field(value, "session")?.to_string(),
        seq: u32::try_from(integer_field(value, "seq")?)
            .map_err(|_| "seq is too large".to_string())?,
        rewritten: bool_field(value, "rewritten")?,
        completion,
    })
}

fn string_field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn integer_field(value: &Value, name: &str) -> Result<u64, String> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{name} must be an unsigned integer"))
}

fn signed_field(value: &Value, name: &str) -> Result<i64, String> {
    value
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{name} must be an integer"))
}

fn bool_field(value: &Value, name: &str) -> Result<bool, String> {
    value
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{name} must be a boolean"))
}

fn optional_string(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| "optional string field has the wrong type".to_string()),
    }
}

fn optional_i32(value: Option<&Value>) -> Result<Option<i32>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|number| i32::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| "optional status has the wrong type".to_string()),
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn decode_hex<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2 {
        return Err(format!("hex field must contain {} characters", N * 2));
    }
    if !text
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("hex field must use lowercase hexadecimal characters".to_string());
    }
    let mut bytes = [0; N];
    for (at, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[at * 2..at * 2 + 2], 16)
            .map_err(|_| "hex field contains a non-hexadecimal character".to_string())?;
    }
    Ok(bytes)
}

pub(super) fn write_output(
    destination: &str,
    write: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<(), String> {
    if destination == "-" {
        let mut stdout = std::io::stdout().lock();
        return write(&mut stdout);
    }
    let path = PathBuf::from(destination);
    if path.exists() {
        return Err(format!("{}: output already exists", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "output path has no file name".to_string())?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| format!("{}: {error}", temporary.display()))?;
        write(&mut file)?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        std::fs::hard_link(&temporary, &path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let _ = std::fs::remove_file(&temporary);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
