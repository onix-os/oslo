//! Completion spec for `npm`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "npm".into(),
        description: "Node Package Manager".into(),
        subcommands: vec![
            SubcommandSpec {
                name: "install".into(),
                description: "Install a package and its dependencies".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-D", "--save-dev"]),
                        description: "Save to devDependencies".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-g", "--global"]),
                        description: "Install globally".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "run".into(),
                description: "Run an arbitrary package script".into(),
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "start".into(),
                description: "Start a package script".into(),
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "test".into(),
                description: "Test a package script".into(),
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "build".into(),
                description: "Build a package script".into(),
                subcommands: vec![],
                options: vec![],
            },
            SubcommandSpec {
                name: "uninstall".into(),
                description: "Remove a package".into(),
                subcommands: vec![],
                options: vec![],
            },
        ],
        options: vec![],
    }
}
