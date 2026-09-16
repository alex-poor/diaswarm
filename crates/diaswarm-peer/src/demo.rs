//! A subject that publishes invented glucose, so somebody can try a follower.
//!
//! **BECAUSE A FOLLOWER ALONE DOES NOTHING, AND THAT IS NOT A BUG.** Ayni shows
//! somebody else's readings. It needs a subject to grant it, and the only
//! subject that exists is the AndroidAPS add-on, which [is deliberately not
//! distributed](../../../fdroid/nz.diaswarm.ayni.yml) — running a loop means
//! building a medical device from source you have read.
//!
//! So anyone evaluating the follower — an F-Droid reviewer, somebody deciding
//! whether to trust it, a developer with one phone — installs it, sees *"Not
//! following anyone yet"*, and stops. **There is nothing they can do with it.**
//! This is the missing other half.
//!
//! 🔴 **THE READINGS ARE INVENTED, AND MUST BE.** Handing a stranger a grant on
//! a real person's vault would be a stranger reading somebody's glucose, and no
//! review convenience is worth that. Everything here is a synthetic curve
//! generated in this file. It is labelled as a demo in the handle so nobody can
//! mistake it for a person.
//!
//! **IT IS AN ORDINARY SUBJECT OTHERWISE.** Same vaults, same sealing, same
//! grant, same pool, same relay. A demo that took a shortcut through the
//! protocol would prove nothing about the protocol.

use std::path::Path;

use anyhow::Result;
use diaswarm_core::vault::Identity;
use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_net::replicate::KeysReplicator;

/// Where the day's curve sits, in mg/dL, and how far it wanders.
const MEAN: f64 = 130.0;
const SWING: f64 = 55.0;

/// One plausible reading for a moment in time.
///
/// **A SINE PLUS A WOBBLE, NOT A RANDOM WALK.** A walk drifts out of
/// physiological range within a day and makes a demo that looks broken; a
/// daily rhythm with meal-shaped bumps reads like a person and stays where
/// glucose actually lives. It is deterministic in the timestamp, so a restart
/// continues the same curve rather than jumping.
pub fn reading_at(ms: i64) -> f64 {
    let minutes = (ms / 60_000) as f64;
    let day = (minutes % 1440.0) / 1440.0 * std::f64::consts::TAU;
    // Three meals, as bumps an hour or so wide.
    let meals: f64 = [8.0, 13.0, 19.0]
        .iter()
        .map(|h| {
            let dt = (minutes % 1440.0) / 60.0 - h;
            30.0 * (-(dt * dt) / 1.2).exp()
        })
        .sum();
    // A slow wobble so consecutive readings are not a clean curve.
    let wobble = 6.0 * ((minutes / 17.0).sin() + (minutes / 7.0).cos() * 0.5);
    (MEAN + SWING * 0.45 * day.sin() + meals + wobble).clamp(55.0, 320.0)
}

/// The readings for a window ending now, one a minute.
///
/// One a minute because that is what the CGM this project was built around
/// actually does — a Libre 3 reports every minute, not every five, and a demo
/// at the wrong cadence would give a follower the wrong idea of what it holds.
pub fn readings_for(now_ms: i64, minutes: i64) -> Vec<Record> {
    (0..minutes)
        .map(|i| {
            let t = now_ms - (minutes - 1 - i) * 60_000;
            Record::new(t, "cgm")
                .set("mgdl", Some(reading_at(t).round().into()))
                .set("trend", Some("FLAT".into()))
                .set("src", Some("DEMO".into()))
        })
        .collect()
}

/// The epoch a moment falls in, for a subject at `offset_ms` from UTC.
pub fn epoch_of(ms: i64, offset_ms: i64) -> i64 {
    (ms + offset_ms).div_euclid(EPOCH_MS)
}

/// Seal a window of invented readings into both vaults and publish them.
///
/// **BOTH VAULTS, BECAUSE A FOLLOWER MAY READ EITHER.** Ayni has a toggle for
/// which one it trusts, and a demo that filled only one would look broken to
/// half the people who tried it.
///
/// 🔴 **SEALING AND ANNOUNCING ARE ONE STEP HERE, DELIBERATELY.** `keysRotate`
/// sealed and dropped the message it was supposed to publish, and a follower
/// went dark for an afternoon before anyone could see why. Anything that
/// produces an operation in this file pushes it before returning.
pub async fn publish_window(
    core: &diaswarm_core::vault::Vault,
    keys: &mut diaswarm_keys::Vault,
    store: &diaswarm_keys::SqliteStore,
    signing: &p2panda_core::SigningKey,
    push: Option<&KeysReplicator>,
    now_ms: i64,
    minutes: i64,
    offset_ms: i64,
) -> Result<usize> {
    let records = readings_for(now_ms, minutes);
    let epoch = epoch_of(now_ms, offset_ms);

    // The core vault re-wraps for every known reader as part of sealing, so a
    // reader granted before this window opens it without a second call.
    core.seal(epoch, &records).map_err(|e| anyhow::anyhow!("sealing into the core vault: {e:?}"))?;

    let segment =
        keys.seal(epoch, &records).map_err(|e| anyhow::anyhow!("sealing into the keys vault: {e}"))?;
    let op = diaswarm_keys::wire::publish(store, signing, &segment)
        .await
        .map_err(|e| anyhow::anyhow!("publishing a segment: {e}"))?;
    if let Some(rep) = push {
        rep.broadcast(&signing.verifying_key().to_hex(), op);
    }

    Ok(records.len())
}

/// Let a reader in, from whatever string they were able to hand over.
///
/// **THE ARGUMENT IS THE FOLLOWER'S INVITE, NOT THE SUBJECT'S.** Ayni's "Show
/// my invite" prints the key a subject grants — the same string a person would
/// scan off the other phone. Taking it whole means the demo is driven by
/// exactly what a reviewer already has on screen, with nothing to extract by
/// hand.
///
/// **A BARE KEYS IDENTITY IS ALSO ACCEPTED**, which is what
/// `diaswarm-peer identity` prints and what a desktop follower has. Android's
/// `keysGrant` already takes either for the same reason: a person pasting the
/// thing their app gave them should get the grant they asked for, not a lecture
/// about which of two formats it was. What they cannot get that way is the core
/// vault — a bare identity carries no core reader key to wrap for — so the
/// return value says which vaults were actually opened up.
pub async fn grant_reader(
    core: &diaswarm_core::vault::Vault,
    subject: &Identity,
    keys: &mut diaswarm_keys::Vault,
    store: &diaswarm_keys::SqliteStore,
    signing: &p2panda_core::SigningKey,
    push: Option<&KeysReplicator>,
    pasted: &str,
) -> Result<String> {
    let pasted = pasted.trim();

    // (reader's core key if we have one, their keys identity, purpose)
    let (reader, keys_hex, purpose) = match diaswarm_core::invite::Invite::parse(pasted) {
        Ok(invite) => {
            let reader = invite
                .subject_bytes()
                .map_err(|e| anyhow::anyhow!("the invite's key is unusable: {e:?}"))?;
            (Some(reader), invite.keys.clone(), invite.purpose.clone())
        }
        // Not an invite. The only other thing worth trying is the bare identity
        // `diaswarm-peer identity` prints; anything else fails below with the
        // decoder's own complaint, which is more specific than ours.
        Err(_) => (None, pasted.to_string(), "follow".to_string()),
    };

    let mut opened: Vec<&str> = Vec::new();

    if let Some(reader) = reader {
        core.record_grant(subject, &reader, &purpose, "grant", 0)
            .map_err(|e| anyhow::anyhow!("recording the grant: {e:?}"))?;
        core.publish_wraps(subject, &reader, &purpose)
            .map_err(|e| anyhow::anyhow!("wrapping for the reader: {e:?}"))?;
        opened.push("core");
        if !keys_hex.is_empty() {
            // Written down so a withdrawal can find both halves of this reader.
            core.remember_reader_keys(
                &diaswarm_core::vault::hex(&reader),
                &purpose,
                &keys_hex,
            )
            .map_err(|e| anyhow::anyhow!("noting the reader's keys identity: {e:?}"))?;
        }
    }

    // **A v2 INVITE CARRIES NO KEYS IDENTITY, AND THAT IS NOT AN ERROR HERE.**
    // It is an older follower. The core grant above already works for it; what
    // it cannot have is the keys vault, and Ayni says so on its own screen.
    if !keys_hex.is_empty() {
        let bundle = diaswarm_keys::decode_identity(&keys_hex)
            .map(|id| id.bundle)
            .or_else(|_| diaswarm_keys::decode_bundle(&keys_hex))
            .map_err(|e| anyhow::anyhow!("that is not an invite or a keys identity: {e}"))?;
        match keys.grant(bundle, &purpose) {
            Ok((welcome, _)) => {
                let op = diaswarm_keys::wire::publish_control(store, signing, &welcome)
                    .await
                    .map_err(|e| anyhow::anyhow!("publishing the welcome: {e}"))?;
                if let Some(rep) = push {
                    rep.broadcast(&signing.verifying_key().to_hex(), op);
                }
            }
            // Already in is success: the caller asked for this reader to be able
            // to read, and they can. A second welcome would be the bug.
            Err(diaswarm_keys::Error::AlreadyGranted(_)) => {}
            Err(e) => return Err(anyhow::anyhow!("granting on the keys vault: {e}")),
        }
        opened.push("keys");
    }

    if opened.is_empty() {
        anyhow::bail!("that invite carries neither a reader key nor a keys identity");
    }
    Ok(format!("{} vault", opened.join(" and ")))
}

/// The name the demo travels under, so nobody can mistake it for a person.
pub const HANDLE: &str = "demo (not a real person)";

/// The invite a follower needs.
///
/// `handle` is the name that travels in the invite, and passing `None` is not
/// cosmetic — see [`invites_for`].
pub fn invite_for(
    core: &diaswarm_core::vault::Vault,
    keys: &diaswarm_keys::Vault,
    endpoint: &str,
    handle: Option<&str>,
) -> Result<String> {
    let subject = diaswarm_core::vault::hex(&core.subject_pub());
    let identity = keys.identity().map_err(|e| anyhow::anyhow!("identity: {e}"))?;
    let keys_hex =
        diaswarm_keys::encode_identity(&identity).map_err(|e| anyhow::anyhow!("encode: {e}"))?;
    diaswarm_core::invite::Invite::new(&subject, endpoint, "follow")
        .and_then(|i| i.with_keys(&keys_hex))
        .and_then(|i| i.with_handle(handle.unwrap_or("")))
        .map(|i| i.encode())
        .map_err(|e| anyhow::anyhow!("composing the invite: {e:?}"))
}

/// Both shapes of the demo's invite: the named one, and one an older build can
/// still read.
///
/// 🔴 **AN INVITE A YEAR NEWER THAN THE APP IS A DEAD END, AND THIS COMMAND'S
/// AUDIENCE IS EXACTLY THE PEOPLE MOST LIKELY TO HIT IT.** `Invite::parse`
/// refuses a version it does not know — correctly, since it cannot check a
/// field it has never heard of — and says *"update to use it"*. A reviewer
/// installing Ayni from F-Droid gets whatever version that build is pinned to,
/// which is not this working tree: at the time of writing the published
/// metadata builds 0.1.6, whose `invite.rs` tops out at v3. Handed only the
/// named invite, they would see a refusal and reasonably conclude the app does
/// not work.
///
/// So the demo prints both. The named one carries [`HANDLE`], which is the
/// label that stops anyone mistaking invented readings for a person's; the
/// fallback drops the name and nothing else, and a follower on the older build
/// sees hex where the name would be. `encode` picks the version from the fields
/// that are actually set, so the second is a genuine v3 rather than a v4 with a
/// blank in it.
pub fn invites_for(
    core: &diaswarm_core::vault::Vault,
    keys: &diaswarm_keys::Vault,
    endpoint: &str,
) -> Result<(String, String)> {
    Ok((
        invite_for(core, keys, endpoint, Some(HANDLE))?,
        invite_for(core, keys, endpoint, None)?,
    ))
}

/// Where the demo's **core** vault has to live, and it is not a choice.
///
/// ⚠️ **NAMED FOR THE SUBJECT, BECAUSE THAT IS HOW THE WIRE FINDS IT.**
/// `diaswarm_net::answer` resolves every core-vault request by joining the
/// store directory to the 64-hex subject in the request — so a vault under any
/// other name is served to nobody, and a follower pointed at it gets the same
/// empty manifest it would get for a subject this peer has never heard of. The
/// first draft of this file used `demo-core/` and would have looked like a
/// network fault.
pub fn core_dir(dir: &Path, subject_hex: &str) -> std::path::PathBuf {
    dir.join(subject_hex.to_ascii_lowercase())
}

/// The keys vault, which is found by signing key rather than by path and so
/// may be called anything.
pub fn keys_dir(dir: &Path) -> std::path::PathBuf {
    dir.join("demo-keys")
}

/// The demo's core identity — its own, never the peer's.
///
/// A carrier already has an identity, and reusing it would make the demo and
/// the daemon the same subject: withdrawing the demo would withdraw the
/// carrier, and a reviewer's grant would be a grant on whatever else that peer
/// holds. Separate files, separate subject.
pub fn identity_path(dir: &Path) -> std::path::PathBuf {
    dir.join("demo-identity.key")
}

/// The key the demo's keys vault publishes under. Its hex is the demo's subject
/// as the pool knows it.
pub fn keys_signing_path(dir: &Path) -> std::path::PathBuf {
    dir.join("demo-keys.key")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "diaswarm-demotest-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 🔴 A DEMO VAULT UNDER THE WRONG NAME IS SERVED TO NOBODY.
    ///
    /// The first draft of this file put the core vault in `demo-core/`, because
    /// that reads well. `diaswarm_net::answer` resolves a core-vault request by
    /// joining the store directory to the 64-hex subject, so that vault would
    /// have answered every follower with the same empty manifest a peer gives
    /// for a subject it has never heard of — a naming mistake wearing the
    /// costume of a network fault. Nothing in a compile or a unit test of the
    /// curve could see it; only asking the wire can.
    #[tokio::test]
    async fn the_wire_can_find_the_demo_vault_by_subject() {
        let dir = tmp("wire");
        let identity = Identity::generate();
        let subject = diaswarm_core::vault::hex(&identity.enc_public());

        let core = diaswarm_core::vault::Vault::create(&core_dir(&dir, &subject), &identity, 0)
            .expect("create");
        core.seal(20_000, &readings_for(1_789_000_000_000, 10)).expect("seal");

        // The question a follower's first fetch asks, answered by the same
        // function the server calls.
        let reply = diaswarm_net::answer(
            &dir,
            &diaswarm_net::Request::Manifest { subject: subject.clone() },
        )
        .expect("the wire could not find the demo vault");
        let manifest: diaswarm_net::Manifest =
            serde_json::from_slice(&reply).expect("not a manifest");
        assert!(!manifest.segments.is_empty(), "the demo vault served no segments");

        // And it is listed, so a peer browsing what this carrier holds sees it.
        let have = diaswarm_net::answer(&dir, &diaswarm_net::Request::Have).expect("have");
        let subjects: Vec<String> = serde_json::from_slice(&have).unwrap();
        assert!(subjects.contains(&subject), "the demo subject is not in `Have`: {subjects:?}");
    }

    /// 🔴 THE FALLBACK INVITE MUST STAY READABLE BY A PUBLISHED BUILD.
    ///
    /// `Invite::parse` refuses a version it does not know, so an invite one
    /// version ahead of the installed app is a dead end that reads as "this app
    /// does not work" — and the audience for this command is precisely the
    /// people running a published build rather than this tree. At the time of
    /// writing the F-Droid metadata builds Ayni 0.1.6, whose invite.rs tops out
    /// at v3.
    ///
    /// **THIS TEST IS A TRIPWIRE, NOT A PREFERENCE.** If a later field bumps
    /// the fallback off v3, this fails and somebody has to decide whether every
    /// installed follower can still read it. That decision must not be made by
    /// accident.
    #[test]
    fn the_fallback_invite_is_one_an_older_follower_can_read() {
        let dir = tmp("shapes");
        let identity = Identity::generate();
        let signing = p2panda_core::SigningKey::generate();
        let core = diaswarm_core::vault::Vault::create(
            &core_dir(&dir, &diaswarm_core::vault::hex(&identity.enc_public())),
            &identity,
            0,
        )
        .unwrap();
        let rng = diaswarm_keys::Rng::default();
        let mut keys = diaswarm_keys::Vault::open(&keys_dir(&dir), 0, &signing).unwrap();
        let (mgr, _) = diaswarm_keys::Vault::key_bundle(&rng).unwrap();
        keys.create(mgr).unwrap();

        let (named, older) = invites_for(&core, &keys, &signing.verifying_key().to_hex()).unwrap();

        assert!(
            older.starts_with("diaswarm:3:"),
            "the fallback is not v3 — an installed follower would refuse it: {}",
            &older[..older.len().min(24)]
        );
        // The name is the only thing that differs, and it is the thing that
        // says these readings are not a person's.
        assert_eq!(
            diaswarm_core::invite::Invite::parse(&named).unwrap().handle,
            HANDLE
        );
        assert_eq!(diaswarm_core::invite::Invite::parse(&older).unwrap().handle, "");
        let (a, b) = (
            diaswarm_core::invite::Invite::parse(&named).unwrap(),
            diaswarm_core::invite::Invite::parse(&older).unwrap(),
        );
        assert_eq!(a.subject, b.subject, "the two invites are for different subjects");
        assert_eq!(a.keys, b.keys, "the two invites carry different keys identities");
    }

    /// THE WHOLE CLAIM OF THIS COMMAND, END TO END: SOMEBODY PASTES THEIR
    /// INVITE AND CAN THEN READ.
    ///
    /// **DRIVEN BY AN INVITE STRING, BECAUSE THAT IS WHAT A REVIEWER HAS.**
    /// The argument to `--grant` is composed here exactly as Ayni's "Show my
    /// invite" composes it — subject key, endpoint, purpose, keys identity — so
    /// a change that makes the two disagree fails here rather than on somebody
    /// else's phone. This is the desktop half of the v2/v3 trap that a single
    /// pair of encode/decode functions agreeing with itself could not catch.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_follower_that_pastes_its_invite_can_read_the_demo() {
        let dir = tmp("grant");
        let store = diaswarm_keys::open_bounded_store(&format!(
            "sqlite://{}",
            dir.join("keys.sqlite").display()
        ))
        .await
        .unwrap();
        let rng = diaswarm_keys::Rng::default();

        // ---- the demo subject, both vaults ----
        let identity = Identity::generate();
        let subject_hex = diaswarm_core::vault::hex(&identity.enc_public());
        let signing = p2panda_core::SigningKey::generate();
        let core = diaswarm_core::vault::Vault::create(&core_dir(&dir, &subject_hex), &identity, 0)
            .expect("core");
        let mut keys =
            diaswarm_keys::Vault::open(&keys_dir(&dir), 0, &signing).expect("keys vault");
        let (mgr, _) = diaswarm_keys::Vault::key_bundle(&rng).unwrap();
        let create = keys.create(mgr).unwrap();
        diaswarm_keys::wire::publish_control(&store, &signing, &create).await.unwrap();

        // ---- a follower, with its own two halves, showing its invite ----
        let reader_core = Identity::generate();
        let reader_signing = p2panda_core::SigningKey::generate();
        let reader_dir = tmp("reader");
        let mut reader_keys =
            diaswarm_keys::Vault::open(&reader_dir.join("own"), 0, &reader_signing).unwrap();
        let (rmgr, _) = diaswarm_keys::Vault::key_bundle(&rng).unwrap();
        let rcreate = reader_keys.create(rmgr).unwrap();
        diaswarm_keys::wire::publish_control(&store, &reader_signing, &rcreate).await.unwrap();
        let reader_identity_hex =
            diaswarm_keys::encode_identity(&reader_keys.identity().unwrap()).unwrap();
        let reader_invite = diaswarm_core::invite::Invite::new(
            &diaswarm_core::vault::hex(&reader_core.enc_public()),
            &reader_signing.verifying_key().to_hex(),
            "follow",
        )
        .unwrap()
        .with_keys(&reader_identity_hex)
        .unwrap()
        .encode();

        // ---- the grant, from that string and nothing else ----
        grant_reader(&core, &identity, &mut keys, &store, &signing, None, &reader_invite)
            .await
            .expect("granting from an invite");

        // ---- and a window sealed after it ----
        let now = 1_789_000_000_000i64;
        let n = publish_window(&core, &mut keys, &store, &signing, None, now, 30, 0).await.unwrap();
        assert_eq!(n, 30);

        // ---- the claim: the follower joins by the demo's invite and reads ----
        let demo_invite = invite_for(&core, &keys, &signing.verifying_key().to_hex(), Some(HANDLE)).expect("invite");
        let parsed = diaswarm_core::invite::Invite::parse(&demo_invite).unwrap();
        assert!(
            parsed.handle.contains("not a real person"),
            "the demo's invite does not say it is a demo: {:?}",
            parsed.handle
        );

        let (joined, subject_key) = diaswarm_keys::follow::join(
            &reader_dir.join("own"),
            &reader_dir.join("joined"),
            &store,
            &reader_signing,
            &parsed.keys,
            "follow",
            0,
        )
        .await
        .expect("the follower could not join");
        let (records, opened, skipped) =
            diaswarm_keys::follow::read(&joined, &store, &subject_key, 0, i64::MIN)
                .await
                .expect("read");
        assert_eq!(skipped.lost(), 0, "a freshly granted follower hit a fault");
        assert!(opened >= 1, "the follower opened nothing");
        assert_eq!(records.len(), 30, "expected 30 invented readings, got {}", records.len());
        // **AND THEY ARE LABELLED.** Nothing downstream — a report, an export,
        // a clinician's screen — may mistake these for a sensor's output.
        assert!(
            records.iter().all(|r| r.get("src").and_then(|v| v.as_str()) == Some("DEMO")),
            "an invented reading arrived without its DEMO label"
        );
    }

    /// A DEMO THAT LEAVES PHYSIOLOGICAL RANGE LOOKS BROKEN.
    #[test]
    fn the_curve_stays_where_glucose_lives() {
        let start = 1_789_000_000_000i64;
        let vals: Vec<f64> = (0..(60 * 24 * 3)).map(|i| reading_at(start + i * 60_000)).collect();
        let lo = vals.iter().cloned().fold(f64::MAX, f64::min);
        let hi = vals.iter().cloned().fold(f64::MIN, f64::max);
        assert!(lo >= 55.0, "a demo reading went to {lo} mg/dL");
        assert!(hi <= 320.0, "a demo reading went to {hi} mg/dL");
        // And it must actually move, or it is a flat line nobody learns from.
        assert!(hi - lo > 60.0, "the curve only spans {:.0} mg/dL", hi - lo);
    }

    /// DETERMINISTIC, SO A RESTART CONTINUES RATHER THAN JUMPS.
    #[test]
    fn the_same_moment_always_gives_the_same_reading() {
        let t = 1_789_123_456_000i64;
        assert_eq!(reading_at(t), reading_at(t));
        assert_ne!(reading_at(t), reading_at(t + 600_000), "the curve is flat");
    }

    /// ONE A MINUTE, NEWEST LAST, ALL IN THE PAST.
    #[test]
    fn a_window_is_minute_by_minute_and_ends_now() {
        let now = 1_789_123_456_000i64;
        let r = readings_for(now, 30);
        assert_eq!(r.len(), 30);
        assert_eq!(r.last().unwrap().t(), now);
        assert_eq!(r[1].t() - r[0].t(), 60_000, "not a one-minute cadence");
        assert!(r.iter().all(|x| x.t() <= now), "a demo reading is in the future");
        assert!(r.iter().all(|x| x.kind() == "cgm"));
        // Labelled, so nothing downstream mistakes it for a sensor.
        assert_eq!(r[0].get("src").and_then(|v| v.as_str()), Some("DEMO"));
    }
}
