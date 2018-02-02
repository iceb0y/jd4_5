use std::fs::File;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};
use futures::Future;
use package::{Package, SingleFilePackage};
use sandbox::Sandbox;
use util::Pool;

// TODO(iceboy): Compiler config, source package, compiled package
pub fn compile(source: Box<Package>, pool: &Arc<Mutex<Pool<Sandbox>>>)
    -> impl Future<Item = Box<Package>, Error = io::Error> {
    let pool_clone = pool.clone();
    pool.lock().unwrap().get(1).map_err(|_| panic!())
        .and_then(move |mut sandboxes| {
            let sandbox = sandboxes.pop().unwrap();
            source.install(&sandbox.in_dir());
            sandbox.execute(
                "/usr/bin/gcc",
                &["gcc", "-static", "-o", "/out/foo", "/in/foo.c"],
                &["PATH=/usr/bin:/bin", "HOME=/"],
                &[])
        })
        .and_then(move |(result, sandbox)| -> Result<Box<Package>, _> {
            // TODO(iceboy): Error handling.
            assert_eq!(result.unwrap(), 0);
            let mut foo = File::open(sandbox.out_dir().join("foo")).unwrap();
            let mut data = Vec::new();
            foo.read_to_end(&mut data).unwrap();
            let package = SingleFilePackage::new(
                "foo", &data, foo.metadata().unwrap().permissions());
            // TODO(iceboy): Cleanup sandbox.
            pool_clone.lock().unwrap().put(sandbox);
            Ok(Box::new(package))
        })
}

// TODO(iceboy): Run config.
pub fn run(target: Box<Package>, pool: &Arc<Mutex<Pool<Sandbox>>>)
    -> impl Future<Item = (), Error = io::Error> {
    let pool_clone = pool.clone();
    pool.lock().unwrap().get(1).map_err(|_| panic!())
        .and_then(move |mut sandboxes| {
            let sandbox = sandboxes.pop().unwrap();
            target.install(&sandbox.in_dir());
            sandbox.execute(
                "/in/foo",
                &["foo"],
                &["PATH=/usr/bin:/bin", "HOME=/"],
                &[])
        })
        .and_then(move |(result, sandbox)| {
            // TODO(iceboy): Error handling.
            assert_eq!(result.unwrap(), 0);
            // TODO(iceboy): Cleanup sandbox.
            pool_clone.lock().unwrap().put(sandbox);
            Ok(())
        })
}
