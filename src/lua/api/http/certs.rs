//! Where the root certificates come from: exactly where curl's do.
//!
//! **Nothing is bundled.** A root store compiled into the binary keeps trusting an authority
//! after it has been distrusted — Symantec, Camerfirma and Entrust have all been pulled in living
//! memory — and cannot learn about a new one until oslo is rebuilt and reshipped. curl does not
//! carry one, and neither does this: the system's list is the one the system updates.
//!
//! The cost is that a machine with no certificates cannot make an HTTPS request. That is not a
//! silent failure here — see the error [`load`] produces, which names every path it looked in.
//!
//! The order is curl's, and so are the variable names, so that anything already configured for
//! curl works for oslo without being told twice:
//!
//! 1. `cacert` / `capath` on the request — curl's `--cacert` and `--capath`
//! 2. `$CURL_CA_BUNDLE` — a file
//! 3. `$SSL_CERT_FILE` — a file
//! 4. `$SSL_CERT_DIR` — a directory of certificates
//! 5. the first of the distribution paths below that exists

use rustls_pki_types::CertificateDer;
use std::path::{Path, PathBuf};

/// Where distributions put the bundle, in the order curl's build-time default would pick.
///
/// A list rather than one path chosen when oslo was compiled, because oslo ships as a static
/// binary that runs on distributions it was not built on — the one thing curl's
/// `--with-ca-bundle` cannot do.
const BUNDLE_PATHS: [&str; 6] = [
    // Debian, Ubuntu, Arch, Gentoo
    "/etc/ssl/certs/ca-certificates.crt",
    // Fedora, RHEL, CentOS
    "/etc/pki/tls/certs/ca-bundle.crt",
    // openSUSE
    "/etc/ssl/ca-bundle.pem",
    // Older RHEL
    "/etc/pki/tls/cacert.pem",
    // Alpine, OpenBSD, macOS
    "/etc/ssl/cert.pem",
    // FreeBSD
    "/usr/local/share/certs/ca-root-nss.crt",
];

/// Where distributions put the directory form.
const DIR_PATHS: [&str; 2] = ["/etc/ssl/certs", "/etc/pki/tls/certs"];

/// What a caller asked for, if anything.
#[derive(Default, Clone)]
pub struct Requested {
    /// curl's `--cacert`: one file, and nothing else is consulted.
    pub cacert: Option<String>,
    /// curl's `--capath`: one directory, and nothing else is consulted.
    pub capath: Option<String>,
}

/// Which store to read, once the rules above have been applied.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Dir(PathBuf),
    /// Nothing was named, so the distribution paths are searched.
    Search,
}

/// Apply curl's precedence, without touching the filesystem.
///
/// Separated from the reading so the *rule* can be tested on a machine whose certificates are
/// wherever they happen to be. `get` is the environment, injected for the same reason.
pub fn choose(requested: &Requested, get: impl Fn(&str) -> Option<String>) -> Source {
    if let Some(file) = &requested.cacert {
        return Source::File(PathBuf::from(file));
    }
    if let Some(dir) = &requested.capath {
        return Source::Dir(PathBuf::from(dir));
    }
    for name in ["CURL_CA_BUNDLE", "SSL_CERT_FILE"] {
        if let Some(path) = get(name).filter(|p| !p.is_empty()) {
            return Source::File(PathBuf::from(path));
        }
    }
    if let Some(dir) = get("SSL_CERT_DIR").filter(|p| !p.is_empty()) {
        return Source::Dir(PathBuf::from(dir));
    }
    Source::Search
}

/// Build the trust store, or say what was looked for and not found.
pub fn load(requested: &Requested) -> Result<Vec<CertificateDer<'static>>, String> {
    match choose(requested, |name| std::env::var(name).ok()) {
        // A path that was named on purpose and cannot be read is an error, never something to
        // fall past: quietly trusting a *different* store than the one asked for is the worst
        // possible answer to a question about trust.
        Source::File(path) => from_file(&path),
        Source::Dir(path) => from_dir(&path),
        Source::Search => search(),
    }
}

/// The distribution paths, first that works.
fn search() -> Result<Vec<CertificateDer<'static>>, String> {
    for path in BUNDLE_PATHS {
        if let Ok(store) = from_file(Path::new(path)) {
            return Ok(store);
        }
    }
    for path in DIR_PATHS {
        if let Ok(store) = from_dir(Path::new(path)) {
            return Ok(store);
        }
    }
    // Every place that was looked, because "certificate verify failed" with no explanation is
    // indistinguishable from a network problem, and this is not one.
    Err(format!(
        "no CA certificates found. oslo reads the system's, as curl does, and carries none of \
         its own. Looked in: {}. Set $SSL_CERT_FILE or $SSL_CERT_DIR, pass cacert = \"…\", or \
         install your distribution's ca-certificates package",
        searched().join(", ")
    ))
}

/// Every path the search covers, so the diagnostic and the search cannot drift apart.
pub fn searched() -> Vec<&'static str> {
    BUNDLE_PATHS
        .iter()
        .chain(DIR_PATHS.iter())
        .copied()
        .collect()
}

/// Every certificate in one PEM file.
fn from_file(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut store = Vec::new();
    add_pem(&mut store, &data);
    if store.is_empty() {
        // An empty store accepts no connection at all, so returning one would turn a
        // configuration mistake into "every site is untrusted" — which reads as a network fault.
        return Err(format!("{}: holds no usable certificates", path.display()));
    }
    Ok(store)
}

/// Every certificate in every file of a directory.
///
/// Unreadable and non-certificate files are skipped rather than fatal: `/etc/ssl/certs` on a
/// Debian system is full of symlinks, `README` files and hash-named duplicates, and refusing the
/// whole directory over one of them would leave the shell with no trust store on the most common
/// system there is.
fn from_dir(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let entries = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut store = Vec::new();
    for entry in entries.flatten() {
        if entry.path().is_file()
            && let Ok(data) = std::fs::read(entry.path())
        {
            add_pem(&mut store, &data);
        }
    }
    if store.is_empty() {
        return Err(format!("{}: holds no usable certificates", path.display()));
    }
    Ok(store)
}

/// Add every certificate in some PEM bytes to the store.
///
/// Silent about individual failures on purpose, for the same reason the directory walk is
/// forgiving: one expired or malformed entry in a system bundle must not take the other two
/// hundred with it. The caller checks whether *anything* was loaded.
fn add_pem(store: &mut Vec<CertificateDer<'static>>, data: &[u8]) {
    let mut reader = std::io::BufReader::new(data);
    for certificate in rustls_pemfile::certs(&mut reader).flatten() {
        store.push(certificate.into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::{Requested, Source, choose, from_dir, from_file, load, searched};
    use std::path::PathBuf;

    /// An explicit `cacert` outranks the environment, as curl's `--cacert` does.
    #[test]
    fn precedence_follows_curls() {
        let all = |name: &str| Some(format!("/from/{name}"));

        let explicit = Requested {
            cacert: Some("/explicit.pem".into()),
            capath: None,
        };
        assert_eq!(
            choose(&explicit, all),
            Source::File(PathBuf::from("/explicit.pem"))
        );

        let by_path = Requested {
            cacert: None,
            capath: Some("/explicit.d".into()),
        };
        assert_eq!(
            choose(&by_path, all),
            Source::Dir(PathBuf::from("/explicit.d"))
        );

        // `$CURL_CA_BUNDLE` before `$SSL_CERT_FILE` before `$SSL_CERT_DIR`.
        let none = Requested::default();
        assert_eq!(
            choose(&none, all),
            Source::File(PathBuf::from("/from/CURL_CA_BUNDLE"))
        );
        assert_eq!(
            choose(&none, |n| (n == "SSL_CERT_FILE" || n == "SSL_CERT_DIR")
                .then(|| format!("/from/{n}"))),
            Source::File(PathBuf::from("/from/SSL_CERT_FILE"))
        );
        assert_eq!(
            choose(&none, |n| (n == "SSL_CERT_DIR")
                .then(|| "/from/dir".to_string())),
            Source::Dir(PathBuf::from("/from/dir"))
        );
        // Nothing set at all falls through to the distribution paths.
        assert_eq!(choose(&none, |_| None), Source::Search);
    }

    /// An empty variable is not a request. Exporting `SSL_CERT_FILE=` is how a shell script
    /// clears one, and treating it as "the file named empty string" would be a hard failure
    /// where the user meant "never mind".
    #[test]
    fn an_empty_variable_is_ignored() {
        let none = Requested::default();
        assert_eq!(choose(&none, |_| Some(String::new())), Source::Search);
    }

    /// A file with no certificates in it is refused rather than yielding a store that trusts
    /// nothing — which would look like every site being untrustworthy.
    #[test]
    fn a_bundle_with_no_certificates_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty.pem");
        std::fs::write(&path, b"not a certificate\n").expect("write");
        assert!(from_file(&path).is_err());
        assert!(from_dir(dir.path()).is_err());
    }

    #[test]
    fn a_missing_file_names_itself() {
        let error = from_file(std::path::Path::new("/nonexistent-zz/ca.pem")).unwrap_err();
        assert!(error.contains("/nonexistent-zz/ca.pem"), "{error}");

        // And an explicit request for it is a failure, not a fall-through to the system store.
        let error = load(&Requested {
            cacert: Some("/nonexistent-zz/ca.pem".into()),
            capath: None,
        })
        .unwrap_err();
        assert!(error.contains("/nonexistent-zz/ca.pem"), "{error}");
    }

    /// The "nothing found" message has to name where it looked, or it is indistinguishable from
    /// a network fault.
    #[test]
    fn the_search_paths_are_named_in_the_diagnostic() {
        assert!(searched().contains(&"/etc/ssl/certs/ca-certificates.crt"));
        assert!(searched().contains(&"/etc/ssl/cert.pem"));
        assert!(searched().contains(&"/etc/ssl/certs"));
    }

    /// If this machine has a system store, reading it must actually produce certificates — the
    /// parser needs exercising against a real bundle, not only against the error paths.
    #[test]
    fn a_real_system_store_loads_if_this_machine_has_one() {
        let Some(found) = searched()
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            // A machine with no certificates is a legitimate thing to build oslo on.
            return;
        };
        let store = load(&Requested::default()).expect("the system store must be readable");
        assert!(!store.is_empty(), "{found} produced an empty store");
    }
}
