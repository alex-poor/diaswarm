//! Serve a vault to peers, or fetch one from a peer.
//!
//! The half of the design that was never built: getting bytes to another human.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_net::wire::{fetch_as, serve, Who};
use iroh::{EndpointAddr, EndpointId, SecretKey};

fn usage() -> ExitCode {
    eprintln!(
        r#"diaswarm-net — move a vault between peers

  diaswarm-net serve <vault> <node-secret-file>
      Serve this vault. Prints the endpoint id others dial. Serves to anyone
      who asks: a vault is ciphertext, wraps nobody else can open, and a grant
      log that names nobody. Access is who holds a key, not who we serve.

  diaswarm-net fetch <endpoint-id> <into-dir> <identity-file> [purpose]
      Fetch a vault and the wraps for this identity. Fetches EVERY segment,
      not only the openable ones — holding ciphertext you cannot read is the
      point, and taking only what you can open would announce what you were
      granted.

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
            println!("  serving      {vault}");
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

        Some("fetch") => {
            let (Some(id), Some(into), Some(ident)) = (arg(1), arg(2), arg(3)) else {
                return usage();
            };
            let purpose = arg(4).unwrap_or("follow");
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

            let dir = PathBuf::from(into);
            match fetch_as(
                EndpointAddr::from(eid),
                Who::Reader { identity: reader, purpose: purpose.to_string() },
                &dir,
                false,
            )
            .await
            {
                Ok((segments, wraps)) => {
                    println!("  fetched      {segments} segments, {wraps} wraps for {purpose}");
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

        _ => return usage(),
    }
    ExitCode::SUCCESS
}
