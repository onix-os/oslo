//! Completion spec for `docker`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "docker",
        description: "Manage Docker containers and images",
        subcommands: vec![
            SubcommandSpec {
                name: "run",
                description: "Run a command in a new container",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-d", "--detach"],
                        description: "Run container in background",
                    },
                    OptionSpec {
                        names: vec!["-it"],
                        description: "Keep STDIN open and allocate a pseudo-TTY",
                    },
                    OptionSpec {
                        names: vec!["-p", "--publish"],
                        description: "Publish container port(s) to host",
                    },
                    OptionSpec {
                        names: vec!["-v", "--volume"],
                        description: "Bind mount a volume",
                    },
                    OptionSpec {
                        names: vec!["--rm"],
                        description: "Automatically remove container when it exits",
                    },
                    OptionSpec {
                        names: vec!["--name"],
                        description: "Assign a name to the container",
                    },
                ],
            },
            SubcommandSpec {
                name: "ps",
                description: "List containers",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-a", "--all"],
                        description: "Show all containers (default shows just running)",
                    },
                    OptionSpec {
                        names: vec!["-q", "--quiet"],
                        description: "Only display container IDs",
                    },
                ],
            },
            SubcommandSpec {
                name: "build",
                description: "Build an image from a Dockerfile",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-t", "--tag"],
                        description: "Name and optionally a tag in format 'name:tag'",
                    },
                    OptionSpec {
                        names: vec!["-f", "--file"],
                        description: "Name of the Dockerfile",
                    },
                    OptionSpec {
                        names: vec!["--no-cache"],
                        description: "Do not use cache when building image",
                    },
                ],
            },
            SubcommandSpec {
                name: "images",
                description: "List images",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-a", "--all"],
                        description: "Show all images",
                    },
                    OptionSpec {
                        names: vec!["-q", "--quiet"],
                        description: "Only show image IDs",
                    },
                ],
            },
            SubcommandSpec {
                name: "stop",
                description: "Stop one or more running containers",
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: vec!["-t", "--time"],
                    description: "Seconds to wait before killing container",
                }],
            },
            SubcommandSpec {
                name: "rm",
                description: "Remove one or more containers",
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: vec!["-f", "--force"],
                    description: "Force removal of running container",
                }],
            },
            SubcommandSpec {
                name: "rmi",
                description: "Remove one or more images",
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: vec!["-f", "--force"],
                    description: "Force removal of image",
                }],
            },
        ],
        options: vec![OptionSpec {
            names: vec!["--version"],
            description: "Show Docker version info",
        }],
    }
}
