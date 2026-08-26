//! Completion spec for `docker`.

use super::super::CommandSpec;
use super::{command, opt, sub};

pub(crate) fn spec() -> CommandSpec {
    command(
        "docker",
        "Manage Docker containers and images",
        vec![
            sub(
                "run",
                "Run a command in a new container",
                vec![
                    opt(&["-d", "--detach"], "Run container in background"),
                    opt(&["-it"], "Keep STDIN open and allocate a pseudo-TTY"),
                    opt(&["-p", "--publish"], "Publish container port(s) to host"),
                    opt(&["-v", "--volume"], "Bind mount a volume"),
                    opt(&["--rm"], "Automatically remove container when it exits"),
                    opt(&["--name"], "Assign a name to the container"),
                ],
            ),
            sub(
                "ps",
                "List containers",
                vec![
                    opt(
                        &["-a", "--all"],
                        "Show all containers (default shows just running)",
                    ),
                    opt(&["-q", "--quiet"], "Only display container IDs"),
                ],
            ),
            sub(
                "build",
                "Build an image from a Dockerfile",
                vec![
                    opt(
                        &["-t", "--tag"],
                        "Name and optionally a tag in format 'name:tag'",
                    ),
                    opt(&["-f", "--file"], "Name of the Dockerfile"),
                    opt(&["--no-cache"], "Do not use cache when building image"),
                ],
            ),
            sub(
                "images",
                "List images",
                vec![
                    opt(&["-a", "--all"], "Show all images"),
                    opt(&["-q", "--quiet"], "Only show image IDs"),
                ],
            ),
            sub(
                "stop",
                "Stop one or more running containers",
                vec![opt(
                    &["-t", "--time"],
                    "Seconds to wait before killing container",
                )],
            ),
            sub(
                "rm",
                "Remove one or more containers",
                vec![opt(
                    &["-f", "--force"],
                    "Force removal of running container",
                )],
            ),
            sub(
                "rmi",
                "Remove one or more images",
                vec![opt(&["-f", "--force"], "Force removal of image")],
            ),
        ],
        vec![opt(&["--version"], "Show Docker version info")],
    )
}
