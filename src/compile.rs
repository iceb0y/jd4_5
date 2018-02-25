use std::fs::{self, File, Permissions};
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use futures::Future;
use nix::sys::stat::Mode;
use nix::unistd;
use package::{Package, SingleFilePackage};
use sandbox::{self, OpenFile, OpenMode, Sandbox};
use util::Pool;

pub trait Compiler {
    fn package(&self, source: Vec<u8>) -> Box<Package>;
    fn compile(&self, source: Box<Package>, pool: &Arc<Mutex<Pool<Sandbox>>>)
        -> Box<Future<Item = Box<Package>, Error = io::Error>>;
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BinaryCompiler {
    compiler_file: PathBuf,
    compiler_args: Vec<String>,
    code_file: PathBuf,
    execute_file: PathBuf,
    execute_args: Vec<String>,
}

impl Compiler for BinaryCompiler {
    fn package(&self, source: Vec<u8>) -> Box<Package> {
        let package = SingleFilePackage::new(
            self.code_file.clone(), source, Permissions::from_mode(0o600));
        Box::new(package)
    }

    fn compile(&self, source: Box<Package>, pool: &Arc<Mutex<Pool<Sandbox>>>)
        -> Box<Future<Item = Box<Package>, Error = io::Error>> {
        let compiler_file = self.compiler_file.clone();
        let compiler_args = self.compiler_args.clone();
        let target_file = self.execute_file.clone();
        let pool_clone = pool.clone();
        let future = pool.lock().unwrap().get(1).map_err(|_| panic!())
            .and_then(move |mut sandboxes| {
                let sandbox = sandboxes.pop().unwrap();
                source.install(&sandbox.in_dir());
                // TODO(iceboy): stdin, stdout, stderr, cgroup
                sandbox.execute(compiler_file, compiler_args,
                                sandbox::default_envs(), vec![], None)
            })
            .and_then(move |(result, sandbox)| -> Result<Box<Package>, _> {
                // TODO(iceboy): Error handling.
                assert_eq!(result.unwrap(), 0);
                let mut foo =
                    File::open(sandbox.out_dir().join(&target_file)).unwrap();
                let mut data = Vec::new();
                foo.read_to_end(&mut data).unwrap();
                let package = SingleFilePackage::new(
                    target_file, data, foo.metadata().unwrap().permissions());
                // TODO(iceboy): Cleanup sandbox.
                pool_clone.lock().unwrap().put(sandbox);
                Ok(Box::new(package))
            });
        Box::new(future)
    }
}

// TODO(iceboy): Run config for each target.
pub fn run(
    user_target: Box<Package>,
    judge_target: Box<Package>,
    pool: &Arc<Mutex<Pool<Sandbox>>>,
) -> Box<Future<Item = (), Error = io::Error>> {
    let pool_clone = pool.clone();
    let future = pool.lock().unwrap().get(2).map_err(|_| panic!())
        .and_then(move |mut sandboxes| {
            let user_sandbox = sandboxes.pop().unwrap();
            user_target.install(&user_sandbox.in_dir());
            let judge_sandbox = sandboxes.pop().unwrap();
            judge_target.install(&judge_sandbox.in_dir());
            unistd::mkfifo(&user_sandbox.in_dir().join("stdout"), Mode::S_IWUSR).unwrap();
            fs::hard_link(
                &user_sandbox.in_dir().join("stdout"),
                &judge_sandbox.in_dir().join("extra")).unwrap();
            user_sandbox.execute(
                PathBuf::from("/in/foo"),
                vec![String::from("foo")],
                sandbox::default_envs(),
                vec![OpenFile::new(PathBuf::from("/in/stdout"), vec![1], OpenMode::WriteOnly)],
                None,
            ).join(judge_sandbox.execute(
                PathBuf::from("/in/foo"),
                vec![String::from("foo")],
                sandbox::default_envs(),
                vec![OpenFile::new(PathBuf::from("/in/extra"), vec![3], OpenMode::ReadOnly)],
                None,
            ))
        })
        .and_then(move |((user_result, user_sandbox), (judge_result, judge_sandbox))| {
            // TODO(iceboy)
            println!("User return code: {}", user_result.unwrap());
            println!("Judge return code: {}", judge_result.unwrap());
            // TODO(iceboy): Cleanup sandbox.
            let mut pool_locked = pool_clone.lock().unwrap();
            pool_locked.put(user_sandbox);
            pool_locked.put(judge_sandbox);
            Ok(())
        });
    Box::new(future)
}
