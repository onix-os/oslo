use std::env;
use std::fs;

pub fn get_git_branch() -> Option<String> {
    let current_dir = env::current_dir().ok()?;
    let mut dir = current_dir.as_path();
    loop {
        let git_head = dir.join(".git/HEAD");
        if git_head.exists() {
            if let Ok(content) = fs::read_to_string(git_head) {
                let trimmed = content.trim();
                if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                    return Some(branch.to_string());
                } else if trimmed.len() >= 7 {
                    return Some(trimmed[..7].to_string());
                }
            }
            break;
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    None
}

pub fn render_default_left_prompt(last_status: i32) -> String {
    let raw_pwd = env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/".to_string());

    let home = env::var("HOME").unwrap_or_default();
    let pwd = if !home.is_empty() && raw_pwd.starts_with(&home) {
        format!("~{}", &raw_pwd[home.len()..])
    } else {
        raw_pwd
    };

    let git_info = if let Some(branch) = get_git_branch() {
        format!(" \x1b[32m({})\x1b[0m", branch)
    } else {
        String::new()
    };

    let arrow_color = if last_status == 0 {
        "\x1b[1;32m" // Green
    } else {
        "\x1b[1;31m" // Red
    };

    format!(
        "\x1b[1;34m{}\x1b[0m{} {}❯\x1b[0m ",
        pwd, git_info, arrow_color
    )
}

pub fn render_default_right_prompt() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let hours = (secs / 3600 + 2) % 24; // Local estimate
    let mins = (secs / 60) % 60;
    let s = secs % 60;

    format!("\x1b[90m[{:02}:{:02}:{:02}]\x1b[0m", hours, mins, s)
}
