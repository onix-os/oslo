//! Completion spec for `cargo`.

use super::super::CommandSpec;
use super::{command, opt, sub};

pub(crate) fn spec() -> CommandSpec {
    command(
        "cargo",
        "Rust package manager and build tool",
        vec![
            sub(
                "build",
                "Compile the current package",
                vec![
                    opt(
                        &["--release"],
                        "Build artifacts in release mode, with optimizations",
                    ),
                    opt(&["--target"], "Build for the target architecture"),
                    opt(&["--bin"], "Build only the specified binary"),
                    opt(&["--example"], "Build only the specified example"),
                    opt(&["--tests"], "Build all tests"),
                ],
            ),
            sub(
                "run",
                "Run a binary or example of the local package",
                vec![
                    opt(&["--release"], "Build artifacts in release mode"),
                    opt(&["--bin"], "Run the specified binary"),
                    opt(&["--example"], "Run the specified example"),
                ],
            ),
            sub(
                "test",
                "Execute all unit and integration tests",
                vec![
                    opt(&["--release"], "Build artifacts in release mode"),
                    opt(&["--test"], "Test only the specified test target"),
                    opt(&["--doc"], "Test only this library's documentation"),
                    opt(
                        &["--nocapture"],
                        "Do not capture stdout during test execution",
                    ),
                ],
            ),
            sub(
                "check",
                "Analyze the current package and report errors",
                vec![
                    opt(&["--release"], "Check artifacts in release mode"),
                    opt(&["--all-targets"], "Check all targets"),
                ],
            ),
            sub(
                "clean",
                "Remove artifacts that cargo has generated",
                vec![opt(&["-p", "--package"], "Package to clean artifacts for")],
            ),
            sub(
                "update",
                "Update dependencies listed in Cargo.lock",
                vec![
                    opt(&["-p", "--package"], "Package to update"),
                    opt(&["--dry-run"], "Don't actually write Cargo.lock"),
                ],
            ),
            sub(
                "fmt",
                "Format all binaries and libraries of the project",
                vec![
                    opt(
                        &["--check"],
                        "Run in check mode, exiting with 1 if unformatted",
                    ),
                    opt(&["--all"], "Format all packages in the workspace"),
                ],
            ),
            sub(
                "clippy",
                "Lint the codebase using rust-clippy",
                vec![
                    opt(&["--all-targets"], "Lint all targets"),
                    opt(&["--fix"], "Automatically apply clippy suggestions"),
                ],
            ),
        ],
        vec![
            opt(&["--version"], "Show version info"),
            opt(&["--help"], "Show help manual"),
        ],
    )
}
