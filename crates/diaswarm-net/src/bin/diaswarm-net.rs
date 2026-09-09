//! Serve a vault to peers, or fetch one from a peer.
//!
//! The half of the design that was never built: getting bytes to another human.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_net::wire::{fetch_as, have, serve, Who};
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
            match fetch_as(
                EndpointAddr::from(eid),
                subject,
                Who::Reader { identity: reader, purpose: purpose.to_string() },
                &dir,
                false,
            )
            .await
            {
                Ok((segments, wraps)) => {
                    println!("  fetched      {segments} segments, {wraps} wraps for {purpose}");
                    // SEGMENTS BUT NO WRAPS IS ITS OWN FAILURE, and it used to
                    // print as an unremarkable pair of numbers followed by an
                    // empty reading. It means the bytes arrived and none of
                    // them are for you: either nothing was granted, or the
                    // subject granted and then never wrapped what it sealed
                    // afterwards.
                    if segments > 0 && wraps == 0 {
                        println!(
                            "  NOTHING FOR YOU  the history is here but none of it is wrapped\n                             \x20              for this key and purpose ({purpose}). Check the\n                             \x20              subject granted THIS key, and that their vault\n                             \x20              has sealed anything since."
                        );
                    }
                    match Vault::open(&dir) {
                        Ok(vault) => {
                            let me = load_identity(Path::new(ident)).unwrap();
                            let opened = vault.read_as(&me, purpose).unwrap_or_default();
                            let records: usize = opened.values().map(Vec::len).sum();
                            println!("  opens        {} epochs, {records} records", opened.len());
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
                let me = load_identity(Path::new(ident)).unwrap();
                let result = fetch_as(
                    EndpointAddr::from(eid),
                    &subject,
                    Who::Reader { identity: me, purpose: purpose.clone() },
                    &dir,
                    false,
                )
                .await;

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
