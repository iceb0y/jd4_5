extern crate futures;
extern crate jd4_5;
extern crate tempdir;
extern crate tokio_core;

use std::process;
use futures::Future;
use jd4_5::sandbox::{Bind, Sandbox};
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let future = Sandbox::new(&Bind::defaults(), &core.handle())
        .and_then(|sandbox| {
            sandbox.execute("/bin/bash", vec![String::from("bunny")], vec![])
        })
        .and_then(|(result, sandbox)| {
            sandbox.close().map(|()| result)
        });
    let result = core.run(future).unwrap();
    process::exit(result.unwrap());
}
