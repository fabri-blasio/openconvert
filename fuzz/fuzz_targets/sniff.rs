//! The highest-value target in the set.
//!
//! `sniff` reads the first bytes of **every hostile file the product will ever
//! see**, before any decision about isolation has been made -- so it is the one
//! parser that cannot itself be sandboxed. A panic here is a denial of service
//! in every front end at once.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = openconvert_run::detect::sniff(&mut Cursor::new(data.to_vec()), None);
});
