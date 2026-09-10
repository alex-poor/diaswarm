//! Print the endpoint id for a raw 32-byte node key file. The secret is never printed.
//!
//! p2panda's node id is the ed25519 verifying key of the node's signing key,
//! which is exactly what `swarmJoin` builds with `SigningKey::from_bytes`.
fn main() {
    let p = std::env::args().nth(1).expect("path to node.key");
    let raw = std::fs::read(p).expect("read");
    let k: [u8; 32] = raw[..32].try_into().expect("32 bytes");
    let sk = ed25519_dalek::SigningKey::from_bytes(&k);
    println!("{}", diaswarm_core::vault::hex(sk.verifying_key().as_bytes()));
}
