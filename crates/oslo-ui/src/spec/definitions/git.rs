//! Completion spec for `git`.

use super::super::{CommandSpec, OptionSpec, SubcommandSpec};

pub(crate) fn spec() -> CommandSpec {
    CommandSpec {
        name: "git".into(),
        description: "Distributed version control system".into(),
        subcommands: vec![
            SubcommandSpec {
                name: "commit".into(),
                description: "Record changes to the repository".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-m", "--message"]),
                        description: "Use the given message as the commit message".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-a", "--all"]),
                        description: "Commit all modified and deleted files".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-v", "--verbose"]),
                        description: "Show unified diff between index and HEAD".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--amend"]),
                        description: "Amend previous commit".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--no-verify"]),
                        description: "Bypass pre-commit and commit-msg hooks".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "checkout".into(),
                description: "Switch branches or restore working tree files".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-b"]),
                        description: "Create and checkout a new branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-B"]),
                        description: "Create/reset and checkout a branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-f", "--force"]),
                        description: "Force checkout (throw away local modifications)".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "status".into(),
                description: "Show the working tree status".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-s", "--short"]),
                        description: "Give output in short-format".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-b", "--branch"]),
                        description: "Show branch and tracking info in short-format".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "push".into(),
                description: "Update remote refs along with associated objects".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-u", "--set-upstream"]),
                        description: "Set upstream tracking branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-f", "--force"]),
                        description: "Force update remote refs".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--all"]),
                        description: "Push all branches".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--tags"]),
                        description: "Push all tags".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "pull".into(),
                description: "Fetch from and integrate with another repository".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--rebase"]),
                        description: "Rebase current branch on top of upstream".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--no-rebase"]),
                        description: "Do not rebase on top of upstream".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--ff-only"]),
                        description: "Refuse to merge unless fast-forward".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "add".into(),
                description: "Add file contents to the index".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-A", "--all"]),
                        description: "Add all tracked and untracked files".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-u", "--update"]),
                        description: "Update tracked files only".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-p", "--patch"]),
                        description: "Interactively select hunks to stage".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "branch".into(),
                description: "List, create, or delete branches".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-a", "--all"]),
                        description: "List both remote-tracking and local branches".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-d", "--delete"]),
                        description: "Delete a branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-D"]),
                        description: "Force delete a branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-m", "--move"]),
                        description: "Move/rename a branch".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "log".into(),
                description: "Show commit logs".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-n"]),
                        description: "Limit number of commits to output".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--oneline"]),
                        description: "Format each commit as a single line".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--graph"]),
                        description: "Draw a text-based graphical representation".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--stat"]),
                        description: "Generate a diffstat for each commit".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "diff".into(),
                description: "Show changes between commits, commit and working tree".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--cached", "--staged"]),
                        description: "Show diff between index and HEAD".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--stat"]),
                        description: "Generate a diffstat instead of patch".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "merge".into(),
                description: "Join two or more development histories together".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--no-ff"]),
                        description: "Create a merge commit even if fast-forward".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--squash"]),
                        description: "Squash commits into single merge".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--abort"]),
                        description: "Abort current in-progress merge".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "fetch".into(),
                description: "Download objects and refs from another repository".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-a", "--all"]),
                        description: "Fetch all remotes".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-p", "--prune"]),
                        description: "Remove remote-tracking references that no longer exist"
                            .into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "clone".into(),
                description: "Clone a repository into a new directory".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--depth"]),
                        description: "Create a shallow clone of depth N".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--branch"]),
                        description: "Point HEAD to specified branch".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--recursive"]),
                        description: "Initialize and clone submodules".into(),
                    },
                ],
            },
            SubcommandSpec {
                name: "init".into(),
                description: "Create an empty Git repository or reinitialize an existing one"
                    .into(),
                subcommands: vec![],
                options: vec![OptionSpec {
                    names: crate::spec::definitions::names(&["-b", "--initial-branch"]),
                    description: "Use specified name for initial branch".into(),
                }],
            },
            SubcommandSpec {
                name: "stash".into(),
                description: "Stash the changes in a dirty working directory away".into(),
                subcommands: vec![
                    SubcommandSpec {
                        name: "push".into(),
                        description: "Save local changes to stash".into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "pop".into(),
                        description: "Remove single stashed state from stash list and apply it"
                            .into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "list".into(),
                        description: "List stashed states".into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "apply".into(),
                        description: "Apply stashed state without removing it from list".into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "drop".into(),
                        description: "Remove single stashed state from stash list".into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                    SubcommandSpec {
                        name: "clear".into(),
                        description: "Remove all stashed states".into(),
                        subcommands: vec![],
                        options: vec![],
                    },
                ],
                options: vec![],
            },
            SubcommandSpec {
                name: "rebase".into(),
                description: "Reapply commits on top of another base tip".into(),
                subcommands: vec![],
                options: vec![
                    OptionSpec {
                        names: crate::spec::definitions::names(&["-i", "--interactive"]),
                        description: "Make a list of commits to be rebased".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--continue"]),
                        description: "Restart rebase process after resolving conflicts".into(),
                    },
                    OptionSpec {
                        names: crate::spec::definitions::names(&["--abort"]),
                        description: "Abort rebase and reset HEAD to original branch".into(),
                    },
                ],
            },
        ],
        options: vec![
            OptionSpec {
                names: crate::spec::definitions::names(&["--version"]),
                description: "Output git version info".into(),
            },
            OptionSpec {
                names: crate::spec::definitions::names(&["--help"]),
                description: "Output git help manual".into(),
            },
            OptionSpec {
                names: crate::spec::definitions::names(&["-C"]),
                description: "Run git as if started in <path>".into(),
            },
        ],
    }
}
