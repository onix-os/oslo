//! Hooks doing a store's crypto, and Lua supplying a store's storage.
#![cfg(feature = "secrets")]

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run `script` as Lua against a home of its own, with `conf` as `secrets.conf`.
fn lua(home: &std::path::Path, conf: &str, script: &str) -> (String, String) {
    let state = home.join("state/oslo");
    std::fs::create_dir_all(&state).expect("mkdir");
    std::fs::write(state.join("secrets.conf"), conf).expect("write");
    let file = home.join("script.lua");
    std::fs::write(&file, script).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("XDG_DATA_HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
        .env_remove("OSLO_SECRET_IDENTITY")
        .env_remove("OSLO_SECRET_STORE")
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const BOTH_AXES: &str = r#"
oslo.on["on-secret-encrypt"](function(store, name, b64)
  if store ~= "vault" then return nil end
  return b64
end)
oslo.on["on-secret-decrypt"](function(store, name, b64)
  if store ~= "vault" then return nil end
  return b64
end)
local kept = {}
oslo.secret.define("vault", {
  get = function(n) return kept[n] end,
  set = function(n, s) kept[n] = s return true end,
  list = function() local o = {} for k in pairs(kept) do o[#o+1] = k end return o end,
})
local v = oslo.secret.open("vault")
v:set("token", "through-lua")
print("stored:", kept.token)
print("read:", v:get("token"))
print("list:", table.concat(v:list(), ","))
"#;

/// **Both axes at once**: a store whose bytes live in Lua and whose crypto is a Lua handler. Nothing
/// of oslo's age is involved, and the surface above it does not know that.
#[test]
fn a_store_can_have_lua_for_both_its_storage_and_its_crypto() {
    let home = tempfile::tempdir().expect("tempdir");
    let (out, err) = lua(home.path(), "[vault]\ncrypto hook\n", BOTH_AXES);
    assert!(out.contains("read:\tthrough-lua"), "{out:?} {err}");
    assert!(out.contains("list:\ttoken"), "{out:?}");
    // What the storage was handed is base64 of what the hook answered, never the value.
    assert!(
        !out.contains("stored:\tthrough-lua"),
        "in the clear: {out:?}"
    );
    assert!(out.contains("stored:\tdGhyb3VnaC1sdWE="), "{out:?}");
}

/// **`nil` means "not mine"**, so one hook serves many stores and the ones nobody claims are
/// untouched — the user's own store keeps using age with a handler attached.
#[test]
fn a_handler_that_declines_leaves_the_other_stores_alone() {
    let home = tempfile::tempdir().expect("tempdir");
    let (out, err) = lua(
        home.path(),
        "[vault]\ncrypto hook\n",
        r#"
        oslo.on["on-secret-encrypt"](function(store) return nil end)
        oslo.on["on-secret-decrypt"](function(store) return nil end)
        oslo.secret.set("mine", "native-value")
        print("user:", oslo.secret.get("mine"))
        local v = oslo.secret.open("vault")
        print("vault:", select(2, v:set("x", "y")))
        "#,
    );
    assert!(out.contains("user:\tnative-value"), "{out:?} {err}");
    assert!(
        out.contains("no on-secret-encrypt handler claimed this store"),
        "an unclaimed hook store is a refusal, not a fallback: {out:?}"
    );
}

/// The watching hooks see every access, on the native path and the defined one — and never a value.
#[test]
fn the_watching_hooks_are_told_which_and_how_but_not_what() {
    let home = tempfile::tempdir().expect("tempdir");
    let (out, err) = lua(
        home.path(),
        "",
        r#"
        oslo.on["pre-secret-access"](function(e) print("pre", e.store, e.name, e.how) end)
        oslo.on["post-secret-access"](function(e) print("post", e.store, e.name, e.how) end)
        oslo.secret.set("tok", "a-value")
        oslo.secret.get("tok")
        "#,
    );
    assert!(out.contains("pre\tuser\ttok\twrite"), "{out:?} {err}");
    assert!(out.contains("post\tuser\ttok\twrite"), "{out:?}");
    assert!(out.contains("pre\tuser\ttok\tread"), "{out:?}");
    assert!(
        !out.contains("a-value"),
        "the value reached a watcher: {out:?}"
    );
}

/// **The file declares the mechanism, so nothing diverges silently.** A process with no Lua meets
/// `crypto hook` and says so, rather than reporting a missing file or inventing a fallback.
#[test]
fn a_hook_backed_store_is_a_loud_failure_where_lua_does_not_run() {
    let home = tempfile::tempdir().expect("tempdir");
    let state = home.path().join("state/oslo");
    std::fs::create_dir_all(&state).expect("mkdir");
    std::fs::write(state.join("secrets.conf"), "[vault]\ncrypto hook\n").expect("write");

    for verb in [vec!["get", "token"], vec!["list"], vec!["set", "token"]] {
        let out = Command::new(oslo_bin())
            .args(["secret", "--store", "vault"])
            .args(&verb)
            .env("XDG_DATA_HOME", home.path())
            .env("XDG_STATE_HOME", home.path().join("state"))
            .stdin(std::process::Stdio::null())
            .output()
            .expect("spawn oslo");
        assert!(!out.status.success(), "{verb:?} succeeded");
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(said.contains("crypto hook"), "{verb:?}: {said:?}");
        assert!(said.contains("cron never does"), "{verb:?}: {said:?}");
    }
}

/// Two mechanisms in one store is a contradiction, not a precedence question: they have different
/// reach, and quietly picking one would make the store readable in one process and not the other.
#[test]
fn a_store_may_not_declare_two_mechanisms() {
    let home = tempfile::tempdir().expect("tempdir");
    let state = home.path().join("state/oslo");
    std::fs::create_dir_all(&state).expect("mkdir");
    std::fs::write(
        state.join("secrets.conf"),
        "[vault]\ncrypto hook\nencrypt command /bin/cat\n",
    )
    .expect("write");

    let out = Command::new(oslo_bin())
        .args(["secret", "--store", "vault", "list"])
        .env("XDG_DATA_HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .expect("spawn oslo");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("pick one"), "{said:?}");
}

/// `user` and a plugin's store are not names Lua may take over: one is what the command line means,
/// the other is reached only through `oslo.secret.mine()`.
#[test]
fn the_two_reserved_names_cannot_be_defined() {
    let home = tempfile::tempdir().expect("tempdir");
    let (out, err) = lua(
        home.path(),
        "",
        r#"
        local nothing = { get=function() end, set=function() end, list=function() return {} end }
        print("user:", select(2, oslo.secret.define("user", nothing)))
        print("plugin:", select(2, oslo.secret.define("plugin.notes", nothing)))
        print("other:", oslo.secret.define("work", nothing))
        "#,
    );
    assert!(out.contains("user:\tuser: not a store"), "{out:?} {err}");
    assert!(
        out.contains("plugin:\tplugin.notes: not a store"),
        "{out:?}"
    );
    assert!(out.contains("other:\ttrue"), "{out:?}");
}

/// **The shape a real `age` + `age-plugin-yubikey` takes.** A hook cannot build this out of
/// `oslo.run`: it is handed base64, `age` speaks raw bytes on a pipe, and a Lua string cannot carry
/// those — while putting the payload in argv would publish the plaintext to `ps`. So
/// `oslo.secret.through` is the pipe, and this is the whole configuration somebody writes.
#[test]
fn a_hook_can_drive_an_external_age_over_a_pipe() {
    let home = tempfile::tempdir().expect("tempdir");
    let age = home.path().join("age");
    std::fs::write(
        &age,
        "#!/bin/sh\necho 'touch the key' >&2\ncase \"$1\" in\n  -R) printf 'AGE-ISH\\n'; base64 ;;\n  --decrypt) tail -n +2 | base64 -d ;;\nesac\n",
    )
    .expect("write");
    std::fs::set_permissions(&age, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("chmod");
    let age = age.to_string_lossy().to_string();

    let (out, err) = lua(
        home.path(),
        "[yubi]\ncrypto hook\n",
        &format!(
            r#"
            oslo.on["on-secret-encrypt"](function(store, name, sealed)
              if store ~= "yubi" then return nil end
              return oslo.secret.through({{ "{age}", "-R", "recipients.txt" }}, sealed)
            end)
            oslo.on["on-secret-decrypt"](function(store, name, sealed)
              if store ~= "yubi" then return nil end
              return oslo.secret.through({{ "{age}", "--decrypt", "--identity", "yk.txt" }}, sealed)
            end)
            local v = oslo.secret.open("yubi")
            v:set("deploy", "held-in-the-device")
            print("read:", v:get("deploy"))
            "#
        ),
    );
    assert!(out.contains("read:\theld-in-the-device"), "{out:?} {err}");

    // What the store holds is the other program's format, and the value is not in it.
    let kept = std::fs::read_to_string(home.path().join("oslo/stores/yubi/deploy.sealed"))
        .expect("the secret was written");
    assert!(kept.starts_with("AGE-ISH"), "{kept:?}");
    assert!(
        !kept.contains("held-in-the-device"),
        "in the clear: {kept:?}"
    );

    // **The prompt reached a terminal rather than a pipe.** A key in hardware asks for a touch on
    // standard error, and a captured prompt is a shell that appears to hang for no visible reason.
    assert!(
        err.contains("touch the key"),
        "the prompt was swallowed: {err:?}"
    );
}
