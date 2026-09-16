//! An always-on pool member, for a machine that does not sleep.
//!
//! **THE POOL'S WEAKNESS IS THAT EVERY HOLDER IS A PHONE.**
//! [D15](../../../docs/decisions.md) promises that a subject whose phone is
//! asleep stays readable, because somebody else holds the same bytes. Every
//! somebody else, so far, is an Android device subject to doze — a condition
//! measured at two hours and counting on 2026-09-14, and one no foreground
//! service turns into "always". One mains-powered peer with a disk makes that
//! promise structural rather than probabilistic.
//!
//! **IT CANNOT READ A BYTE OF WHAT IT HOLDS.** Segments are sealed and this
//! peer is granted nothing; there is no configuration that would make it a
//! server. What it carries is other people's ciphertext, and the deal runs both
//! ways — see the trade table in the README.
//!
//! This is the carrier half of [D29](../../../docs/decisions.md). Reading what
//! *you* were granted, and exporting it, is this binary's next job and needs no
//! new grant semantics; the research gateway needs time-scoped grants and does
//! not exist.

mod agp;
mod fhir;
mod nightscout;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::Swarm;
use p2panda_core::{Hash, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

/// How often a pass runs.
///
/// **NOT A FRESHNESS KNOB.** Operations arrive by live push between passes
/// (D21), so this is only how often the *share* is recomputed — how quickly
/// this peer notices a subject it ought to be carrying. Freshness is measured
/// elsewhere and is dominated by the sensor, not by this.
const PASS: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(
    name = "diaswarm-peer",
    version,
    about = "Hold a share of the diaswarm pool, so somebody's phone can sleep",
    long_about = None,
)]
struct Args {
    /// Where the node key, follow list and keys.sqlite live.
    ///
    /// Defaults to $XDG_DATA_HOME/diaswarm (or ~/.local/share/diaswarm).
    #[arg(value_name = "DIR")]
    dir: Option<PathBuf>,

    /// How many new strangers to take on per pass.
    ///
    /// `0` carries only what this peer reads, and no strangers.
    ///
    /// It does NOT make this peer a freeloader — reciprocity is unconditional
    /// (D30: if you read it, you carry it), and subjects you follow are always
    /// carried. What `0` switches off is generosity.
    ///
    /// Think twice before using it. Not for secrecy of the data — that is
    /// encryption's job, and a carrier can open none of what it holds — but
    /// because carrying for strangers is what keeps "P holds Y" ambiguous
    /// between following Y and merely holding it. A pool where everyone passed
    /// `0` would publish who reads whom. `0` is for a research gateway, which
    /// must not hold history it was never granted (D5).
    #[arg(long, default_value_t = 4, value_name = "N")]
    adopt: usize,

    /// Join a named pool instead of the default, without a relay. For testing.
    #[arg(long, value_name = "NAME")]
    network: Option<String>,

    /// Carry this subject, whatever gossip does or does not say. Repeatable.
    ///
    /// **THE ONE WAY TO BE SURE.** Everything else this peer carries it has to
    /// be *told about*: a follow comes from a pairing, a stranger from an
    /// announcement that has to reach it over an overlay whose membership is
    /// sampled and can simply not include the peer holding the data. A carrier
    /// somebody runs for their own family already knows whose records it means;
    /// naming them here removes discovery from the path entirely.
    ///
    /// Take the value from the phone's log line `swarm: keys subject <hex>`.
    #[arg(long = "carry", value_name = "SUBJECT-HEX")]
    carry: Vec<String>,

    /// Do one pass and exit.
    #[arg(long)]
    once: bool,

    /// One JSON object per pass instead of a human-readable line.
    #[arg(long)]
    json: bool,

    /// What to do instead of carrying. Omit to run the carrier loop.
    #[command(subcommand)]
    cmd: Option<Cmd>,

    /// Print the last N sync events each pass.
    ///
    /// **THE DIAGNOSTIC THAT TOOK A DAY TO BUILD FOR THE PHONES AND WAS NOT
    /// HERE.** "Nothing replicated" has several very different causes — no peer
    /// found, a session that started and failed, a session that finished having
    /// transferred nothing, live mode never starting — and they are
    /// indistinguishable from a count that does not move. This peer stalled at
    /// 1,716 operations while the publisher sealed every minute, and none of
    /// the numbers printed said which of those it was.
    #[arg(long, default_value_t = 0, value_name = "N")]
    events: usize,
}


/// The things this peer does that are not "hold a share and wait".
///
/// **THE DEFAULT IS STILL THE CARRIER LOOP**, with no subcommand, because that
/// is what is already running under systemd and in people's shell history.
/// Adding a subcommand must not change what `diaswarm-peer --carry X` does.
///
/// This is D29 step 1: the desktop stops being only a carrier and becomes a
/// reader for yourself and your family. It needs no new grant semantics — the
/// reading rule is `diaswarm_keys::follow`, the same one the phones use.
#[derive(clap::Subcommand, Debug)]
enum Cmd {
    /// Print the identity a subject grants, to paste into "Share with someone".
    ///
    /// Creates this peer's encryption identity on first use and publishes it to
    /// the local control log, which replicates the next time the carrier loop
    /// runs. **Nothing is readable until a subject grants this string.**
    Identity,

    /// Read what a subject has granted this peer.
    ///
    /// Reads what has already replicated into `keys.sqlite`; it does not go on
    /// the network, so run the carrier loop if nothing is arriving.
    Read {
        /// The subject's identity, from their invite — not their 64-hex key.
        #[arg(long, value_name = "IDENTITY-HEX")]
        subject: String,
        /// Only the newest N days. 0 reads everything, which is linear in the
        /// subject's whole history.
        #[arg(long, default_value_t = 1, value_name = "N")]
        days: u64,
        /// One JSON object per record instead of a summary.
        #[arg(long)]
        json: bool,
    },

    /// A clinical summary of what a subject granted, as a FHIR bundle.
    ///
    /// **THIS IS THE CLINICIAN'S VIEW, AND `export` IS NOT.** A clinic does not
    /// read 143,000 glucose values; it reads time in range, mean, variability
    /// and how much of the window the sensor was working. FHIR because that is
    /// what an institution can ingest — feasibility §10.7.
    Summary {
        /// The subject's identity, from their invite — not their 64-hex key.
        #[arg(long, value_name = "IDENTITY-HEX")]
        subject: String,
        /// The window to summarise. 0 summarises everything readable.
        #[arg(long, default_value_t = 90, value_name = "N")]
        days: u64,
        /// The Patient this is about, for `Observation.subject`.
        ///
        /// **REQUIRED, AND NOT INVENTED HERE.** A gateway that minted patient
        /// identifiers would be the identity broker D5 forbids; the receiving
        /// institution's identifier is the caller's to supply.
        #[arg(long, value_name = "PATIENT-ID")]
        patient: String,
        /// Where to write. `-` writes to stdout.
        #[arg(long, default_value = "-", value_name = "FILE")]
        out: PathBuf,
        /// How to wrap it.
        ///
        /// `document` is a self-contained clinical document — what a person
        /// hands over, and the default. `transaction` is the IG's submission
        /// bundle: **a queue of HTTP POSTs** for somebody who has actually
        /// arranged to submit to a server. This tool does not submit it.
        #[arg(long = "as", default_value = "document", value_name = "ENVELOPE")]
        envelope: String,
    },

    /// Nightscout `entries.json` and `treatments.json`, for the ecosystem.
    ///
    /// **THE CHEAPEST BRIDGE TO EVERYTHING THAT ALREADY EXISTS** — follower
    /// apps, watch faces, clinic dashboards. feasibility §10.7.
    Nightscout {
        /// The subject's identity, from their invite — not their 64-hex key.
        #[arg(long, value_name = "IDENTITY-HEX")]
        subject: String,
        /// Only the newest N days. 0 exports everything readable.
        #[arg(long, default_value_t = 0, value_name = "N")]
        days: u64,
        /// Directory to write `entries.json` and `treatments.json` into.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },

    /// Write what a subject has granted this peer to a CSV file, one row per
    /// record. The researcher's shape, not the clinician's.
    Export {
        /// The subject's identity, from their invite — not their 64-hex key.
        #[arg(long, value_name = "IDENTITY-HEX")]
        subject: String,
        /// Where to write. `-` writes to stdout.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Only the newest N days. 0 exports everything.
        #[arg(long, default_value_t = 0, value_name = "N")]
        days: u64,
    },
}

/// Operations actually in the store for the subjects this peer carries.
///
/// **NOT `Replicator::received()`, AND THE DIFFERENCE IS THE POINT.** That
/// counter increments once per *store event*, so an operation arriving on two
/// topics — which is exactly what the D31 transition does while it carries both
/// the subject topic and the old bucket topic — counts twice. On this machine
/// it read 3,400 against 1,729 actually held. A count that is roughly double
/// the truth looks like health, which is the failure mode this project has been
/// bitten by four times. Ask the store.
async fn held(store: &SqliteStore, subjects: &[String]) -> u64 {
    let mut total = 0u64;
    for hex in subjects {
        let Some(key) = decode_key(hex) else { continue };
        for log_id in diaswarm_keys::wire::LOG_IDS {
            let size = <SqliteStore as LogStore<
                diaswarm_keys::wire::KeysOperation,
                VerifyingKey,
                u32,
                u32,
                Hash,
            >>::get_log_size(store, &key, &log_id, None, None)
            .await
            .unwrap_or(None);
            total += size.map(|(ops, _bytes)| ops as u64).unwrap_or(0);
        }
    }
    total
}

/// A 64-character hex subject as a key.
fn decode_key(s: &str) -> Option<VerifyingKey> {
    if s.len() != 64 {
        return None;
    }
    let mut b = [0u8; 32];
    for (i, out) in b.iter_mut().enumerate() {
        *out = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    VerifyingKey::from_bytes(&b).ok()
}

/// Where a peer keeps its state when nobody said.
fn default_dir() -> Result<PathBuf> {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x).join("diaswarm"));
        }
    }
    let home = std::env::var("HOME").context("no HOME and no XDG_DATA_HOME; pass a directory")?;
    Ok(PathBuf::from(home).join(".local/share/diaswarm"))
}

/// Bytes on disk under a directory, for saying what this actually costs.
fn disk_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => disk_bytes(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn human(bytes: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", U[i]) }
}


/// Where this peer's encryption identity lives.
///
/// **SEPARATE FROM `node.key`, AND DELIBERATELY.** The node key is how the
/// network ranks and finds this peer; the keys identity is what subjects grant
/// against. D13 keeps a grant log that names nobody, and reusing one key for
/// both would tie "who carries" to "who reads" in exactly the way the design
/// spends effort avoiding.
fn keys_identity_key(dir: &Path) -> Result<p2panda_core::SigningKey> {
    let path = dir.join("keys-identity.key");
    match std::fs::read(&path) {
        Ok(b) if b.len() == 32 => {
            Ok(p2panda_core::SigningKey::from_bytes(&b.try_into().expect("checked length")))
        }
        _ => {
            let k = p2panda_core::SigningKey::generate();
            std::fs::write(&path, k.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            Ok(k)
        }
    }
}

/// **UTC, AND IT BARELY MATTERS HERE.** The offset decides how a vault buckets
/// its *own* sealing into days. A reader opens segments the subject already
/// stamped, so this only affects how a local export would group them, and a
/// carrier has no business asserting the subject's timezone.
const PEER_OFFSET_MS: i64 = 0;

fn own_vault_dir(dir: &Path) -> PathBuf {
    dir.join("keys-vault")
}

/// Create this peer's encryption identity if it has none, and print it.
async fn cmd_identity(dir: &Path) -> Result<()> {
    let signing = keys_identity_key(dir)?;
    let own = own_vault_dir(dir);
    std::fs::create_dir_all(&own).with_context(|| format!("creating {}", own.display()))?;
    let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
    let store = diaswarm_keys::open_bounded_store(&url)
        .await
        .map_err(|e| anyhow::anyhow!("opening {url}: {e}"))?;

    let mut vault = diaswarm_keys::Vault::open(&own, PEER_OFFSET_MS, &signing)
        .map_err(|e| anyhow::anyhow!("own vault: {e}"))?;

    // No group on disk means this is the first open. Creating one generates the
    // encryption identity and writes it out; a vault that came back without it
    // would be a new member and every grant to it would be dead.
    if !vault.is_welcomed() {
        let rng = diaswarm_keys::Rng::default();
        let (manager, _bundle) = diaswarm_keys::Vault::key_bundle(&rng)
            .map_err(|e| anyhow::anyhow!("generating an identity: {e}"))?;
        let create = vault.create(manager).map_err(|e| anyhow::anyhow!("create: {e}"))?;
        // **PUBLISHED, NOT DISCARDED.** The control log is D13's grant log, and
        // one that starts at the first grant cannot show that nothing came
        // before it. It leaves here on the next carrier pass.
        diaswarm_keys::wire::publish_control(&store, &signing, &create)
            .await
            .map_err(|e| anyhow::anyhow!("publishing the identity: {e}"))?;
        eprintln!("created this peer's encryption identity in {}", own.display());
    }

    let identity = vault.identity().map_err(|e| anyhow::anyhow!("identity: {e}"))?;
    let text = diaswarm_keys::encode_identity(&identity)
        .map_err(|e| anyhow::anyhow!("encoding: {e}"))?;
    println!("{text}");
    eprintln!();
    eprintln!("Paste that into the subject's Swarm sharing → \"Share with someone\".");
    eprintln!("Nothing is readable until they do, and it replicates on the next carrier pass.");
    Ok(())
}

/// What a read came back with, including what it could not open and why.
struct Granted {
    records: Vec<diaswarm_core::Record>,
    opened: usize,
    skipped: diaswarm_keys::Skipped,
    /// The oldest moment this peer can open anything, in UNIX seconds — read
    /// off the secrets it holds, not off anything it was told. See
    /// `Vault::granted_from`.
    granted_from: Option<u64>,
}

/// Join if needed and read. Shared by `read` and `export` so they cannot differ.
async fn granted_records(
    dir: &Path,
    subject_identity: &str,
    days: u64,
) -> Result<Granted> {
    let signing = keys_identity_key(dir)?;
    let own = own_vault_dir(dir);
    if !own.join("group.cbor").exists() {
        anyhow::bail!(
            "this peer has no encryption identity yet — run `diaswarm-peer identity` first"
        );
    }
    let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
    let store = diaswarm_keys::open_bounded_store(&url)
        .await
        .map_err(|e| anyhow::anyhow!("opening {url}: {e}"))?;

    let decoded = diaswarm_keys::decode_identity(subject_identity)
        .map_err(|e| anyhow::anyhow!("that is not a subject identity: {e}"))?;
    let joined = dir.join("joined").join(decoded.signer.to_hex());
    std::fs::create_dir_all(&joined)?;

    // **JOIN ONCE.** `join` replaces the group state, so doing it on every read
    // would throw away a secret bundle that took a replication round trip to
    // get. An already-welcomed vault goes straight to reading.
    let vault = match diaswarm_keys::Vault::open(&joined, PEER_OFFSET_MS, &signing) {
        Ok(v) if v.is_welcomed() => v,
        _ => {
            let (v, _subject) = diaswarm_keys::follow::join(
                &own, &joined, &store, &signing, subject_identity, "follow", PEER_OFFSET_MS,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            v
        }
    };

    let granted_from = vault.granted_from();
    let (records, opened, skipped) =
        diaswarm_keys::follow::read(&vault, &store, &decoded.signer, days, i64::MIN)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Granted { records, opened, skipped, granted_from })
}

/// **THE WINDOW THIS PEER ACTUALLY HOLDS, SAID PLAINLY.**
///
/// D29 exists to stop a screen offering a window the grant does not enforce.
/// This prints the one derived from the key material, so it cannot overstate
/// what can be read — and says so when nothing bounds it.
fn describe_window(granted_from: Option<u64>) -> String {
    let Some(from) = granted_from else {
        return "no secrets held — nothing is readable".to_string();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(from);
    let days = now.saturating_sub(from) / 86_400;
    format!("granted from unix {from} — about {days} day(s) of history")
}

/// What could not be opened, in words that do not overstate the cause.
///
/// ⚠️ **"OUTSIDE YOUR WINDOW" IS ONLY SAID FOR `not_ours`.** Everything else is
/// a fault, and calling a fault access control is how a reader gets told its
/// data is fine when it is not — which is exactly what an unannounced rotation
/// looked like on 2026-09-16.
fn describe_skipped(opened: usize, s: &diaswarm_keys::Skipped) -> String {
    let mut parts = vec![format!("{opened} segment(s) opened")];
    if s.not_ours > 0 {
        parts.push(format!("{} outside this peer's window (unreadable, not hidden)", s.not_ours));
    }
    if s.lost() > 0 {
        parts.push(format!(
            "⚠️ {} FAILED to open despite holding the secret — this is a fault, not a window",
            s.lost()
        ));
    }
    parts.join("; ")
}

async fn cmd_read(dir: &Path, subject: &str, days: u64, json: bool) -> Result<()> {
    let g = granted_records(dir, subject, days).await?;
    let (records, opened) = (&g.records, g.opened);
    if json {
        for r in records {
            println!("{}", r.to_canonical_json());
        }
    } else {
        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
        for r in records {
            *kinds.entry(r.kind()).or_default() += 1;
        }
        let span = match (records.first(), records.last()) {
            (Some(a), Some(b)) => format!("{} … {}", a.t(), b.t()),
            _ => "nothing".to_string(),
        };
        println!("window: {}", describe_window(g.granted_from));
        println!("{} record(s) over {span}", records.len());
        for (k, n) in kinds {
            println!("  {k:<16} {n}");
        }
        // **SAY WHAT COULD NOT BE OPENED, AND WHY.** A short answer must never
        // be a silent one, and it must not blame access control for a fault.
        println!("{}", describe_skipped(opened, &g.skipped));
    }
    Ok(())
}

async fn cmd_export(dir: &Path, subject: &str, out: &Path, days: u64) -> Result<()> {
    let g = granted_records(dir, subject, days).await?;
    let records = &g.records;

    // **THE UNION OF WHAT ARRIVED, NOT A FIXED LIST.** A record is open by
    // design, so a fixed header silently drops whatever a device started
    // reporting last week — see `Record::fields`.
    let mut columns: std::collections::BTreeSet<&str> = Default::default();
    for r in records {
        for (k, _) in r.fields() {
            columns.insert(k);
        }
    }
    // `t` and `k` first; they are the two every record has.
    let mut header: Vec<&str> = vec!["t", "k"];
    header.extend(columns.iter().copied().filter(|c| *c != "t" && *c != "k"));

    let mut csv = String::new();
    csv.push_str(&header.join(","));
    csv.push('\n');
    for r in records {
        let row: Vec<String> = header
            .iter()
            .map(|c| match r.get(c) {
                Some(v) => {
                    let text = v.to_string();
                    let text = text.trim_matches('"').to_string();
                    if text.contains(',') || text.contains('"') {
                        format!("\"{}\"", text.replace('"', "\"\""))
                    } else {
                        text
                    }
                }
                None => String::new(),
            })
            .collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }

    if out == Path::new("-") {
        print!("{csv}");
    } else {
        std::fs::write(out, csv.as_bytes())
            .with_context(|| format!("writing {}", out.display()))?;
        eprintln!(
            "{} record(s), {} column(s) → {}",
            records.len(),
            header.len(),
            out.display()
        );
        eprintln!("window: {}", describe_window(g.granted_from));
        eprintln!("{}", describe_skipped(g.opened, &g.skipped));
    }
    Ok(())
}

async fn cmd_summary(
    dir: &Path,
    subject: &str,
    days: u64,
    patient: &str,
    out: &Path,
    envelope: fhir::Envelope,
) -> Result<()> {
    let g = granted_records(dir, subject, days).await?;
    let Some(summary) = agp::Agp::from_records(&g.records) else {
        anyhow::bail!(
            "no CGM readings in what this peer can open — {}",
            describe_skipped(g.opened, &g.skipped)
        );
    };
    let bundle = fhir::bundle(&summary, patient, envelope);

    if out == Path::new("-") {
        println!("{bundle}");
    } else {
        std::fs::write(out, bundle.as_bytes())
            .with_context(|| format!("writing {}", out.display()))?;
        eprintln!("FHIR CGM summary → {}", out.display());
    }

    // **EVERYTHING BELOW GOES TO STDERR**, so `--out -` stays a clean bundle a
    // pipe can hand to a validator.
    eprintln!("window: {}", describe_window(g.granted_from));
    eprintln!("{}", describe_skipped(g.opened, &g.skipped));
    eprintln!(
        "{} reading(s) at a {}s cadence · in range {:.1}% · mean {:.0} mg/dL · GMI {:.1}% · CV {:.1}%",
        summary.readings,
        summary.cadence_seconds,
        summary.in_range_percent,
        summary.mean_mgdl,
        summary.gmi_percent,
        summary.cv_percent,
    );
    // **SAY WHEN IT IS NOT ENOUGH TO ACT ON.** The consensus asks for 14 days at
    // 70% active; a report that quietly summarises four days looks exactly like
    // one that summarises ninety.
    match summary.insufficiency() {
        Some(why) => eprintln!("⚠️ NOT a reliable estimate: {why}"),
        None => eprintln!("meets the consensus minimum (14 days, 70% active)"),
    }
    Ok(())
}

async fn cmd_nightscout(dir: &Path, subject: &str, days: u64, out: &Path) -> Result<()> {
    let g = granted_records(dir, subject, days).await?;
    let entries = nightscout::entries(&g.records);
    let treatments = nightscout::treatments(&g.records);

    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let e_path = out.join("entries.json");
    let t_path = out.join("treatments.json");
    std::fs::write(&e_path, nightscout::as_array(&entries).as_bytes())
        .with_context(|| format!("writing {}", e_path.display()))?;
    std::fs::write(&t_path, nightscout::as_array(&treatments).as_bytes())
        .with_context(|| format!("writing {}", t_path.display()))?;

    eprintln!("{} entry(ies) → {}", entries.len(), e_path.display());
    eprintln!("{} treatment(s) → {}", treatments.len(), t_path.display());
    // **SAY WHAT WAS DROPPED.** Records whose kind has no Nightscout event type
    // are left out rather than guessed at, and a count that does not add up is
    // the only way a reader would notice.
    let carried = g.records.len();
    let priming = nightscout::priming_excluded(&g.records);
    if priming > 0 {
        // **NOT A LOSS, AND NOT SILENT EITHER.** A priming bolus never reaches
        // the patient — AAPS excludes it from IOB and TDD for the same reason —
        // and exporting it as `insulin` would inflate every total downstream.
        eprintln!("{priming} priming bolus(es) excluded — they never reached the patient");
    }
    let dropped = carried - entries.len() - treatments.len() - priming;
    if dropped > 0 {
        eprintln!(
            "{dropped} record(s) had no Nightscout shape and were left out (stream headers, and event types outside its vocabulary)"
        );
    }
    eprintln!("window: {}", describe_window(g.granted_from));
    eprintln!("{}", describe_skipped(g.opened, &g.skipped));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let dir = match args.dir {
        Some(d) => d,
        None => default_dir()?,
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    // **THE SUBCOMMANDS RUN WITHOUT TOUCHING THE NETWORK.** They read what has
    // already replicated into `keys.sqlite`. That keeps them fast, keeps them
    // safe to run while the carrier loop is up in another process, and makes
    // "nothing arrived" a separate problem from "I cannot open it".
    match args.cmd {
        Some(Cmd::Identity) => return cmd_identity(&dir).await,
        Some(Cmd::Read { subject, days, json }) => {
            return cmd_read(&dir, &subject, days, json).await;
        }
        Some(Cmd::Export { subject, out, days }) => {
            return cmd_export(&dir, &subject, &out, days).await;
        }
        Some(Cmd::Summary { subject, days, patient, out, envelope }) => {
            let envelope = match envelope.as_str() {
                "document" => fhir::Envelope::Document,
                "transaction" => fhir::Envelope::Transaction,
                other => anyhow::bail!("--as must be document or transaction, not {other}"),
            };
            return cmd_summary(&dir, &subject, days, &patient, &out, envelope).await;
        }
        Some(Cmd::Nightscout { subject, days, out }) => {
            return cmd_nightscout(&dir, &subject, days, &out).await;
        }
        None => {}
    }

    // The same node key file a phone uses, so a peer keeps the identity it had
    // across restarts. Everything that ranks peers ranks them by it, and a new
    // key reads as one peer leaving and another arriving.
    let key_path = dir.join("node.key");
    let signing = match std::fs::read(&key_path) {
        Ok(b) if b.len() == 32 => {
            p2panda_core::SigningKey::from_bytes(&b.try_into().expect("checked length"))
        }
        _ => {
            let k = p2panda_core::SigningKey::generate();
            std::fs::write(&key_path, k.as_bytes())
                .with_context(|| format!("writing {}", key_path.display()))?;
            k
        }
    };
    let own = signing.verifying_key().to_hex();

    // **THROUGH THE RELAY UNLESS TOLD OTHERWISE.** A carrier that can only be
    // found on one wifi is not a carrier. `--network` exists for tests, which
    // must not dial somebody else's relay.
    let swarm = match &args.network {
        Some(name) => {
            Swarm::join_network(dir.clone(), signing, diaswarm_net::swarm::network_id(name)).await?
        }
        None => Swarm::join(dir.clone(), signing).await?,
    };

    let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
    // A desktop has memory to spare, but one behaviour to reason about is
    // worth more — and a carrier left running for weeks is exactly where an
    // uncapped page cache would show up next.
    let store = diaswarm_keys::open_bounded_store(&url)
        .await
        .map_err(|e| anyhow::anyhow!("opening {url}: {e}"))?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;

    let node = swarm.node_id().await?;
    if args.json {
        println!(r#"{{"event":"up","subject":"{own}","node":"{node}","dir":{:?}}}"#, dir.display().to_string());
    } else {
        println!("diaswarm-peer {} — {}", env!("CARGO_PKG_VERSION"), dir.display());
        println!("  node {node}");
        println!("  adopting up to {} new subject(s) a pass; it can read none of them", args.adopt);
    }

    // A stall is only visible against what was held last time.
    let mut was_held = 0u64;
    let mut stalled_for = 0u32;

    loop {
        let tick = swarm.tick().await;
        let share =
            diaswarm_net::share::carry_share(&swarm, &replicator, &dir, &own, args.adopt, &args.carry)
                .await;
        let bytes = disk_bytes(&dir);
        let stored = held(&store, &replicator.carried()).await;
        // **HOW MANY SUBJECTS IT HAS HEARD OF BUT IS NOT YET CARRYING.**
        // Without this, "carrying 1" is ambiguous between "nobody has told me
        // about anybody" and "I was told and did not act", which are different
        // bugs in different files. Costs one in-memory read.
        let heard_waiting = swarm.keys_wanted().await.map(|w| w.len()).unwrap_or(0);
        if stored > was_held {
            stalled_for = 0;
        } else if stored > 0 {
            stalled_for += 1;
        }
        was_held = stored;
        match (tick, share) {
            (Ok(t), Ok(s)) => {
                let pushed = replicator.live_received() > 0;
                if args.json {
                    println!(
                        r#"{{"pool":{},"buckets":{},"carrying":{},"adopted":{},"held":{},"store_events":{},"pushed":{},"bytes":{},"stalled_passes":{}}}"#,
                        t.pool, t.buckets, s.carrying, s.adopted, stored,
                        replicator.received(), pushed, bytes, stalled_for
                    );
                } else {
                    println!(
                        "pool {} · heard {} · carrying {} (+{}) · holding {} · pushes {} · {}{}",
                        t.pool,
                        heard_waiting,
                        s.carrying,
                        s.adopted,
                        stored,
                        if pushed { "yes" } else { "not yet" },
                        human(bytes),
                        if stalled_for >= 3 {
                            format!(" · STALLED {stalled_for} passes — nothing new is arriving")
                        } else {
                            String::new()
                        },
                    );
                }
                // **RE-SUBSCRIBE WHEN IT HAS CLEARLY STOPPED**, which is the
                // remedy Ayni has had since the phones hit this exact symptom:
                // catch-up delivers once on connect and live mode then produces
                // nothing for ever, `received_live_operations: 0` on every
                // session while `LiveModeStarted` keeps firing. Dropping the
                // handles and streaming the topics again is what brings it
                // back there, and the peer simply never had it.
                //
                // Three passes, not one: a pass with nothing new is ordinary
                // when the publisher has nothing to say, and re-subscribing on
                // every quiet minute would be its own kind of broken.
                if stalled_for >= 3 && stalled_for % 3 == 0 {
                    match replicator.restream().await {
                        Ok(n) => println!("    stalled — re-subscribed {n} topic(s)"),
                        Err(e) => println!("    stalled — re-subscribe failed: {e:#}"),
                    }
                }
                // The events, when asked for, or unprompted once a stall is
                // undeniable — because that is the moment somebody needs them.
                if args.events > 0 || stalled_for == 3 {
                    let all = replicator.events();
                    let n = if args.events > 0 { args.events } else { 8 };
                    for line in all.iter().rev().take(n).rev() {
                        println!("    sync: {line}");
                    }
                    if all.is_empty() {
                        println!("    sync: no session events recorded at all");
                    }
                }
            }
            // **SAY WHICH HALF FAILED.** A pass that could not reach the pool
            // and a pass that reached it and carried nothing look identical in
            // a count, and this project has already lost a night to that.
            (Err(e), _) => eprintln!("pool pass failed: {e:#}"),
            (Ok(t), Err(e)) => eprintln!("pool {} reached, but carrying failed: {e:#}", t.pool),
        }
        if args.once {
            return Ok(());
        }
        // **SHUT DOWN WHEN ASKED.** A daemon killed mid-write leaves a SQLite
        // file somebody has to reason about; ^C and `systemctl stop` should
        // both end a pass cleanly rather than in the middle of one.
        tokio::select! {
            _ = tokio::time::sleep(PASS) => {}
            _ = tokio::signal::ctrl_c() => {
                if !args.json {
                    println!("stopping — {} held, {} on disk", replicator.carried().len(), human(bytes));
                }
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diaswarm_core::{EPOCH_MS, Record};
    use diaswarm_keys::{Vault, wire};

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "diaswarm-peertest-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// THE WHOLE POINT OF D29 STEP 1: A DESKTOP READS WHAT IT WAS GRANTED.
    ///
    /// **THIS IS THE PATH `cargo check` CANNOT SEE.** The reading rule is
    /// tested in `diaswarm-keys`; what is only tested here is this binary's
    /// wiring — that the identity it prints is the one a grant lands on, that
    /// the joined vault goes in its own directory, and that an export names the
    /// columns that actually arrived rather than a fixed list.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_reads_and_exports_what_a_subject_granted_it() {
        let dir = tmp("read");
        let rng = diaswarm_keys::Rng::default();

        // ---- the peer publishes an identity, exactly as `identity` does ----
        cmd_identity(&dir).await.expect("identity");
        let peer_signing = keys_identity_key(&dir).unwrap();
        let peer_vault = Vault::open(&own_vault_dir(&dir), PEER_OFFSET_MS, &peer_signing).unwrap();
        let peer_identity = peer_vault.identity().expect("peer identity");
        drop(peer_vault);

        // ---- a subject, sealing a day ----
        let subject_key = p2panda_core::SigningKey::generate();
        let subject_dir = tmp("subject");
        let mut subject = Vault::open(&subject_dir, PEER_OFFSET_MS, &subject_key).unwrap();
        let (mgr, subject_bundle) = Vault::key_bundle(&rng).unwrap();
        let create = subject.create(mgr).unwrap();

        // The peer's store is what replication would have filled.
        let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
        let store = diaswarm_keys::open_bounded_store(&url).await.unwrap();
        wire::publish_control(&store, &subject_key, &create).await.unwrap();

        let epoch = 20_000i64;
        // A realistic minute-cadence hour, so the summary has something to be a
        // summary of, plus one treatment to prove kinds stay separate.
        let mut records: Vec<Record> = (0..60)
            .map(|i| {
                let mgdl = 100.0 + (i as f64 * 3.0) % 120.0;
                Record::new(epoch * EPOCH_MS + i * 60_000, "cgm").set("mgdl", Some(mgdl.into()))
            })
            .collect();
        // **`u`, NOT `units` — spec/records.md §2.** The first version of this
        // fixture invented a field name, and the Nightscout exporter correctly
        // dropped the record for not having the one the spec defines. A fixture
        // that does not match the wire tests nothing.
        records.push(
            Record::new(epoch * EPOCH_MS + 120_000, "bolus").set("u", Some(1.5.into())),
        );
        let segment = subject.seal(epoch, &records).unwrap();
        wire::publish(&store, &subject_key, &segment).await.unwrap();

        // ---- and granting the peer, by the string the peer printed ----
        let (welcome, _tag) = subject.grant(peer_identity.bundle.clone(), "follow").unwrap();
        wire::publish_control(&store, &subject_key, &welcome).await.unwrap();

        let subject_identity_hex = diaswarm_keys::encode_identity(&diaswarm_keys::KeysIdentity {
            signer: subject_key.verifying_key(),
            bundle: subject_bundle,
        })
        .unwrap();

        // ---- the claim ----
        let g = granted_records(&dir, &subject_identity_hex, 0).await.expect("read");
        assert_eq!(g.skipped.lost(), 0, "a freshly granted peer hit {} fault(s)", g.skipped.lost());
        assert_eq!(g.skipped.not_ours, 0, "an unscoped grant left segments outside the window");
        assert!(g.opened >= 1, "no segment was opened at all");
        assert_eq!(g.records.len(), 61, "expected 60 readings and a bolus, got {}", g.records.len());
        assert!(g.records.iter().any(|r| r.kind() == "cgm"), "the cgm record is missing");
        assert!(g.records.iter().any(|r| r.kind() == "bolus"), "the bolus record is missing");

        // **THE WINDOW IS READ OFF THE KEYS, AND IT IS THE THING D29 IS ABOUT.**
        // A peer that cannot answer this cannot honestly label a clinician
        // screen, and one that answers from a flag it was passed is worse than
        // one that cannot answer at all.
        let from = g.granted_from.expect("a granted peer must know its own window");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(from <= now, "the window starts in the future: {from} > {now}");
        assert!(
            describe_window(g.granted_from).contains("day(s) of history"),
            "the window did not describe itself"
        );
        assert!(
            describe_window(None).contains("nothing is readable"),
            "a peer holding no secrets must not describe a window at all"
        );

        // **AND A FAULT IS NEVER CALLED A WINDOW.** This is the wording that an
        // unannounced rotation would otherwise have hidden behind.
        let faulty = diaswarm_keys::Skipped { undecryptable: 3, ..Default::default() };
        let said = describe_skipped(1, &faulty);
        assert!(said.contains("FAILED"), "a fault was not reported as one: {said}");
        assert!(
            !said.contains("outside this peer's window"),
            "a fault was described as access control: {said}"
        );

        // ---- reading twice must not re-join and lose the secret bundle ----
        let again = granted_records(&dir, &subject_identity_hex, 0).await.expect("reread");
        assert_eq!(again.records.len(), 61, "the second read lost records — it probably re-joined");

        // ---- and the export names the columns that arrived ----
        let out = dir.join("export.csv");
        cmd_export(&dir, &subject_identity_hex, &out, 0).await.expect("export");
        let csv = std::fs::read_to_string(&out).unwrap();
        let header = csv.lines().next().unwrap();
        assert!(header.starts_with("t,k"), "header does not lead with t,k: {header}");
        assert!(header.contains("mgdl"), "a column that arrived is missing: {header}");
        assert!(header.contains('u'), "a column that arrived is missing: {header}");

        // ---- AND THE CLINICIAN'S VIEW, THROUGH THE REAL COMMAND ----
        //
        // **THIS IS THE PATH A CLINIC ACTUALLY GETS**, and it runs over data
        // that came out of a real grant rather than a fixture: sealed by a
        // subject, replicated as operations, opened with a granted secret.
        let bundle_path = dir.join("summary.json");
        cmd_summary(&dir, &subject_identity_hex, 0, "patient-42", &bundle_path, fhir::Envelope::Document)
            .await
            .expect("summary");
        let bundle = std::fs::read_to_string(&bundle_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&bundle).expect("bundle is not JSON");
        assert_eq!(v["resourceType"], "Bundle");
        assert_eq!(v["type"], "document", "the handed-over file is a queue of HTTP POSTs");
        assert_eq!(v["entry"].as_array().unwrap().len(), 9);
        assert_eq!(
            v["entry"][1]["resource"]["identifier"][0]["value"], "patient-42",
            "the summary is about somebody else"
        );
        // The bolus must not have been counted as glucose.
        let summary = agp::Agp::from_records(&g.records).expect("agp");
        assert_eq!(summary.readings, 60, "a treatment leaked into the glucose statistics");
        assert_eq!(summary.cadence_seconds, 60, "the minute cadence was not detected");
        assert!(
            summary.insufficiency().is_some(),
            "one hour of data must not be reported as enough to act on"
        );

        // ---- AND THE ECOSYSTEM'S SHAPE, THROUGH THE REAL COMMAND ----
        let ns_dir = dir.join("ns");
        cmd_nightscout(&dir, &subject_identity_hex, 0, &ns_dir).await.expect("nightscout");
        let e: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ns_dir.join("entries.json")).unwrap())
                .expect("entries.json is not JSON");
        let t: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ns_dir.join("treatments.json")).unwrap())
                .expect("treatments.json is not JSON");
        assert_eq!(e.as_array().unwrap().len(), 60, "the 60 readings did not become entries");
        assert_eq!(t.as_array().unwrap().len(), 1, "the bolus did not become a treatment");
        assert_eq!(e[0]["type"], "sgv");
        assert_eq!(t[0]["eventType"], "Correction Bolus");
        assert_eq!(t[0]["insulin"], 1.5);
        assert_eq!(csv.lines().count(), 62, "expected a header and 61 rows");
    }
}
