//! Does a subject's chosen name survive the invite, and does adding it break
//! every invite already handed out?
//!
//! **THE SECOND QUESTION IS THE IMPORTANT ONE.** An invite is scanned off a
//! screen or pasted into a message; the ones already in people's phones do not
//! stop existing because the format grew a field. v1, v2 and v3 all still parse,
//! and a subject with no handle still emits exactly what it emitted before — so
//! the only invites that say `4` are the ones that have something new to say.

use diaswarm_core::invite::Invite;

fn subject() -> String {
    "aa".repeat(32)
}
fn endpoint() -> String {
    "bb".repeat(32)
}

fn base() -> Invite {
    Invite::new_via(&subject(), &endpoint(), "follow", "https://relay.example/")
        .expect("invite")
}

/// A NAME GOES IN AND COMES BACK OUT, UNCHANGED.
#[test]
fn a_handle_survives_the_round_trip() {
    let invite = base().with_keys(&"cc".repeat(32)).unwrap().with_handle("Alex").unwrap();
    let text = invite.encode();
    assert!(text.starts_with("diaswarm:4:"), "a handle should emit v4, got {text}");

    let back = Invite::parse(&text).expect("parse");
    assert_eq!(back.handle, "Alex");
    assert_eq!(back.subject, invite.subject);
    assert_eq!(back.keys, invite.keys);
}

/// AND ITS CASE SURVIVES, WHICH THE RELAY'S DELIBERATELY DOES NOT.
///
/// A relay is a URL and case carries nothing, so it is normalised. A person's
/// name is not a URL and "alex" is not what they typed.
#[test]
fn a_handle_keeps_its_case() {
    let invite = base().with_keys(&"cc".repeat(32)).unwrap().with_handle("Alex McTest").unwrap();
    let back = Invite::parse(&invite.encode()).expect("parse");
    assert_eq!(back.handle, "Alex McTest", "the name was normalised and should not be");
}

/// A COLON IN A NAME DOES NOT SPLIT THE INVITE.
///
/// The format is colon-delimited, so this is the field-separator injection the
/// escaping exists for — and a name is the first field a person can type freely.
#[test]
fn a_handle_containing_the_separator_still_round_trips() {
    let awkward = "Alex: the second";
    let invite = base().with_keys(&"cc".repeat(32)).unwrap().with_handle(awkward).unwrap();
    let back = Invite::parse(&invite.encode()).expect("parse");
    assert_eq!(back.handle, awkward, "a colon in a name broke the invite");
}

/// NO HANDLE MEANS THE OLD SHAPE, BYTE FOR BYTE.
///
/// This is what stops the change reaching anyone who has no use for it.
#[test]
fn without_a_handle_the_invite_is_unchanged() {
    let with_keys = base().with_keys(&"cc".repeat(32)).unwrap();
    assert!(
        with_keys.encode().starts_with("diaswarm:3:"),
        "a keys invite with no handle must still be v3"
    );
    let plain = base();
    assert!(
        plain.encode().starts_with("diaswarm:2:"),
        "an invite with neither keys nor handle must still be v2"
    );
}

/// EVERY OLDER INVITE STILL PARSES.
#[test]
fn older_invites_still_parse() {
    for invite in [base(), base().with_keys(&"cc".repeat(32)).unwrap()] {
        let text = invite.encode();
        let back = Invite::parse(&text).unwrap_or_else(|e| panic!("{text} failed: {e:?}"));
        assert_eq!(back.handle, "", "an invite with no handle should parse to none");
    }
}

/// A NAME IS BOUNDED, BECAUSE IT HAS TO PHOTOGRAPH.
#[test]
fn an_overlong_handle_is_refused() {
    let err = base().with_handle(&"a".repeat(41)).unwrap_err();
    assert!(format!("{err:?}").contains("40"), "unhelpful refusal: {err:?}");
    base().with_handle(&"a".repeat(40)).expect("40 characters should be allowed");
}

/// AND IT CANNOT LIE ABOUT WHICH WAY IT READS.
///
/// **THIS IS THE ONE THAT MATTERS.** A handle is drawn next to a key. A
/// bidirectional override makes what is displayed differ from what is stored,
/// which is exactly the confusion a label beside an identity must not create.
#[test]
fn a_handle_cannot_carry_direction_overrides_or_control_characters() {
    for bad in ["Alex\u{202e}drowssap", "Alex\u{2066}x", "Alex\nx", "Alex\u{0}x"] {
        assert!(
            base().with_handle(bad).is_err(),
            "accepted a handle containing a control or direction character: {bad:?}"
        );
    }
    // Accents and non-Latin scripts are names, not attacks.
    for good in ["Ærøskøbing", "أحمد", "小明", "José"] {
        base().with_handle(good).unwrap_or_else(|e| panic!("refused a real name {good:?}: {e:?}"));
    }
}

/// A DAMAGED v4 INVITE IS STILL CAUGHT.
///
/// The checksum covers the handle, so editing the name after the fact does not
/// quietly produce a valid invite for somebody else's key.
#[test]
fn tampering_with_the_handle_fails_the_checksum() {
    let invite = base().with_keys(&"cc".repeat(32)).unwrap().with_handle("Alex").unwrap();
    let text = invite.encode();
    let tampered = text.replace(":Alex:", ":Mum:");
    assert_ne!(tampered, text, "the test did not actually change anything");
    assert!(
        Invite::parse(&tampered).is_err(),
        "a renamed invite kept its checksum — the handle is outside the check"
    );
}
