//! Completion spec for `cargo`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "cargo",
        description: "Rust package manager and build tool",
        subcommands: vec![
            SubcommandSpec {
                name: "build",
                description: "Compile the current package",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--release"],
                        description: "Build artifacts in release mode, with optimizations",
                    },
                    OptionSpec {
                        names: vec!["--target"],
                        description: "Build for the target architecture",
                    },
                    OptionSpec {
                        names: vec!["--bin"],
                        description: "Build only the specified binary",
                    },
                    OptionSpec {
                        names: vec!["--example"],
                        description: "Build only the specified example",
                    },
                    OptionSpec {
                        names: vec!["--tests"],
                        description: "Build all tests",
                    },
                ],
            },
            SubcommandSpec {
                name: "run",
                description: "Run a binary or example of the local package",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--release"],
                        description: "Build artifacts in release mode",
                    },
                    OptionSpec {
                        names: vec!["--bin"],
                        description: "Run the specified binary",
                    },
                    OptionSpec {
                        names: vec!["--example"],
                        description: "Run the specified example",
                    },
                ],
            },
            SubcommandSpec {
                name: "test",
                description: "Execute all unit and integration tests",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--release"],
                        description: "Build artifacts in release mode",
                    },
                    OptionSpec {
                        names: vec!["--test"],
                        description: "Test only the specified test target",
                    },
                    OptionSpec {
                        names: vec!["--doc"],
                        description: "Test only this library's documentation",
                    },
                    OptionSpec {
                        names: vec!["--nocapture"],
                        description: "Do not capture stdout during test execution",
                    },
                ],
            },
            SubcommandSpec {
                name: "check",
                description: "Analyze the current package and report errors",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--release"],
                        description: "Check artifacts in release mode",
                    },
                    OptionSpec {
                        names: vec!["--all-targets"],
                        description: "Check all targets",
                    },
                ],
            },
            SubcommandSpec {
                name: "clean",
                description: "Remove artifacts that cargo has generated",
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: vec!["-p", "--package"],
                    description: "Package to clean artifacts for",
                }],
            },
            SubcommandSpec {
                name: "update",
                description: "Update dependencies listed in Cargo.lock",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-p", "--package"],
                        description: "Package to update",
                    },
                    OptionSpec {
                        names: vec!["--dry-run"],
                        description: "Don't actually write Cargo.lock",
                    },
                ],
            },
            SubcommandSpec {
                name: "fmt",
                description: "Format all binaries and libraries of the project",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--check"],
                        description: "Run in check mode, exiting with 1 if unformatted",
                    },
                    OptionSpec {
                        names: vec!["--all"],
                        description: "Format all packages in the workspace",
                    },
                ],
            },
            SubcommandSpec {
                name: "clippy",
                description: "Lint the codebase using rust-clippy",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--all-targets"],
                        description: "Lint all targets",
                    },
                    OptionSpec {
                        names: vec!["--fix"],
                        description: "Automatically apply clippy suggestions",
                    },
                ],
            },
        ],
        options: vec![
            OptionSpec {
                names: vec!["--version"],
                description: "Show version info",
            },
            OptionSpec {
                names: vec!["--help"],
                description: "Show help manual",
            },
        ],
    }
}
