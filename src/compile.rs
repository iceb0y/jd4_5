use std::fs::{File, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::thread;
use package::{Package, SingleFilePackage};
use sandbox::{self, Pipe, Port, Sandbox};
use util::Pool;

pub trait Compiler : Sync {
    fn compile(&self, source: &[u8], pool: &Pool<Sandbox>) -> Box<Package>;
}

pub struct BinaryCompiler {
    compiler_file: PathBuf,
    compiler_args: Box<[String]>,
    code_file: PathBuf,
    target_file: PathBuf,
    target_args: Box<[String]>,
}

pub struct Interpreter {
    code_file: PathBuf,
    target_file: PathBuf,
    target_args: Box<[String]>,
}

impl BinaryCompiler {
    pub fn new(
        compiler_file: PathBuf,
        compiler_args: Box<[String]>,
        code_file: PathBuf,
        target_file: PathBuf,
        target_args: Box<[String]>,
    ) -> BinaryCompiler {
        BinaryCompiler {
            compiler_file,
            compiler_args,
            code_file,
            target_file,
            target_args,
        }
    }
}

impl Interpreter {
    pub fn new(
        code_file: PathBuf,
        target_file: PathBuf,
        target_args: Box<[String]>,
    ) -> Interpreter {
        Interpreter { code_file, target_file, target_args }
    }
}

impl Compiler for BinaryCompiler {
    fn compile(&self, source: &[u8], pool: &Pool<Sandbox>) -> Box<Package> {
        let compiler_file = self.compiler_file.clone();
        let compiler_args = self.compiler_args.clone();
        let mut sandbox = pool.get_one();
        let mut file = File::create(
            sandbox.in_dir().join(&self.code_file)).unwrap();
        file.write_all(&source).unwrap();
        // TODO(iceboy): stdin, stdout, stderr, cgroup
        let status = sandbox.execute(
            compiler_file,
            compiler_args,
            sandbox::default_envs(),
            Box::new([]),
            None).unwrap();
        assert_eq!(status, 0);
        let mut foo =
            File::open(sandbox.out_dir().join(&self.target_file)).unwrap();
        let mut data = Vec::new();
        foo.read_to_end(&mut data).unwrap();
        let package = SingleFilePackage::new(
            self.target_file.clone(), data.into_boxed_slice(),
            foo.metadata().unwrap().permissions());
        // TODO(iceboy): Cleanup sandbox.
        pool.put(sandbox);
        Box::new(package)
    }
}

impl Compiler for Interpreter {
    fn compile(&self, source: &[u8], _: &Pool<Sandbox>) -> Box<Package> {
        Box::new(SingleFilePackage::new(
            self.code_file.clone(),
            source.to_vec().into_boxed_slice(),
            Permissions::from_mode(0o600)))
    }
}

// TODO(iceboy): Run config for each target.
pub fn run(user_target: Box<Package>, judge_target: Box<Package>, pool: &Pool<Sandbox>) {
    let (mut user_sandbox, mut judge_sandbox) = pool.get_two();
    user_target.install(&user_sandbox.in_dir());
    judge_target.install(&judge_sandbox.in_dir());
    let (pin, pout) = Pipe::new();
    let user_thread = thread::spawn(move || {
        let user_result = user_sandbox.execute(
            PathBuf::from("/in/foo"),
            Box::new([String::from("foo")]),
            sandbox::default_envs(),
            Box::new([(pout, Port::stdout())]),
            None);
        (user_result, user_sandbox)
    });
    let judge_result = judge_sandbox.execute(
        PathBuf::from("/in/foo"),
        Box::new([String::from("foo")]),
        sandbox::default_envs(),
        Box::new([(pin, Port::extra())]),
        None);
    // TODO(iceboy): Cleanup sandbox.
    pool.put(judge_sandbox);
    let (user_result, user_sandbox) = user_thread.join().unwrap();
    pool.put(user_sandbox);
    // TODO(iceboy)
    println!("User return code: {}", user_result.unwrap());
    println!("Judge return code: {}", judge_result.unwrap());
}
