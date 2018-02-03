extern crate futures;
extern crate jd4_5;
extern crate tempdir;
extern crate tokio_core;

use std::path::PathBuf;
use std::process;
use jd4_5::sandbox::{self, Sandbox};
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let sandbox = Sandbox::new(&core.handle());
    let future = sandbox.execute(
        PathBuf::from("/bin/bash"),
        vec![String::from("bunny")],
        sandbox::default_envs(),
        vec![],
        None);
    let (result, _) = core.run(future).unwrap();
    process::exit(result.unwrap());
}
