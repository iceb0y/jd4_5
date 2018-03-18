extern crate jd4_5;

use std::path::PathBuf;
use std::process;
use jd4_5::sandbox::{self, Sandbox};

fn main() {
    let mut sandbox = Sandbox::new();
    let status = sandbox.execute(
        PathBuf::from("/bin/bash"),
        Box::new([String::from("bunny")]),
        sandbox::default_envs(),
        PathBuf::from("/"),
        Box::new([]),
        None).unwrap();
    process::exit(status);
}
