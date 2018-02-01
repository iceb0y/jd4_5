extern crate futures;
extern crate jd4_5;
extern crate tempdir;
extern crate tokio_core;

use std::process;
use jd4_5::sandbox::Sandbox;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let sandbox = Sandbox::new(&core.handle());
    let future =
        sandbox.execute("/bin/bash", vec![String::from("bunny")], vec![]);
    let (result, _) = core.run(future).unwrap();
    process::exit(result.unwrap());
}
