//! Completion spec for `docker`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "docker".into(),
        description: "Manage Docker containers and images".into(),
        subcommands: vec![
            SubcommandSpec {
                name: "run".into(),
                description: "Run a command in a new container".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-d", "--detach"]),
                        description: "Run container in background".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-it"]),
                        description: "Keep STDIN open and allocate a pseudo-TTY".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-p", "--publish"]),
                        description: "Publish container port(s) to host".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-v", "--volume"]),
                        description: "Bind mount a volume".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--rm"]),
                        description: "Automatically remove container when it exits".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--name"]),
                        description: "Assign a name to the container".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "ps".into(),
                description: "List containers".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-a", "--all"]),
                        description: "Show all containers (default shows just running)".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-q", "--quiet"]),
                        description: "Only display container IDs".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "build".into(),
                description: "Build an image from a Dockerfile".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-t", "--tag"]),
                        description: "Name and optionally a tag in format 'name:tag'".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-f", "--file"]),
                        description: "Name of the Dockerfile".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--no-cache"]),
                        description: "Do not use cache when building image".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "images".into(),
                description: "List images".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-a", "--all"]),
                        description: "Show all images".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-q", "--quiet"]),
                        description: "Only show image IDs".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "stop".into(),
                description: "Stop one or more running containers".into(),
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: crate::spec::definitions::names(&["-t", "--time"]),
                    description: "Seconds to wait before killing container".into(),
                }],
            },
            SubcommandSpec {
                name: "rm".into(),
                description: "Remove one or more containers".into(),
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: crate::spec::definitions::names(&["-f", "--force"]),
                    description: "Force removal of running container".into(),
                }],
            },
            SubcommandSpec {
                name: "rmi".into(),
                description: "Remove one or more images".into(),
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: crate::spec::definitions::names(&["-f", "--force"]),
                    description: "Force removal of image".into(),
                }],
            },
        ],
        options: vec![OptionSpec {
            names: crate::spec::definitions::names(&["--version"]),
            description: "Show Docker version info".into(),
        }],
    }
}
