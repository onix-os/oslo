//! Who is calling whom: the shell-function call chain, and what a script can read of it.
//!
//! Two stacks, and keeping them apart is the point. [`Environment::call_stack`] is the *functions*
//! currently executing, which is what `caller` reports and what it asks "am I inside a function at
//! all" — a question the `while caller $i` idiom depends on. `$FUNCNAME` is nearly the same list
//! and not quite: bash puts the way the code was reached on the end of it, so a function called
//! from a script file reads `f main` and one called from a sourced file reads `f source`. Those
//! are not function frames, so they live in their own list and are added only where the array is
//! published.

use super::{Environment, ShellArray, UNNAMED_FUNCTION};
use oslo_base::error::Result;

impl Environment {
    /// Begin a shell-function call; `Err` once the call chain is too deep to be safe.
    ///
    /// The caller must pair this with [`Self::exit_function`] on every path out of the call,
    /// including an unwinding `return` or error. A refused entry is not entered and must not be
    /// exited.
    ///
    /// Prefer [`Self::enter_function_named`]: `caller` can only report a name that was recorded
    /// on the way in, and this form records the placeholder bash prints when it has none.
    pub fn enter_function(&mut self) -> Result<()> {
        self.enter_function_named(UNNAMED_FUNCTION)
    }

    /// Begin a shell-function call, recording which function it is.
    ///
    /// The name is what `caller n` reports as the second field. Kept beside the depth counter
    /// rather than in a table of its own so the two cannot drift: one push, one pop, both here.
    pub fn enter_function_named(&mut self, name: &str) -> Result<()> {
        self.function_depth.enter()?;
        self.call_stack.push(name.to_string());
        self.publish_call_stack();
        Ok(())
    }

    pub fn exit_function(&mut self) {
        self.function_depth.exit();
        self.call_stack.pop();
        self.publish_call_stack();
    }

    /// Publish `$FUNCNAME`, which is the call stack a script can read.
    ///
    /// **It was empty, and said nothing about being empty.** `f() { echo "$FUNCNAME"; }` printed a
    /// blank line where bash prints `f` — so a log line or an error handler built on it lost the
    /// one piece of information it existed to carry, silently, in every script that used one.
    ///
    /// An array, as in bash: `${FUNCNAME[0]}` is the function running now and `${FUNCNAME[1]}` is
    /// whoever called it, so the order is the reverse of [`Self::call_stack`], which reads
    /// outermost first. A bare `$FUNCNAME` is element 0, which the array machinery already does.
    ///
    /// Rebuilt on entry and exit rather than synthesised when read, because `get_array` hands back
    /// a reference into the table and cannot make one up. The depth is capped at
    /// [`MAX_FUNCTION_DEPTH`], so the copy is bounded and small — the same way `PIPESTATUS` is
    /// published by whoever computes it.
    ///
    /// Unset outside every function, which is also bash: `${FUNCNAME+set}` is how a script asks
    /// whether it is inside one at all.
    fn publish_call_stack(&mut self) {
        if self.call_stack.is_empty() {
            self.arrays.remove("FUNCNAME");
            return;
        }
        let frames: Vec<String> = self
            .call_stack
            .iter()
            .rev()
            .chain(self.script_frames.iter().rev())
            .cloned()
            .collect();
        self.set_array("FUNCNAME", ShellArray::from_values(frames));
    }

    /// Note how the code about to run was reached, for `$FUNCNAME`'s outermost entries.
    ///
    /// `main` for a script file and `source` for a sourced one, which is what bash calls them.
    /// Nothing is pushed for `-c` or for standard input, and bash pushes nothing there either.
    pub fn enter_script_frame(&mut self, kind: &str) {
        self.script_frames.push(kind.to_string());
        self.publish_call_stack();
    }

    /// Leave the frame [`Self::enter_script_frame`] pushed.
    pub fn exit_script_frame(&mut self) {
        self.script_frames.pop();
        self.publish_call_stack();
    }

    /// Note the file whose commands are about to run, for a diagnostic's location.
    ///
    /// **Its own stack rather than `$0`.** A diagnostic from inside a sourced file names *that*
    /// file — bash reports `inner.sh: line 4:` for a failure there, not the script that sourced it
    /// — but `$0` deliberately does *not* change across `source`, because POSIX says a sourced
    /// file shares the caller's positional parameters and `$0` is one of them. Swapping
    /// `shell_name` would have made the message right and `$0` wrong.
    ///
    /// A stack because a sourced file may source another, and the innermost is the one a failure
    /// belongs to. See [`Environment::origin`](crate::env::Environment::origin).
    pub fn enter_source_file(&mut self, path: &str) {
        self.source_files.push(path.to_string());
    }

    /// Leave the file [`Self::enter_source_file`] pushed.
    pub fn exit_source_file(&mut self) {
        self.source_files.pop();
    }

    /// The file a diagnostic should name: the innermost sourced one, or `$0` for the script itself.
    pub(crate) fn current_file(&self) -> &str {
        self.source_files.last().unwrap_or(&self.shell_name)
    }

    /// The shell functions currently executing, innermost last.
    ///
    /// A frame entered through [`Self::enter_function`] rather than
    /// [`Self::enter_function_named`] reads as [`UNNAMED_FUNCTION`].
    pub fn call_stack(&self) -> &[String] {
        &self.call_stack
    }

    /// Whether a shell function is currently executing.
    ///
    /// `local` needs this rather than the scope-frame stack, because a prefix assignment
    /// (`FOO=bar cmd`) pushes a frame too — so a non-empty stack does not mean "inside a
    /// function", and `local x=1` at the top level would silently create a global.
    pub fn in_function(&self) -> bool {
        self.function_depth.depth() > 0
    }
}
