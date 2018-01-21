#![feature(conservative_impl_trait)]

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

use futures::Future;
use subprocess::Subprocess;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let subprocess = Subprocess::new(&handle).unwrap();
    core.run(
        subprocess.backdoor()
            .and_then(|subprocess| {
                subprocess.close()
            })).unwrap();
}
