extern crate jd4_5;
extern crate futures;
extern crate tokio_core;

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use futures::Future;
use tokio_core::reactor::Core;
use jd4_5::compile;
use jd4_5::package::SingleFilePackage;
use jd4_5::sandbox::Sandbox;
use jd4_5::util::Pool;

pub fn main() {
    let source = SingleFilePackage::new("foo.c", "#include <stdio.h>

int main(void) {
    printf(\"Hello world!\\n\");
}".as_bytes(), Permissions::from_mode(0o600));
    let mut core = Core::new().unwrap();
    let pool: Arc<Mutex<Pool<Sandbox>>> = Arc::new(Mutex::new(Pool::new()));
    pool.lock().unwrap().put(Sandbox::new(&core.handle()));
    let future = compile::compile(Box::new(source), &pool)
        .and_then(|target| compile::run(target, &pool));
    core.run(future).unwrap();
}
