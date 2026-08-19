//! `require "oslo"`, and `require "oslo.ui"` — the namespaces as modules.

use oslo_luavm::Host;

/// Make every `oslo.X` table `require`-able as `oslo.X`.
///
/// **This is what makes them libraries rather than fields on a global.** A namespace you can only
/// reach by indexing a global is a naming convention; a module you can `require` is a thing with a
/// name, that can be aliased locally, shadowed by one of your own in `~/.config/oslo/lua`, and
/// depended on by a library that never mentions `oslo` at all:
///
/// ```lua
/// local ui = require "oslo.ui"
/// local env = require "oslo.env"
/// ```
///
/// Registered in `package.preload` rather than on disk, so the lookup never touches the filesystem
/// and a user's own `oslo/ui.lua` cannot shadow the built-in by accident — `preload` wins.
///
/// # It is written in Lua, and it has to be
///
/// **`require "oslo"` must answer with the table the global *is*, not a copy of it.** Registering a
/// native that hands back a shell-side value looks equivalent and is not: every value crossing the
/// boundary is converted, so each call produced a fresh table. `require "oslo"` and `oslo` were
/// then two different objects, and
///
/// ```lua
/// local oslo = require "oslo"
/// oslo.completion.max_rows = 42     -- written into a copy nothing reads
/// ```
///
/// was silently discarded — the worst shape a configuration bug can take, because the file looks
/// right and the setting simply never happens. Indexing the global from inside Lua reaches the VM's
/// own table, so the closure captures the real thing and identity holds:
/// `require "oslo" == oslo`.
pub(super) fn publish(host: &dyn Host) {
    // Only the tables. A function on `oslo` is not a module, and registering one would make
    // `require "oslo.glob"` answer with something that is not what `require` promises.
    let source = r#"
        package.preload["oslo"] = function() return oslo end
        for name, value in pairs(oslo) do
            if type(value) == "table" then
                package.preload["oslo." .. name] = function() return value end
            end
        end
    "#;
    if let Err(e) = host.eval(source, "=oslo.publish") {
        oslo_base::messages::error("oslo modules", e.to_string());
    }
}
