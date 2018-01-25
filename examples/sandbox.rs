extern crate futures;
extern crate jd4_5;
extern crate tokio_core;

use std::process;
use futures::Future;
use jd4_5::sandbox::Sandbox;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let sandbox = Sandbox::new(&core.handle());
    let future = sandbox.execute("bash")
        .and_then(|(result, sandbox)| {
            sandbox.close().map(|()| result)
        });
    let result = core.run(future).unwrap();
    process::exit(result.unwrap());
}
