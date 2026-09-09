//! Serve a vault to peers, or fetch one from a peer.
//!
//! The half of the design that was never built: getting bytes to another human.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_net::peer::{add_follow, load_follows, refresh_all};
use diaswarm_net::wire::{fetch_as, have, serve};
use iroh::{EndpointAddr, EndpointId, SecretKey};

fn usage() -> ExitCode {
    eprintln!(
        r#"diaswarm-net — move a vault between peers

  diaswarm-net serve <store> <node-secret-file>
      Serve EVERY vault in the store — your own and the ones you have
      replicated. That is what makes this a swarm rather than a personal
      server: a reader can get a subject's history from anyone holding it,
      so availability stops depending on that subject's phone being awake.
      Serves to anyone who asks; the bytes are ciphertext and a grant log
      that names nobody.

  diaswarm-net have <endpoint-id>
      Which subjects a peer holds.

  diaswarm-net fetch <endpoint-id> <subject-hex> <store> <identity> [purpose]
      Fetch one subject's vault from a peer into the store — and by landing
      in the store, it is then served onward by this node too. Fetches EVERY
      segment, not only the openable ones: holding ciphertext you cannot read
      is the point, and taking only what you can open would announce what you
      were granted.

  diaswarm-net follow <endpoint-id> <subject-hex> <store> <identity> [purpose] [seconds]
      Keep a local replica in step, and say what it can see. Reports the
      latest reading and HOW OLD IT IS, because a follower's dangerous
      failure is not an error on screen — it is a stale number that looks
      current. §12.3: "nothing happened" and "nothing arrived" must not
      look alike.

  diaswarm-net keep <store> <invite>
      Start keeping a copy of whoever sent you that invite. One string,
      checksummed, rather than two 64-character keys moved by hand — a
      transposed character in those produces a perfectly valid key belonging
      to nobody, and the failure is silence.

  diaswarm-net keep <store> <subject-hex> <from-endpoint-id> [purpose]
      Add a subject to what this peer keeps a copy of, or add another
      endpoint to try for one it already keeps. Give a purpose to read it;
      omit one to relay it — holding a history you cannot open is a normal
      thing to do here, not a broken setup.

  diaswarm-net peer <store> <node-secret-file> <identity> [seconds]
      BE A PEER: serve everything in the store, and keep everything in
      follows.json up to date, in one process. This is the difference
      between a swarm and a demo. `follow` is a leaf — it takes a subject's
      data and gives nothing back, so every reader depends on that subject's
      phone being awake. A peer serves what it has replicated, so a partner
      still sees glucose while the phone is asleep, and the grant log gets
      the other copies D13 needs for truncation to be detectable.

      Each subject is fetched from the first endpoint that answers. That
      list is the mechanism, not a fallback: any peer holding a subject
      serves identical bytes.

  diaswarm-net nodekey <file>        a stable node secret, created if absent
"#
    );
    ExitCode::from(2)
}

fn load_identity(path: &Path) -> Result<Identity, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes: [u8; 64] = raw.try_into().map_err(|_| "not a 64-byte identity".to_string())?;
    Ok(Identity::from_bytes(&bytes))
}

/// The most recent CGM reading this reader can open, and when it was taken.
fn latest_reading(dir: &Path, reader: &Identity, purpose: &str) -> Option<(i64, f64)> {
    let vault = Vault::open(dir).ok()?;
    let opened = vault.read_as(reader, purpose).ok()?;
    opened
        .values()
        .flatten()
        .filter(|r| r.kind() == "cgm")
        .filter_map(|r| Some((r.t(), r.get("mgdl")?.as_f64()?)))
        .max_by_key(|(t, _)| *t)
}

fn node_secret(path: &Path) -> Result<SecretKey, String> {
    if let Ok(raw) = std::fs::read(path) {
        let b: [u8; 32] = raw.try_into().map_err(|_| "not a 32-byte node key".to_string())?;
        return Ok(SecretKey::from_bytes(&b));
    }
    let sk = SecretKey::generate();
    std::fs::write(path, sk.to_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(sk)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);

    match arg(0) {
        Some("nodekey") => {
            let Some(p) = arg(1) else { return usage() };
            match node_secret(Path::new(p)) {
                Ok(sk) => println!("  endpoint id  {}", sk.public()),
                Err(e) => {
                    eprintln!("  {e}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some("keep") => {
            // ONE ARGUMENT IS AN INVITE, three are the pieces by hand.
            // Nobody should have to move two 64-character hex strings between
            // two devices correctly; the invite carries both with a checksum,
            // so a mistyped character is an error rather than a fetch that
            // reaches nobody.
            if let (Some(store), Some(text), None) = (arg(1), arg(2), arg(3)) {
                match diaswarm_core::invite::Invite::parse(text) {
                    Ok(inv) => {
                        match add_follow(
                            Path::new(store),
                            &inv.subject,
                            &inv.endpoint,
                            Some(&inv.purpose),
                        ) {
                            Ok(changed) => {
                                println!(
                                    "  {}    {}…  to read, as {}",
                                    if changed { "keeping " } else { "unchanged" },
                                    &inv.subject[..16],
                                    inv.purpose
                                );
                                println!("  from         {}", inv.endpoint);
                                println!();
                                println!("  They still have to grant your key before you can read");
                                println!("  any of it. Yours is:");
                                println!("      (run `diaswarm pub <your identity>` and send it over)");
                            }
                            Err(e) => {
                                eprintln!("  {e:#}");
                                return ExitCode::FAILURE;
                            }
                        }
                        return ExitCode::SUCCESS;
                    }
                    Err(e) => {
                        eprintln!("  {e:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }

            let (Some(store), Some(subject), Some(from)) = (arg(1), arg(2), arg(3)) else {
                return usage();
            };
            let purpose = arg(4);
            match add_follow(Path::new(store), subject, from, purpose) {
                Ok(changed) => {
                    let how = match purpose {
                        Some(p) => format!("to read, as {p}"),
                        None => "to relay — held, not readable".to_string(),
                    };
                    if changed {
                        println!("  keeping      {}…  {how}", &subject[..16.min(subject.len())]);
                        println!("  from         {from}");
                    } else {
                        println!("  unchanged    already keeping that, from that endpoint");
                    }
                }
                Err(e) => {
                    eprintln!("  {e:#}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some("peer") => {
            let (Some(store), Some(key), Some(ident)) = (arg(1), arg(2), arg(3)) else {
                return usage();
            };
            let every = arg(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(120);
            let store = PathBuf::from(store);
            let identity = match load_identity(Path::new(ident)) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("  {e}");
                    return ExitCode::FAILURE;
                }
            };
            let secret = match node_secret(Path::new(key)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  {e}");
                    return ExitCode::FAILURE;
                }
            };
            let follows = load_follows(&store).unwrap_or_default();
            println!("  endpoint id  {}", secret.public());
            println!("  serving      {}  (every vault in the store)", store.display());
            if follows.is_empty() {
                println!("  keeping      nothing yet — add some with `diaswarm-net keep`");
            }
            for f in &follows {
                let how = f.purpose.clone().unwrap_or_else(|| "relay".into());
                println!(
                    "  keeping      {}…  as {how}, from {} endpoint(s)",
                    &f.subject[..16],
                    f.from.len()
                );
            }
            let router = match serve(store.clone(), secret).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  {e:?}");
                    return ExitCode::FAILURE;
                }
            };
            println!("  online       refreshing every {every}s (ctrl-c to stop)");

            loop {
                match refresh_all(&store, false).await {
                    Ok(results) => {
                        for r in results {
                            let short = &r.subject[..16];
                            match &r.via {
                                Some(via) => {
                                    println!(
                                        "  {short}…  +{} segments, +{} wraps   via {}…",
                                        r.segments,
                                        r.wraps,
                                        &via[..16]
                                    );
                                    // For a subject this peer can read, say what
                                    // it reads AND HOW OLD IT IS. A follower's
                                    // dangerous failure is not an error on
                                    // screen — it is a number that looks current
                                    // and is nine hours old (§12.3). A relay
                                    // prints nothing here, because holding
                                    // something unreadable is the intent.
                                    if let Some(p) = follows
                                        .iter()
                                        .find(|f| f.subject == r.subject)
                                        .and_then(|f| f.purpose.as_deref())
                                    {
                                        let dir = store.join(&r.subject);
                                        match latest_reading(&dir, &identity, p) {
                                            Some((t, mgdl)) => {
                                                let now = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .map(|d| d.as_millis() as i64)
                                                    .unwrap_or(0);
                                                println!(
                                                    "            {:.1} mg/dL, {} min old",
                                                    mgdl,
                                                    (now - t) / 60_000
                                                );
                                            }
                                            None => println!(
                                                "            nothing opens as '{p}' — held, not readable"
                                            ),
                                        }
                                    }
                                }
                                // NOT SILENCE. A peer nobody could reach and a
                                // peer with nothing new must not look alike —
                                // §12.3, and the failure this whole display
                                // exists to catch.
                                None => {
                                    println!("  {short}…  NOT REACHED from any of its endpoints:");
                                    for (ep, why) in &r.failures {
                                        println!("       {}…  {}", &ep[..16.min(ep.len())], why.lines().next().unwrap_or(""));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => println!("  refresh failed: {e:#}"),
                }
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(every)) => {}
                }
            }
            router.shutdown().await.ok();
        }

        Some("serve") => {
            let (Some(vault), Some(key)) = (arg(1), arg(2)) else { return usage() };
            let secret = match node_secret(Path::new(key)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  {e}");
                    return ExitCode::FAILURE;
                }
            };
            println!("  serving      {vault}  (every vault in the store)");
            println!("  endpoint id  {}", secret.public());
            println!("  fetch with:  diaswarm-net fetch {} <dir> <identity>", secret.public());
            match serve(PathBuf::from(vault), secret).await {
                Ok(router) => {
                    println!("  online       waiting for peers (ctrl-c to stop)");
                    tokio::signal::ctrl_c().await.ok();
                    router.shutdown().await.ok();
                }
                Err(e) => {
                    eprintln!("  {e:?}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some("have") => {
            let Some(id) = arg(1) else { return usage() };
            let Ok(eid) = id.parse::<EndpointId>() else {
                eprintln!("  not an endpoint id: {id}");
                return ExitCode::FAILURE;
            };
            match have(EndpointAddr::from(eid), false).await {
                Ok(subjects) if subjects.is_empty() => println!("  holds nothing"),
                Ok(subjects) => {
                    for s in subjects {
                        println!("  {s}");
                    }
                }
                Err(e) => {
                    eprintln!("  {e:?}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some("fetch") => {
            let (Some(id), Some(subject), Some(into), Some(ident)) =
                (arg(1), arg(2), arg(3), arg(4))
            else {
                return usage();
            };
            let purpose = arg(5).unwrap_or("follow");
            let reader = match load_identity(Path::new(ident)) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  {e}");
                    return ExitCode::FAILURE;
                }
            };
            let Ok(eid) = id.parse::<EndpointId>() else {
                eprintln!("  not an endpoint id: {id}");
                return ExitCode::FAILURE;
            };

            let dir = PathBuf::from(into).join(subject.to_ascii_lowercase());
            match fetch_as(EndpointAddr::from(eid), subject, &dir, false).await {
                Ok((segments, wraps)) => {
                    println!("  fetched      {segments} segments, {wraps} new wraps");
                    match Vault::open(&dir) {
                        Ok(vault) => {
                            let opened = vault.read_as(&reader, purpose).unwrap_or_default();
                            let records: usize = opened.values().map(Vec::len).sum();
                            println!("  opens        {} epochs, {records} records", opened.len());
                            // HELD BUT UNREADABLE IS ITS OWN OUTCOME, and it
                            // used to print as an unremarkable pair of numbers
                            // followed by an empty reading.
                            //
                            // Judged on what OPENS, not on how many wraps just
                            // arrived. Those became different questions when
                            // peers started mirroring every reader's wraps: a
                            // second sync legitimately installs none, and a
                            // relay holds thousands it cannot use. Only "I can
                            // open nothing" is a fault.
                            if records == 0 && segments > 0 {
                                println!("  NOTHING FOR YOU  the history is here, and none of it opens for this");
                                println!("                   key as '{purpose}'. Check the subject granted THIS");
                                println!("                   key, and under this purpose.");
                            }
                        }
                        Err(e) => println!("  fetched, but the vault does not open: {e:?}"),
                    }
                }
                Err(e) => {
                    eprintln!("  {e:?}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some("follow") => {
            let (Some(id), Some(subject), Some(into), Some(ident)) =
                (arg(1), arg(2), arg(3), arg(4))
            else {
                return usage();
            };
            let subject = subject.to_ascii_lowercase();
            let purpose = arg(5).unwrap_or("follow").to_string();
            let every = arg(6).and_then(|s| s.parse::<u64>().ok()).unwrap_or(120);
            let Ok(eid) = id.parse::<EndpointId>() else {
                eprintln!("  not an endpoint id: {id}");
                return ExitCode::FAILURE;
            };
            let dir = PathBuf::from(into).join(&subject);
            let Ok(reader) = load_identity(Path::new(ident)) else {
                eprintln!("  cannot read {ident}");
                return ExitCode::FAILURE;
            };

            println!("  following    {} every {every}s", &id[..16]);
            let (mut reachable, mut missed) = (0u32, 0u32);

            loop {
                let result = fetch_as(EndpointAddr::from(eid), &subject, &dir, false).await;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);

                match result {
                    Ok((new_segments, wraps)) => {
                        reachable += 1;
                        match latest_reading(&dir, &reader, &purpose) {
                            // The age is the point. A number without one is a
                            // number someone acts on when it is hours old.
                            Some((t, mgdl)) => println!(
                                "  {:>5.1} mg/dL   {:>4} min old   +{} segments, {} wraps",
                                mgdl,
                                (now - t) / 60_000,
                                new_segments,
                                wraps
                            ),
                            None => println!("  nothing this reader can open"),
                        }
                    }
                    Err(e) => {
                        missed += 1;
                        // An unreachable subject is §9.7's availability problem.
                        // Counting it is the only way to know how bad it is on
                        // real hardware, which nothing in the docs measures.
                        // "unreachable" and "incompatible" are different
                        // problems for whoever is reading this: one is a phone
                        // in a tunnel, the other is a peer that needs updating.
                        let why = if e.to_string().contains("connect") {
                            "unreachable  "
                        } else {
                            "peer refused "
                        };
                        println!("  {why} {missed} missed of {}   {e}", reachable + missed);
                    }
                }
                // Flush explicitly: Rust block-buffers stdout when it is not a
                // terminal, so a follower piped into anything would print
                // nothing for minutes and look hung.
                use std::io::Write;
                let _ = std::io::stdout().flush();
                tokio::time::sleep(std::time::Duration::from_secs(every)).await;
            }
        }

        _ => return usage(),
    }
    ExitCode::SUCCESS
}
