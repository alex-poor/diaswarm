//! Seal a canonical stream into a vault, grant it, revoke it, read it.
//!
//! The desktop half of the demonstration: a phone seals a day, someone holding
//! a granted key opens it, the grant is withdrawn, and the next day does not
//! open while every day they already had still does.
//!
//! No network. A vault is files, and moving them is a later stage.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Print, tolerating a closed pipe.
///
/// `println!` panics on EPIPE, so `diaswarm read … | head` ends in a Rust
/// backtrace rather than simply stopping. Downstream closing the pipe early is
/// normal, not an error.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

use diaswarm_core::seal::grant_tag;
use diaswarm_core::vault::{by_epoch, hex, unhex, Identity, Vault};
use diaswarm_core::Record;

fn usage() -> ExitCode {
    eprintln!(
        r#"diaswarm — seal, grant, revoke, read

  diaswarm keygen <identity-file>
      A new identity. The file holds secret keys; it is the whole of what
      being this party means.

  diaswarm init <vault> <subject-identity> <offset-hours>
      The offset is required, not defaulted. Epochs are days cut at a fixed
      phase, and picking it silently would pick someone's day boundary for
      them: at UTC+12, a UTC epoch runs local noon to noon, so one local day
      is split across two. Pass your standing UTC offset (12 for NZ). It does
      not follow DST — that is the point, an epoch must not depend on when or
      where a record was written.
  diaswarm seal <vault> <subject-identity> <stream.ndjson>
      Cut the stream into UTC-day epochs and seal each under its own key.

  diaswarm grant  <vault> <subject-identity> <reader-pub-hex> [purpose] [from-segment]
  diaswarm revoke <vault> <subject-identity> <reader-pub-hex> [purpose]
      Granting starts the wrapping. Revoking stops it IMMEDIATELY — the open
      segment is closed first, so nothing written afterwards is wrapped for
      them. Prospectively, though: nothing recalls what a reader already
      holds, and nothing here pretends to.

  diaswarm pub  <identity-file>          the public key to hand someone
  diaswarm tag  <identity-file> <other-pub-hex> <purpose>
      The name this pair uses inside a vault: the grant log records it and
      the wrap files are called after it. Either side derives it — subject
      with reader's public key, or reader with subject's. When a reader
      fetches segments but opens nothing, compare this against the log.
  diaswarm read <vault> <identity-file> [purpose]   what this reader can open
  diaswarm log  <vault> <subject-identity>   every grant and withdrawal, verified
  diaswarm rewrap <vault>
      Wrap every segment each granted reader is entitled to and has not been
      given. Sealing does this already; run it to repair a vault written
      before that was true, or one whose wraps were lost.
"#
    );
    ExitCode::from(2)
}

fn load(path: &Path) -> Result<Identity, String> {
    let raw = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes: [u8; 64] = raw
        .try_into()
        .map_err(|_| format!("{}: not a 64-byte identity", path.display()))?;
    Ok(Identity::from_bytes(&bytes))
}

fn reader_pub(arg: &str) -> Result<[u8; 32], String> {
    let bytes = unhex(arg).map_err(|_| "public key must be 64 hex characters".to_string())?;
    bytes.try_into().map_err(|_| "public key must be 32 bytes".to_string())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let arg = |i: usize| -> Result<&str, String> {
        args.get(i).map(String::as_str).ok_or_else(|| "missing argument".to_string())
    };

    match cmd {
        "keygen" => {
            let path = PathBuf::from(arg(1)?);
            let id = Identity::generate();
            fs::write(&path, id.to_bytes()).map_err(|e| e.to_string())?;
            // The secret is the whole of the identity; do not leave it readable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            out!("  identity  {}", path.display());
            out!("  public    {}", hex(&id.enc_public()));
        }

        "pub" => {
            let id = load(Path::new(arg(1)?))?;
            out!("{}", hex(&id.enc_public()));
        }

        // The tag is the only name a reader has inside a vault: it is what
        // grants are recorded under and what wrap files are called. When a
        // reader fetches and opens nothing, the question is always "is the tag
        // I derive the tag the subject filed under?" — and until this existed
        // there was no way to ask it without reading the source.
        //
        // Derivable from either side: the subject's identity plus the reader's
        // public key, or the reader's identity plus the subject's public key.
        // Both produce the same 32 bytes; that is the point of the ECDH.
        "tag" => {
            let id = load(Path::new(arg(1)?))?;
            let other = reader_pub(arg(2)?)?;
            out!("{}", hex(&grant_tag(&id.encryption, &other, arg(3)?)));
        }

        "init" => {
            let subject = load(Path::new(arg(2)?))?;
            let hours: i64 = arg(3)?
                .parse()
                .map_err(|_| "offset-hours must be a whole number, e.g. 12".to_string())?;
            let offset = hours * 3_600_000;
            Vault::create(Path::new(arg(1)?), &subject, offset).map_err(|e| format!("{e:?}"))?;
            out!("  vault    {}", arg(1)?);
            out!("  subject  {}", hex(&subject.enc_public()));
            out!("  epochs   days cut at UTC{hours:+}, fixed");
        }

        "seal" => {
            let vault = Vault::open(Path::new(arg(1)?)).map_err(|e| format!("{e:?}"))?;
            let _subject = load(Path::new(arg(2)?))?;
            let text = fs::read_to_string(arg(3)?).map_err(|e| e.to_string())?;
            let records: Vec<Record> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| Record::from_json(l).ok())
                .collect();
            let grouped: BTreeMap<i64, Vec<Record>> = by_epoch(&records, vault.offset());
            let mut sealed = 0;
            for (epoch, day) in &grouped {
                vault.seal(*epoch, day).map_err(|e| format!("{e:?}"))?;
                sealed += 1;
            }
            out!("  sealed   {sealed} epochs, {} records", records.len().saturating_sub(1));
        }

        "grant" | "revoke" => {
            let vault = Vault::open(Path::new(arg(1)?)).map_err(|e| format!("{e:?}"))?;
            let subject = load(Path::new(arg(2)?))?;
            let reader = reader_pub(arg(3)?)?;
            let purpose = args.get(4).map(String::as_str).unwrap_or("follow");
            if cmd == "revoke" {
                // Rotates first, so everything from here lands in a segment
                // this reader is not wrapped for. Immediate, not "at the next
                // day boundary" — which at UTC+12 could have been most of a day.
                let from = vault
                    .revoke(&subject, &reader, purpose)
                    .map_err(|e| format!("{e:?}"))?;
                out!("  stop     {} for {purpose}, from segment {from}", &arg(3)?[..16]);
                out!("           immediate — the current segment was closed first");
                return Ok(());
            }

            let tag = hex(&grant_tag(&subject.encryption, &reader, purpose));
            let prior = vault
                .grants()
                .map_err(|e| format!("{e:?}"))?
                .into_iter()
                .filter(|g| g.tag == tag)
                .map(|g| g.segment)
                .max();

            // A FIRST grant covers all history, because handing over the whole
            // record is what "share my data with my partner" means. A RE-grant
            // resumes from the next segment: dating it before an existing
            // withdrawal would put it earlier in a log read as "the latest
            // statement at or before this segment", so it would be silently
            // swallowed by the stop that follows — it would look like it worked
            // and do nothing.
            let at = match args.get(5).and_then(|s| s.parse::<u64>().ok()) {
                Some(explicit) => explicit,
                None if prior.is_some() => vault.next_seq().map_err(|e| format!("{e:?}"))?,
                None => 0,
            };
            if prior.is_some() && args.get(5).is_none() {
                out!("  note     re-granting from segment {at}, not from the start;");
                out!("           pass a segment to override, but a grant dated");
                out!("           before an existing withdrawal has no effect");
            }
            vault
                .record_grant(&subject, &reader, purpose, "grant", at)
                .map_err(|e| format!("{e:?}"))?;
            let n = vault
                .publish_wraps(&subject, &reader, purpose)
                .map_err(|e| format!("{e:?}"))?;
            out!("  grant    {} for {purpose} from segment {at}", &arg(3)?[..16]);
            out!("  wraps    {n} written");
        }

        // Repair, and the answer to "the reader says they can see nothing".
        //
        // Sealing keeps wraps current by itself, so this is only needed for a
        // vault written before it did — or one whose wraps were lost. It reads
        // the subject's private book, so it can only be run where that lives.
        "rewrap" => {
            let vault = Vault::open(Path::new(arg(1)?)).map_err(|e| format!("{e:?}"))?;
            let book = vault.readers().map_err(|e| format!("{e:?}"))?;
            if book.is_empty() {
                out!("  readers  none remembered — nothing to wrap for.");
                out!("           A grant made by an older build left no record of");
                out!("           whose key it was; re-run `grant` with the reader's");
                out!("           public key. The log will not gain a line for it.");
                return Ok(());
            }
            let done = vault.rewrap().map_err(|e| format!("{e:?}"))?;
            for k in &book {
                out!("  reader   {}… for {}", &k.reader[..16], k.purpose);
            }
            out!("  wraps    {} written", done.written);
            if done.unwrappable > 0 {
                out!("  LOST     {} segments have no key and open for nobody,", done.unwrappable);
                out!("           including you. Nothing can recover those.");
            }
        }

        "read" => {
            let vault = Vault::open(Path::new(arg(1)?)).map_err(|e| format!("{e:?}"))?;
            let reader = load(Path::new(arg(2)?))?;
            let purpose = args.get(3).map(String::as_str).unwrap_or("follow");
            let opened = vault.read_as(&reader, purpose).map_err(|e| format!("{e:?}"))?;
            let total: usize = opened.values().map(Vec::len).sum();
            let all = vault.epochs().map_err(|e| format!("{e:?}"))?;
            let segs = vault.segments().map_err(|e| format!("{e:?}"))?.len();
            let _ = segs;
            out!(
                "  opens    {} of {} epochs, {total} records",
                opened.len(),
                all.len()
            );
            // A histogram, because "33,465 records" does not say whether the
            // profile blocks that make the insulin interpretable are among them.
            let mut kinds: std::collections::BTreeMap<&str, usize> =
                std::collections::BTreeMap::new();
            for r in opened.values().flatten() {
                *kinds.entry(r.kind()).or_default() += 1;
            }
            let line: Vec<String> =
                kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
            out!("  kinds    {}", line.join("   "));

            for (epoch, records) in opened.iter().take(3) {
                let first = records.first().map(|r| r.to_canonical_json()).unwrap_or_default();
                out!("  {epoch}   {:>5} records   {}", records.len(), &first[..first.len().min(72)]);
            }
            if opened.len() > 3 {
                out!("  …        {} more", opened.len() - 3);
            }
        }

        "log" => {
            let vault = Vault::open(Path::new(arg(1)?)).map_err(|e| format!("{e:?}"))?;
            let subject = load(Path::new(arg(2)?))?;
            // The log itself names nobody. The subject's book does, and it
            // lives inside the vault but is never served — the same file
            // sealing uses to keep wraps current, so there is one record of
            // who reads rather than two that can disagree. The earlier version
            // kept a second book beside the identity holding only the first 16
            // hex of each key, which was enough to print a label and not
            // enough to wrap anything.
            let book = vault.readers().map_err(|e| format!("{e:?}"))?;
            for g in vault.grants().map_err(|e| format!("{e:?}"))? {
                let ok = Vault::verify(&g, &subject.verifying());
                let known = book.iter().find(|k| k.tag == g.tag);
                let label = known.map(|k| format!("{}… for {}", &k.reader[..16], k.purpose));
                let who = label.as_deref().unwrap_or("(unknown — not in this vault's book)");
                out!(
                    "  {}  {:<6} from segment {:>4}  tag {}  {}",
                    if ok { "signed " } else { "BAD SIG" },
                    g.act,
                    g.segment,
                    &g.tag[..12],
                    who
                );
            }
            match vault.verify_chain(&subject.verifying()).map_err(|e| format!("{e:?}"))? {
                None => out!("  chain intact — no entry has been removed or reordered"),
                Some(at) => out!("  CHAIN BROKEN at entry {at} — an entry was removed or altered"),
            }
        }

        _ => return Err(String::new()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => usage(),
        Err(e) => {
            eprintln!("  {e}");
            ExitCode::FAILURE
        }
    }
}
