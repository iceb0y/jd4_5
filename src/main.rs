#[macro_use]
extern crate serde_derive;

extern crate bincode;
extern crate nix;
extern crate serde;

mod subprocess;

fn main() {
    subprocess::fork_and_communicate();
}
