//! Completion spec for `cargo`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "cargo".into(),
        description: "Rust package manager and build tool".into(),
        subcommands: vec![
            SubcommandSpec {
                name: "build".into(),
                description: "Compile the current package".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--release"]),
                        description: "Build artifacts in release mode, with optimizations".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--target"]),
                        description: "Build for the target architecture".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--bin"]),
                        description: "Build only the specified binary".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--example"]),
                        description: "Build only the specified example".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--tests"]),
                        description: "Build all tests".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "run".into(),
                description: "Run a binary or example of the local package".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--release"]),
                        description: "Build artifacts in release mode".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--bin"]),
                        description: "Run the specified binary".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--example"]),
                        description: "Run the specified example".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "test".into(),
                description: "Execute all unit and integration tests".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--release"]),
                        description: "Build artifacts in release mode".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--test"]),
                        description: "Test only the specified test target".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--doc"]),
                        description: "Test only this library's documentation".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--nocapture"]),
                        description: "Do not capture stdout during test execution".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "check".into(),
                description: "Analyze the current package and report errors".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--release"]),
                        description: "Check artifacts in release mode".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--all-targets"]),
                        description: "Check all targets".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "clean".into(),
                description: "Remove artifacts that cargo has generated".into(),
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: crate::spec::definitions::names(&["-p", "--package"]),
                    description: "Package to clean artifacts for".into(),
                }],
            },
            SubcommandSpec {
                name: "update".into(),
                description: "Update dependencies listed in Cargo.lock".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-p", "--package"]),
                        description: "Package to update".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--dry-run"]),
                        description: "Don't actually write Cargo.lock".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "fmt".into(),
                description: "Format all binaries and libraries of the project".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--check"]),
                        description: "Run in check mode, exiting with 1 if unformatted".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--all"]),
                        description: "Format all packages in the workspace".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "clippy".into(),
                description: "Lint the codebase using rust-clippy".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--all-targets"]),
                        description: "Lint all targets".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--fix"]),
                        description: "Automatically apply clippy suggestions".into(),
                    },
                ],
            },
        ],
        options: vec![
            OptionSpec {
                names: crate::spec::definitions::names(&["--version"]),
                description: "Show version info".into(),
            },
            OptionSpec {
                names: crate::spec::definitions::names(&["--help"]),
                description: "Show help manual".into(),
            },
        ],
    }
}
