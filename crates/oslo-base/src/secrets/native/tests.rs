//! The sealed box, against the properties that make it worth having.

use super::*;

fn keypair() -> ([u8; KEY], [u8; KEY]) {
    let secret = generate_secret().expect("a key");
    (public_of(&secret), secret)
}

#[test]
fn what_goes_in_comes_back_out() {
    let (public, secret) = keypair();
    for value in [b"".as_slice(), b"short", &[0u8; 100_000]] {
        let sealed = seal(&[public], value).expect("seal");
        assert!(
            !sealed.windows(value.len().max(1)).any(|w| w == value) || value.is_empty(),
            "the value is in the file in the clear"
        );
        assert_eq!(unseal(&secret, &sealed).expect("unseal"), value);
    }
}

/// **The whole reason this is not one symmetric key.** Two people, two secrets neither has shared,
/// one file both can open.
#[test]
fn several_recipients_can_each_open_it_with_their_own_key() {
    let (mine, my_secret) = keypair();
    let (theirs, their_secret) = keypair();
    let (strangers, stranger_secret) = keypair();

    let sealed = seal(&[mine, theirs], b"the deploy token").expect("seal");

    assert_eq!(
        unseal(&my_secret, &sealed).expect("mine"),
        b"the deploy token"
    );
    assert_eq!(
        unseal(&their_secret, &sealed).expect("theirs"),
        b"the deploy token"
    );
    // And somebody who is not on the list is not on the list, however many stanzas they try.
    let refused = unseal(&stranger_secret, &sealed).expect_err("a stranger opened it");
    assert!(refused.contains("no key of yours"), "{refused}");
    assert_ne!(strangers, mine);
}

/// Two writes of one value differ: equal files would tell anybody holding two backups which
/// secrets had not changed between them.
#[test]
fn the_same_value_seals_differently_every_time() {
    let (public, secret) = keypair();
    let once = seal(&[public], b"the same value").expect("seal");
    let twice = seal(&[public], b"the same value").expect("seal");
    assert_ne!(once, twice);
    assert_eq!(
        unseal(&secret, &once).expect("a"),
        unseal(&secret, &twice).expect("b")
    );
}

/// It is an AEAD, so a file somebody has edited is refused rather than decrypted into rubbish —
/// including an edit to the recipient stanzas rather than to the payload.
#[test]
fn a_changed_file_is_refused_wherever_it_was_changed() {
    let (public, secret) = keypair();
    let sealed = seal(&[public], b"value").expect("seal");

    for at in [MAGIC.len() + 1, sealed.len() - 1] {
        let mut bent = sealed.clone();
        bent[at] ^= 1;
        assert!(unseal(&secret, &bent).is_err(), "a change at {at} opened");
    }
    assert!(unseal(&secret, b"nonsense").unwrap_err().contains("OSLO2"));
    // A truncated file says where it stopped rather than panicking on a slice.
    assert!(unseal(&secret, &sealed[..MAGIC.len() + 2]).is_err());
}

/// A secret and a recipient are different things and are written differently, so neither can be
/// pasted where the other belongs.
#[test]
fn a_key_and_a_recipient_do_not_look_alike() {
    let (public, secret) = keypair();
    let written = write_secret(&secret);
    let published = write_public(&public);

    assert!(written.starts_with("OSLO-KEY-1:"));
    assert!(published.starts_with("OSLO-PUB-1:"));
    assert_eq!(read_secret(&written).expect("read"), secret);
    assert_eq!(read_public(&published).expect("read"), public);

    // The one that matters: a recipient handed to the key reader is refused, not silently used as
    // a secret — which would make the store readable by anybody holding the published half.
    assert!(
        read_secret(&published).is_err(),
        "a recipient was read as a key"
    );
    assert!(
        read_public(&written).is_err(),
        "a key was read as a recipient"
    );

    assert_eq!(
        read_secret(&format!("# a comment\n{written}")).expect("read"),
        secret
    );
    for bad in [
        "",
        "# only a comment",
        "not-a-key",
        "OSLO-KEY-1:!!!",
        "OSLO-KEY-1:c2hvcnQ=",
    ] {
        assert!(read_secret(bad).is_err(), "{bad:?} was accepted");
    }
}

/// Writing to nobody is refused rather than producing a file nothing can open.
#[test]
fn a_store_with_no_recipients_cannot_be_written() {
    assert!(seal(&[], b"value").unwrap_err().contains("no recipient"));
}
