//! Print an invite for a subject and endpoint, so one can be handed over
//! without a camera. `cargo run -p diaswarm-core --example invite -- <subject> <endpoint> [purpose]`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let purpose = a.get(2).cloned().unwrap_or_else(|| "follow".to_string());
    match diaswarm_core::invite::Invite::new(&a[0], &a[1], &purpose) {
        Ok(i) => println!("{}", i.encode()),
        Err(e) => eprintln!("{e:?}"),
    }
}
