//! Completion spec for `npm`.

use super::super::CommandSpec;
use super::{command, opt, sub};

pub(crate) fn spec() -> CommandSpec {
    command(
        "npm",
        "Node Package Manager",
        vec![
            sub(
                "install",
                "Install a package and its dependencies",
                vec![
                    opt(&["-D", "--save-dev"], "Save to devDependencies"),
                    opt(&["-g", "--global"], "Install globally"),
                ],
            ),
            sub("run", "Run an arbitrary package script", vec![]),
            sub("start", "Start a package script", vec![]),
            sub("test", "Test a package script", vec![]),
            sub("build", "Build a package script", vec![]),
            sub("uninstall", "Remove a package", vec![]),
        ],
        vec![],
    )
}
