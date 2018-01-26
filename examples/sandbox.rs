extern crate futures;
extern crate jd4_5;
extern crate tokio_core;

use std::ffi::CString;
use std::process;
use futures::Future;
use jd4_5::sandbox::Sandbox;
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let sandbox = Sandbox::new(&core.handle());
    let file = CString::new("/bin/bash").unwrap();
    let args = vec![CString::new("bunny").unwrap()];
    let future = sandbox.execute(file, args)
        .and_then(|(result, sandbox)| {
            sandbox.close().map(|()| result)
        });
    let result = core.run(future).unwrap();
    process::exit(result.unwrap());
}
