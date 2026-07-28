//! Reading and writing: `echo` and `read`.

use crate::env::scope::Environment;
use crate::error::Result;
use nix::errno::Errno;

pub fn builtin_echo(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut print_newline = true;
    let mut start_idx = 1;

    if args.len() > 1 && args[1] == "-n" {
        print_newline = false;
        start_idx = 2;
    }

    let mut output = args[start_idx..].join(" ");
    if print_newline {
        output.push('\n');
    }

    let _ = nix::unistd::write(
        unsafe { std::os::fd::BorrowedFd::borrow_raw(1) },
        output.as_bytes(),
    );

    Ok(0)
}

/// One logical line of input as `read` sees it: the delimiter is gone, and backslash escapes
/// have been resolved (unless `-r` was given).
struct InputLine {
    /// Line content with the escaping backslashes themselves removed.
    bytes: Vec<u8>,
    /// Parallel to `bytes`: a byte that arrived escaped can never act as a field delimiter.
    escaped: Vec<bool>,
    /// Input ran out before a newline arrived. bash reports failure in that case *even when
    /// data was read* — that is what makes `while read l; do ...; done < file` terminate.
    eof: bool,
}

impl InputLine {
    fn push(&mut self, byte: u8, escaped: bool) {
        self.bytes.push(byte);
        self.escaped.push(escaped);
    }

    /// Default `IFS` whitespace. Newline never survives into `bytes`, so space and tab are all
    /// that can delimit here. (Honouring a user-set `IFS` is R5.4.)
    fn is_delim(&self, i: usize) -> bool {
        !self.escaped[i] && matches!(self.bytes[i], b' ' | b'\t')
    }

    fn slice(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.bytes[start..end]).into_owned()
    }
}

/// Read one line straight from fd 0, one byte at a time.
///
/// Deliberately unbuffered: `read` must not consume input past its own delimiter, or the next
/// command sharing this descriptor (`read x; cat`) would find it already drained. This is the
/// same trade bash makes on non-seekable input.
fn read_logical_line(raw: bool) -> std::result::Result<InputLine, Errno> {
    let mut line = InputLine {
        bytes: Vec::new(),
        escaped: Vec::new(),
        eof: false,
    };
    let mut pending_escape = false;
    let mut buf = [0u8; 1];

    loop {
        let n = match nix::unistd::read(0, &mut buf) {
            Ok(n) => n,
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(e),
        };

        if n == 0 {
            // A backslash with nothing behind it escapes nothing and is simply dropped, as in
            // bash: `printf 'a\' | read x` leaves `x=a`.
            line.eof = true;
            return Ok(line);
        }

        let byte = buf[0];
        if pending_escape {
            pending_escape = false;
            // Backslash-newline is a line continuation: both characters vanish.
            if byte != b'\n' {
                line.push(byte, true);
            }
            continue;
        }

        match byte {
            b'\n' => return Ok(line),
            b'\\' if !raw => pending_escape = true,
            _ => line.push(byte, false),
        }
    }
}

/// Split `line` across `names`, giving the last name the unsplit remainder.
fn assign_fields(env: &mut Environment, names: &[String], line: &InputLine) {
    let len = line.bytes.len();
    let mut pos = 0;

    for (i, name) in names.iter().enumerate() {
        while pos < len && line.is_delim(pos) {
            pos += 1;
        }
        let start = pos;

        let value = if i == names.len() - 1 {
            // Remainder verbatim — original separators included — minus trailing IFS whitespace.
            let mut end = len;
            while end > start && line.is_delim(end - 1) {
                end -= 1;
            }
            pos = len;
            line.slice(start, end)
        } else {
            while pos < len && !line.is_delim(pos) {
                pos += 1;
            }
            line.slice(start, pos)
        };

        env.set_var(name, &value, false);
    }
}

/// `read [-r] [--] [name...]` — one line of input, split across `name...` or into `REPLY`.
///
/// Returns 1 when the line was not terminated by a newline, which is the only thing that ever
/// stops `while read`. The remaining options (`-p`, `-n`, `-t`, `-d`, `-a`, `-u`, `IFS`) are
/// still unimplemented and fall through to the name list, per R5.4.
pub fn builtin_read(env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut raw = false;
    let mut idx = 1;
    while idx < args.len() {
        match args[idx].as_str() {
            "-r" => raw = true,
            "--" => {
                idx += 1;
                break;
            }
            _ => break,
        }
        idx += 1;
    }
    let names = &args[idx..];

    let line = match read_logical_line(raw) {
        Ok(line) => line,
        Err(_) => return Ok(1),
    };

    if names.is_empty() {
        // No names: the whole line lands in REPLY, unsplit and untrimmed.
        let reply = line.slice(0, line.bytes.len());
        env.set_var("REPLY", &reply, false);
    } else {
        assign_fields(env, names, &line);
    }

    // Non-zero on a line that no delimiter ended, even though its data was assigned.
    Ok(i32::from(line.eof))
}
