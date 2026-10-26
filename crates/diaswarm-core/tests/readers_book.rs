//! The private readers book: naming a reader and forgetting one.
//!
//! These back the two "Who can read" fixes. A keys-only grant never created a
//! book row (the only creator, `Vault::remember_reader`, is on the retired
//! core path), so the screen had only a key to show and a withdrawal could not
//! change the list. `remember_reader_name` now creates the row and labels it;
//! `forget_reader` removes it by key or by name. Both work straight on
//! `readers.json`, with no `meta.json`, which is the whole point on a keys-only
//! phone.

use diaswarm_core::vault::{forget_reader, read_readers_book, remember_reader_name};

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-readers-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

const KEY: &str = "68cbe8cd9549aa48f290e8b7cb426d8ee0cf4e5a3bb0e86b0443f002f3fece60";

#[test]
fn naming_a_reader_creates_the_row_when_there_was_none() {
    let dir = tmp("create");
    // A keys-only phone has no row yet — the name must bring one into being.
    assert!(read_readers_book(&dir).unwrap().is_empty());

    remember_reader_name(&dir, KEY, "follow", "Ayni").unwrap();

    let book = read_readers_book(&dir).unwrap();
    assert_eq!(book.len(), 1);
    assert!(book[0].reader.eq_ignore_ascii_case(KEY));
    assert_eq!(book[0].purpose, "follow");
    assert_eq!(book[0].name.as_deref(), Some("Ayni"));
}

#[test]
fn a_later_name_updates_but_an_empty_one_does_not_blank_it() {
    let dir = tmp("update");
    remember_reader_name(&dir, KEY, "follow", "Ayni").unwrap();
    // A re-share with a real handle updates the label...
    remember_reader_name(&dir, KEY, "follow", "Ayni's phone").unwrap();
    assert_eq!(read_readers_book(&dir).unwrap()[0].name.as_deref(), Some("Ayni's phone"));
    // ...but a keyless/handle-less re-share must not erase it.
    remember_reader_name(&dir, KEY, "follow", "   ").unwrap();
    assert_eq!(read_readers_book(&dir).unwrap()[0].name.as_deref(), Some("Ayni's phone"));
}

#[test]
fn forgetting_by_key_removes_the_row() {
    let dir = tmp("forget-key");
    remember_reader_name(&dir, KEY, "follow", "Ayni").unwrap();
    let removed = forget_reader(&dir, KEY).unwrap();
    assert_eq!(removed, 1);
    assert!(read_readers_book(&dir).unwrap().is_empty());
}

#[test]
fn forgetting_by_name_is_case_insensitive() {
    let dir = tmp("forget-name");
    remember_reader_name(&dir, KEY, "follow", "Ayni").unwrap();
    let removed = forget_reader(&dir, "aYnI").unwrap();
    assert_eq!(removed, 1, "pasting the name, in any case, must remove the reader");
    assert!(read_readers_book(&dir).unwrap().is_empty());
}

#[test]
fn forgetting_something_unknown_changes_nothing() {
    let dir = tmp("forget-miss");
    remember_reader_name(&dir, KEY, "follow", "Ayni").unwrap();
    let removed = forget_reader(&dir, "nobody-by-this-name-or-key").unwrap();
    assert_eq!(removed, 0);
    assert_eq!(read_readers_book(&dir).unwrap().len(), 1, "a miss must leave the list intact");
}
