//! Completion spec for `npm`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "npm",
        description: "Node Package Manager",
        subcommands: vec![
            SubcommandSpec {
                name: "install",
                description: "Install a package and its dependencies",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-D", "--save-dev"],
                        description: "Save to devDependencies",
                    },
                    OptionSpec {
                        names: vec!["-g", "--global"],
                        description: "Install globally",
                    },
                ],
            },
            SubcommandSpec {
                name: "run",
                description: "Run an arbitrary package script",
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "start",
                description: "Start a package script",
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "test",
                description: "Test a package script",
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "build",
                description: "Build a package script",
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "uninstall",
                description: "Remove a package",
                subcommands: vec![],
                options: vec![],
            },
        ],
        options: vec![],
    }
}
