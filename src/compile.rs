use std::fs::{File, Permissions};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::thread;
use package::{Package, SingleFilePackage};
use sandbox::{self, Pipe, Port, Sandbox};
use util::Pool;

pub trait Compiler {
    fn package(&self, source: Box<[u8]>) -> Box<Package>;
    fn compile(&self, source: Box<Package>, pool: &Pool<Sandbox>) -> Box<Package>;
}

#[derive(Serialize, Deserialize)]
pub struct BinaryCompiler {
    compiler_file: PathBuf,
    compiler_args: Box<[String]>,
    code_file: PathBuf,
    execute_file: PathBuf,
    execute_args: Box<[String]>,
}

impl Compiler for BinaryCompiler {
    fn package(&self, source: Box<[u8]>) -> Box<Package> {
        let package = SingleFilePackage::new(
            self.code_file.clone(), source, Permissions::from_mode(0o600));
        Box::new(package)
    }

    fn compile(&self, source: Box<Package>, pool: &Pool<Sandbox>) -> Box<Package> {
        let compiler_file = self.compiler_file.clone();
        let compiler_args = self.compiler_args.clone();
        let target_file = self.execute_file.clone();
        let mut sandbox = pool.get_one();
        source.install(&sandbox.in_dir());
        // TODO(iceboy): stdin, stdout, stderr, cgroup
        let status = sandbox.execute(
            compiler_file,
            compiler_args,
            sandbox::default_envs(),
            Box::new([]),
            None).unwrap();
        assert_eq!(status, 0);
        let mut foo =
            File::open(sandbox.out_dir().join(&target_file)).unwrap();
        let mut data = Vec::new();
        foo.read_to_end(&mut data).unwrap();
        let package = SingleFilePackage::new(
            target_file, data.into_boxed_slice(),
            foo.metadata().unwrap().permissions());
        pool.put(sandbox);
        Box::new(package)
    }
}

// TODO(iceboy): Run config for each target.
pub fn run(user_target: Box<Package>, judge_target: Box<Package>, pool: &Pool<Sandbox>) {
    let (mut user_sandbox, mut judge_sandbox) = pool.get_two();
    user_target.install(&user_sandbox.in_dir());
    judge_target.install(&judge_sandbox.in_dir());
    let (pin, pout) = Pipe::new();
    let user_thread = thread::spawn(move || {
        user_sandbox.execute(
            PathBuf::from("/in/foo"),
            Box::new([String::from("foo")]),
            sandbox::default_envs(),
            Box::new([(pout, Port::stdout())]),
            None)
    });
    let judge_result = judge_sandbox.execute(
        PathBuf::from("/in/foo"),
        Box::new([String::from("foo")]),
        sandbox::default_envs(),
        Box::new([(pin, Port::extra())]),
        None);
    // TODO(iceboy): Cleanup sandbox.
    pool.put(judge_sandbox);
    let user_result = user_thread.join().unwrap();
    //pool.put(user_sandbox);
    // TODO(iceboy)
    println!("User return code: {}", user_result.unwrap());
    println!("Judge return code: {}", judge_result.unwrap());
}
