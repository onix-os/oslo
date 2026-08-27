//! Completion spec for `git`.

use super::super::CommandSpec;
use super::{command, group, opt, sub};

pub(crate) fn spec() -> CommandSpec {
    command(
        "git",
        "Distributed version control system",
        vec![
            sub(
                "commit",
                "Record changes to the repository",
                vec![
                    opt(
                        &["-m", "--message"],
                        "Use the given message as the commit message",
                    ),
                    opt(&["-a", "--all"], "Commit all modified and deleted files"),
                    opt(
                        &["-v", "--verbose"],
                        "Show unified diff between index and HEAD",
                    ),
                    opt(&["--amend"], "Amend previous commit"),
                    opt(&["--no-verify"], "Bypass pre-commit and commit-msg hooks"),
                ],
            ),
            sub(
                "checkout",
                "Switch branches or restore working tree files",
                vec![
                    opt(&["-b"], "Create and checkout a new branch"),
                    opt(&["-B"], "Create/reset and checkout a branch"),
                    opt(
                        &["-f", "--force"],
                        "Force checkout (throw away local modifications)",
                    ),
                ],
            ),
            sub(
                "status",
                "Show the working tree status",
                vec![
                    opt(&["-s", "--short"], "Give output in short-format"),
                    opt(
                        &["-b", "--branch"],
                        "Show branch and tracking info in short-format",
                    ),
                ],
            ),
            sub(
                "push",
                "Update remote refs along with associated objects",
                vec![
                    opt(&["-u", "--set-upstream"], "Set upstream tracking branch"),
                    opt(&["-f", "--force"], "Force update remote refs"),
                    opt(&["--all"], "Push all branches"),
                    opt(&["--tags"], "Push all tags"),
                ],
            ),
            sub(
                "pull",
                "Fetch from and integrate with another repository",
                vec![
                    opt(&["--rebase"], "Rebase current branch on top of upstream"),
                    opt(&["--no-rebase"], "Do not rebase on top of upstream"),
                    opt(&["--ff-only"], "Refuse to merge unless fast-forward"),
                ],
            ),
            sub(
                "add",
                "Add file contents to the index",
                vec![
                    opt(&["-A", "--all"], "Add all tracked and untracked files"),
                    opt(&["-u", "--update"], "Update tracked files only"),
                    opt(&["-p", "--patch"], "Interactively select hunks to stage"),
                ],
            ),
            sub(
                "branch",
                "List, create, or delete branches",
                vec![
                    opt(
                        &["-a", "--all"],
                        "List both remote-tracking and local branches",
                    ),
                    opt(&["-d", "--delete"], "Delete a branch"),
                    opt(&["-D"], "Force delete a branch"),
                    opt(&["-m", "--move"], "Move/rename a branch"),
                ],
            ),
            sub(
                "log",
                "Show commit logs",
                vec![
                    opt(&["-n"], "Limit number of commits to output"),
                    opt(&["--oneline"], "Format each commit as a single line"),
                    opt(&["--graph"], "Draw a text-based graphical representation"),
                    opt(&["--stat"], "Generate a diffstat for each commit"),
                ],
            ),
            sub(
                "diff",
                "Show changes between commits, commit and working tree",
                vec![
                    opt(
                        &["--cached", "--staged"],
                        "Show diff between index and HEAD",
                    ),
                    opt(&["--stat"], "Generate a diffstat instead of patch"),
                ],
            ),
            sub(
                "merge",
                "Join two or more development histories together",
                vec![
                    opt(&["--no-ff"], "Create a merge commit even if fast-forward"),
                    opt(&["--squash"], "Squash commits into single merge"),
                    opt(&["--abort"], "Abort current in-progress merge"),
                ],
            ),
            sub(
                "fetch",
                "Download objects and refs from another repository",
                vec![
                    opt(&["-a", "--all"], "Fetch all remotes"),
                    opt(
                        &["-p", "--prune"],
                        "Remove remote-tracking references that no longer exist",
                    ),
                ],
            ),
            sub(
                "clone",
                "Clone a repository into a new directory",
                vec![
                    opt(&["--depth"], "Create a shallow clone of depth N"),
                    opt(&["--branch"], "Point HEAD to specified branch"),
                    opt(&["--recursive"], "Initialize and clone submodules"),
                ],
            ),
            sub(
                "init",
                "Create an empty Git repository or reinitialize an existing one",
                vec![opt(
                    &["-b", "--initial-branch"],
                    "Use specified name for initial branch",
                )],
            ),
            group(
                "stash",
                "Stash the changes in a dirty working directory away",
                vec![
                    sub("push", "Save local changes to stash", vec![]),
                    sub(
                        "pop",
                        "Remove single stashed state from stash list and apply it",
                        vec![],
                    ),
                    sub("list", "List stashed states", vec![]),
                    sub(
                        "apply",
                        "Apply stashed state without removing it from list",
                        vec![],
                    ),
                    sub(
                        "drop",
                        "Remove single stashed state from stash list",
                        vec![],
                    ),
                    sub("clear", "Remove all stashed states", vec![]),
                ],
            ),
            sub(
                "rebase",
                "Reapply commits on top of another base tip",
                vec![
                    opt(
                        &["-i", "--interactive"],
                        "Make a list of commits to be rebased",
                    ),
                    opt(
                        &["--continue"],
                        "Restart rebase process after resolving conflicts",
                    ),
                    opt(
                        &["--abort"],
                        "Abort rebase and reset HEAD to original branch",
                    ),
                ],
            ),
        ],
        vec![
            opt(&["--version"], "Output git version info"),
            opt(&["--help"], "Output git help manual"),
            opt(&["-C"], "Run git as if started in <path>"),
        ],
    )
}
