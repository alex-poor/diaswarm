//! Does p2panda-net find a peer nobody told us about, and can our own protocol
//! share its socket?
use std::time::Duration;

use futures_util::StreamExt;
use p2panda_core::Hash;
use p2panda_net::iroh_mdns::MdnsDiscoveryMode;
use p2panda_net::{AddressBook, Discovery, Endpoint, Gossip, MdnsDiscovery};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let label = std::env::args().nth(1).unwrap_or_else(|| "node".into());
    let topic = Hash::digest(b"diaswarm-pool").into();

    let book = AddressBook::builder().spawn().await?;
    let ep = Endpoint::builder(book.clone()).spawn().await?;
    // ACTIVE, not the default. Spawning it without a mode did nothing at all,
    // which looked exactly like discovery failing.
    let _mdns = MdnsDiscovery::builder(book.clone(), ep.clone())
        .mode(MdnsDiscoveryMode::Active)
        .spawn()
        .await?;
    let _disc = Discovery::builder(book.clone(), ep.clone()).spawn().await?;
    let gossip = Gossip::builder(book.clone(), ep.clone()).spawn().await?;

    let iroh = ep.endpoint().await?;
    println!("[{label}] id {}", iroh.id());

    let pool = gossip.stream(topic).await?;
    let mut rx = pool.subscribe();
    let heard = label.clone();
    tokio::spawn(async move {
        while let Some(Ok(bytes)) = rx.next().await {
            println!("[{heard}] GOSSIP HEARD: {}", String::from_utf8_lossy(&bytes));
        }
    });

    for i in 0..20 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let _ = pool.publish(format!("hello from {label}").as_bytes()).await;
        let found = book.node_infos_by_topics([topic]).await.unwrap_or_default();
        println!("[{label}] t+{:>3}s  nodes on topic: {}", (i + 1) * 5, found.len());
    }
    Ok(())
}
