//! Completion spec for `git`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "git",
        description: "Distributed version control system",
        subcommands: vec![
            SubcommandSpec {
                name: "commit",
                description: "Record changes to the repository",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-m", "--message"],
                        description: "Use the given message as the commit message",
                    },
                    OptionSpec {
                        names: vec!["-a", "--all"],
                        description: "Commit all modified and deleted files",
                    },
                    OptionSpec {
                        names: vec!["-v", "--verbose"],
                        description: "Show unified diff between index and HEAD",
                    },
                    OptionSpec {
                        names: vec!["--amend"],
                        description: "Amend previous commit",
                    },
                    OptionSpec {
                        names: vec!["--no-verify"],
                        description: "Bypass pre-commit and commit-msg hooks",
                    },
                ],
            },
            SubcommandSpec {
                name: "checkout",
                description: "Switch branches or restore working tree files",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-b"],
                        description: "Create and checkout a new branch",
                    },
                    OptionSpec {
                        names: vec!["-B"],
                        description: "Create/reset and checkout a branch",
                    },
                    OptionSpec {
                        names: vec!["-f", "--force"],
                        description: "Force checkout (throw away local modifications)",
                    },
                ],
            },
            SubcommandSpec {
                name: "status",
                description: "Show the working tree status",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-s", "--short"],
                        description: "Give output in short-format",
                    },
                    OptionSpec {
                        names: vec!["-b", "--branch"],
                        description: "Show branch and tracking info in short-format",
                    },
                ],
            },
            SubcommandSpec {
                name: "push",
                description: "Update remote refs along with associated objects",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-u", "--set-upstream"],
                        description: "Set upstream tracking branch",
                    },
                    OptionSpec {
                        names: vec!["-f", "--force"],
                        description: "Force update remote refs",
                    },
                    OptionSpec {
                        names: vec!["--all"],
                        description: "Push all branches",
                    },
                    OptionSpec {
                        names: vec!["--tags"],
                        description: "Push all tags",
                    },
                ],
            },
            SubcommandSpec {
                name: "pull",
                description: "Fetch from and integrate with another repository",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--rebase"],
                        description: "Rebase current branch on top of upstream",
                    },
                    OptionSpec {
                        names: vec!["--no-rebase"],
                        description: "Do not rebase on top of upstream",
                    },
                    OptionSpec {
                        names: vec!["--ff-only"],
                        description: "Refuse to merge unless fast-forward",
                    },
                ],
            },
            SubcommandSpec {
                name: "add",
                description: "Add file contents to the index",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-A", "--all"],
                        description: "Add all tracked and untracked files",
                    },
                    OptionSpec {
                        names: vec!["-u", "--update"],
                        description: "Update tracked files only",
                    },
                    OptionSpec {
                        names: vec!["-p", "--patch"],
                        description: "Interactively select hunks to stage",
                    },
                ],
            },
            SubcommandSpec {
                name: "branch",
                description: "List, create, or delete branches",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-a", "--all"],
                        description: "List both remote-tracking and local branches",
                    },
                    OptionSpec {
                        names: vec!["-d", "--delete"],
                        description: "Delete a branch",
                    },
                    OptionSpec {
                        names: vec!["-D"],
                        description: "Force delete a branch",
                    },
                    OptionSpec {
                        names: vec!["-m", "--move"],
                        description: "Move/rename a branch",
                    },
                ],
            },
            SubcommandSpec {
                name: "log",
                description: "Show commit logs",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-n"],
                        description: "Limit number of commits to output",
                    },
                    OptionSpec {
                        names: vec!["--oneline"],
                        description: "Format each commit as a single line",
                    },
                    OptionSpec {
                        names: vec!["--graph"],
                        description: "Draw a text-based graphical representation",
                    },
                    OptionSpec {
                        names: vec!["--stat"],
                        description: "Generate a diffstat for each commit",
                    },
                ],
            },
            SubcommandSpec {
                name: "diff",
                description: "Show changes between commits, commit and working tree",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--cached", "--staged"],
                        description: "Show diff between index and HEAD",
                    },
                    OptionSpec {
                        names: vec!["--stat"],
                        description: "Generate a diffstat instead of patch",
                    },
                ],
            },
            SubcommandSpec {
                name: "merge",
                description: "Join two or more development histories together",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--no-ff"],
                        description: "Create a merge commit even if fast-forward",
                    },
                    OptionSpec {
                        names: vec!["--squash"],
                        description: "Squash commits into single merge",
                    },
                    OptionSpec {
                        names: vec!["--abort"],
                        description: "Abort current in-progress merge",
                    },
                ],
            },
            SubcommandSpec {
                name: "fetch",
                description: "Download objects and refs from another repository",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-a", "--all"],
                        description: "Fetch all remotes",
                    },
                    OptionSpec {
                        names: vec!["-p", "--prune"],
                        description: "Remove remote-tracking references that no longer exist",
                    },
                ],
            },
            SubcommandSpec {
                name: "clone",
                description: "Clone a repository into a new directory",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["--depth"],
                        description: "Create a shallow clone of depth N",
                    },
                    OptionSpec {
                        names: vec!["--branch"],
                        description: "Point HEAD to specified branch",
                    },
                    OptionSpec {
                        names: vec!["--recursive"],
                        description: "Initialize and clone submodules",
                    },
                ],
            },
            SubcommandSpec {
                name: "init",
                description: "Create an empty Git repository or reinitialize an existing one",
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: vec!["-b", "--initial-branch"],
                    description: "Use specified name for initial branch",
                }],
            },
            SubcommandSpec {
                name: "stash",
                description: "Stash the changes in a dirty working directory away",
                subcommands: vec![
                    SubcommandSpec {
                        name: "push",
                        description: "Save local changes to stash",
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "pop",
                        description: "Remove single stashed state from stash list and apply it",
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "list",
                        description: "List stashed states",
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "apply",
                        description: "Apply stashed state without removing it from list",
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "drop",
                        description: "Remove single stashed state from stash list",
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "clear",
                        description: "Remove all stashed states",
                        subcommands: vec![],
                        options: vec![],
                    },
                ],
                options: vec![],
            },
            SubcommandSpec {
                name: "rebase",
                description: "Reapply commits on top of another base tip",
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: vec!["-i", "--interactive"],
                        description: "Make a list of commits to be rebased",
                    },
                    OptionSpec {
                        names: vec!["--continue"],
                        description: "Restart rebase process after resolving conflicts",
                    },
                    OptionSpec {
                        names: vec!["--abort"],
                        description: "Abort rebase and reset HEAD to original branch",
                    },
                ],
            },
        ],
        options: vec![
            OptionSpec {
                names: vec!["--version"],
                description: "Output git version info",
            },
            OptionSpec {
                names: vec!["--help"],
                description: "Output git help manual",
            },
            OptionSpec {
                names: vec!["-C"],
                description: "Run git as if started in <path>",
            },
        ],
    }
}
