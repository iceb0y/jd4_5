#[macro_use]
extern crate serde_derive;

extern crate bincode;
extern crate byteorder;
extern crate futures;
extern crate nix;
extern crate serde;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_serde_bincode;
extern crate tokio_uds;

mod subprocess;

fn main() {
    subprocess::fork_and_communicate();
}
