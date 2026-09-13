//! Does an endpoint built exactly as the app builds one reach its home relay?
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    for url in [
        diaswarm_net::swarm::DEFAULT_RELAY,
        "https://aps1-1.relay.n0.iroh.link",
    ] {
        let dir = std::env::temp_dir().join(format!("relayprobe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = p2panda_core::SigningKey::generate();
        let swarm = diaswarm_net::swarm::Swarm::join_via(
            &dir,
            key,
            diaswarm_net::swarm::default_network(),
            url,
        )
        .await?;
        let ep = swarm.iroh_endpoint().await?;
        use iroh::Watcher as _;
        let mut last = String::from("(none)");
        let mut ok = false;
        for _ in 0..30 {
            let st = ep.home_relay_status().get();
            last = format!("{st:?}");
            if last.contains("Connected") {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if ok {
            println!("CONNECTED  {url:?}\n           {last}");
        } else {
            println!("NO RELAY   {url:?} after 15s\n           {last}");
        }
    }
    Ok(())
}
