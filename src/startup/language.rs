//! Deciding whether a program is Lua or shell.
//!
//! oslo is a Lua shell, so running a Lua file must not need a flag to say so. `oslo build.lua`,
//! `oslo deploy.sh` and `oslo script` all just work; the old `--lua-script FILE` is gone, because
//! a shell whose scripting language needs an opt-in flag is not really that shell.
//!
//! Four questions, asked in order of how much the answer can be trusted:
//!
//! 1. **An explicit `--lua` or `--sh`.** The author said so; nothing else gets a vote.
//! 2. **The shebang.** `#!/usr/bin/lua` is what the kernel itself would honour, and it is a
//!    deliberate statement by whoever wrote the file.
//! 3. **The extension.** `.lua` against `.sh`/`.bash`/`.zsh` — weaker than a shebang (a file can
//!    be misnamed) but still an explicit act.
//! 4. **The text.** Only when the first three say nothing, and only when the evidence is
//!    one-sided: markers that are syntactically impossible in the other language.
//!
//! When even the text is ambiguous — no markers, or markers for both — the answer is shell.
//! Not because shell matters more, but because that is the case where the file was *given* to a
//! shell with no indication of anything else, and because getting it wrong the other way silently
//! reinterprets POSIX scripts that have worked for decades. A file that wants to be sure should
//! carry a shebang, which is good practice regardless.

/// Which interpreter a program is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Shell,
    Lua,
}

/// Markers that cannot appear in a POSIX shell script without being a syntax error, so seeing one
/// is evidence of Lua rather than a guess about style.
///
/// `local` is deliberately absent even though Lua uses it constantly: it is also an oslo builtin,
/// so `local x=1` is valid in both languages and says nothing.
const LUA_MARKERS: &[&str] = &[
    "--[[",      // long comment; `--` alone is a shell comment, so the brackets are the signal
    "function ", // shell functions are `name()`, never the `function` keyword followed by a space
    "local function ",
    "end)",
    "then\n",
    "elseif ",
    "nil",
    "require(",
    "require ",
    "ipairs(",
    "pairs(",
    "print(", // shell has `echo`/`printf`; `print(` as a call is Lua
    "oslo.",  // the Lua API this shell exposes
    "..",     // Lua string concatenation
];

/// Markers that are shell and cannot be Lua.
///
/// Lua has no `$`, no backticks, and no `fi`/`esac`/`done` keywords, so each of these is a
/// syntax error in Lua rather than a stylistic hint.
const SHELL_MARKERS: &[&str] = &[
    "$(", "${", "$1", "$@", "$?", "$#", "fi\n", "esac", "done\n", "elif ", "echo ", "&&", "||",
    "|", ">>", "2>",
];

/// Decide from an explicit flag, the path and the program text.
///
/// `forced` short-circuits everything: it is the `--lua`/`--sh` the caller passed.
pub fn detect(forced: Option<Language>, path: Option<&str>, text: &str) -> Language {
    if let Some(language) = forced {
        return language;
    }
    if let Some(language) = from_shebang(text) {
        return language;
    }
    if let Some(language) = path.and_then(from_extension) {
        return language;
    }
    from_text(text)
}

/// The `#!` line, if it names an interpreter this shell recognises.
///
/// Matched on the basename so `/usr/bin/lua5.4`, `/usr/local/bin/lua` and `/usr/bin/env lua` all
/// land the same way, and so a *directory* called `lua` in the path of `/opt/lua/bin/bash` does
/// not make a bash script look like Lua.
fn from_shebang(text: &str) -> Option<Language> {
    let line = text.lines().next()?.strip_prefix("#!")?;
    // `env` forwards to the next word, which is the interpreter that matters.
    let words: Vec<&str> = line.split_whitespace().collect();
    let interpreter = words
        .iter()
        .map(|w| w.rsplit('/').next().unwrap_or(w))
        .find(|w| *w != "env" && !w.starts_with('-'))?;
    if interpreter.starts_with("lua") {
        return Some(Language::Lua);
    }
    if interpreter.ends_with("sh") {
        return Some(Language::Shell);
    }
    // `#!/usr/bin/env oslo` names *this shell* and says nothing about the language, which is the
    // whole question — so it decides nothing and the extension or the text answers instead. It is
    // also the shebang someone will most naturally write on an oslo Lua script, and reading it as
    // "shell" sent every such file to the shell parser.
    None
}

fn from_extension(path: &str) -> Option<Language> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.')?.1;
    match ext {
        "lua" => Some(Language::Lua),
        "sh" | "bash" | "zsh" | "ksh" => Some(Language::Shell),
        _ => None,
    }
}

/// Weigh the two sets of markers, and answer only when the evidence is one-sided.
fn from_text(text: &str) -> Language {
    let body = strip_comments(text);
    let lua = LUA_MARKERS.iter().filter(|m| body.contains(**m)).count();
    let shell = SHELL_MARKERS.iter().filter(|m| body.contains(**m)).count();
    if lua > 0 && shell == 0 {
        Language::Lua
    } else {
        // Both, or neither: see the module comment for why this way round.
        Language::Shell
    }
}

/// Drop `#`-comment lines before sniffing.
///
/// A comment is prose, and prose mentioning `$PATH` or `print(x)` is not evidence about the
/// language. Lua's own `--` comments are left alone: `--[[` is one of the markers.
fn strip_comments(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_text(text: &str) -> Language {
        detect(None, None, text)
    }

    #[test]
    fn an_explicit_flag_beats_everything_else() {
        // A file that looks like shell in every other way still runs as Lua when asked.
        let shell = "#!/bin/sh\necho $HOME\n";
        assert_eq!(
            detect(Some(Language::Lua), Some("x.sh"), shell),
            Language::Lua
        );
        let lua = "#!/usr/bin/lua\nprint(1)\n";
        assert_eq!(
            detect(Some(Language::Shell), Some("x.lua"), lua),
            Language::Shell
        );
    }

    #[test]
    fn a_shebang_decides_and_outranks_the_extension() {
        assert_eq!(
            detect(None, Some("x.sh"), "#!/usr/bin/lua\n"),
            Language::Lua
        );
        assert_eq!(
            detect(None, Some("x.lua"), "#!/bin/bash\n"),
            Language::Shell
        );
        // `env` forwards to the interpreter after it.
        assert_eq!(detect(None, None, "#!/usr/bin/env lua\n"), Language::Lua);
        assert_eq!(detect(None, None, "#!/usr/bin/env bash\n"), Language::Shell);
        // Versioned interpreters.
        assert_eq!(detect(None, None, "#!/usr/bin/lua5.4\n"), Language::Lua);
    }

    /// `#!/usr/bin/env oslo` names the shell, not the language, so it must not decide — it is the
    /// shebang an oslo Lua script naturally carries, and reading it as "shell" sent every such
    /// file to the shell parser. The extension, then the text, answer instead.
    #[test]
    fn oslos_own_shebang_defers_to_the_next_test() {
        assert_eq!(
            detect(None, Some("deploy.lua"), "#!/usr/bin/env oslo\nprint(1)\n"),
            Language::Lua
        );
        assert_eq!(
            detect(None, Some("deploy.sh"), "#!/usr/bin/env oslo\necho hi\n"),
            Language::Shell
        );
        // No extension either: the text is all that is left.
        assert_eq!(
            detect(None, None, "#!/usr/bin/env oslo\nlocal t = {}\nprint(#t)\n"),
            Language::Lua
        );
    }

    /// A directory called `lua` on the way to a shell interpreter must not decide the answer.
    #[test]
    fn only_the_interpreters_basename_counts() {
        assert_eq!(detect(None, None, "#!/opt/lua/bin/bash\n"), Language::Shell);
    }

    #[test]
    fn the_extension_decides_when_there_is_no_shebang() {
        assert_eq!(detect(None, Some("build.lua"), "x = 1\n"), Language::Lua);
        assert_eq!(detect(None, Some("build.sh"), "x=1\n"), Language::Shell);
        assert_eq!(detect(None, Some("/a/b/deploy.lua"), ""), Language::Lua);
    }

    #[test]
    fn unmarked_text_is_read_for_syntax_that_can_only_be_one_language() {
        assert_eq!(detect_text("local t = {}\nprint(#t)\n"), Language::Lua);
        assert_eq!(
            detect_text("for i, v in ipairs(xs) do print(v) end\n"),
            Language::Lua
        );
        assert_eq!(
            detect_text("oslo.set_alias(\"gs\", \"git status\")\n"),
            Language::Lua
        );
        assert_eq!(detect_text("echo \"$HOME\"\n"), Language::Shell);
        assert_eq!(
            detect_text("if [ -f x ]; then echo hi; fi\n"),
            Language::Shell
        );
        assert_eq!(detect_text("cat a | sort > b\n"), Language::Shell);
    }

    /// The tie-break, stated as a test so changing it is a deliberate act.
    #[test]
    fn ambiguous_or_empty_text_is_shell() {
        assert_eq!(detect_text(""), Language::Shell);
        assert_eq!(detect_text("x = 1\n"), Language::Shell);
        // Markers for both: a Lua file that shells out is not evidence enough on its own.
        assert_eq!(detect_text("print(1)\necho hi\n"), Language::Shell);
    }

    /// Comments are prose. A shell script explaining Lua must not be read as Lua.
    #[test]
    fn hash_comments_do_not_vote() {
        assert_eq!(
            detect_text("# this script does not use require() or ipairs()\nls\n"),
            Language::Shell
        );
    }
}
