use std::fs::{self, File, Permissions};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use nix::sys::stat::Mode;
use nix::unistd;
use package::{Package, SingleFilePackage};
use sandbox::{self, ExecuteResult, OpenFile, OpenMode, Sandbox};
use util::Pool;

pub trait Compiler {
    fn package(&self, source: Box<[u8]>) -> Box<Package>;
    fn compile(&self, source: Box<Package>, pool: &Pool<Sandbox>) -> Box<Package>;
}

pub struct Pipe(Arc<PipeState>);

pub struct PipeState {
    path: Mutex<Option<PathBuf>>,
    condvar: Condvar,
}

pub struct PipeSpec(String, OpenFile);

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
        piped_run(
            &mut user_sandbox,
            PathBuf::from("/in/foo"),
            Box::new([String::from("foo")]),
            Box::new([(PipeSpec::stdout(), pout)]))
    });
    let judge_result = piped_run(
        &mut judge_sandbox,
        PathBuf::from("/in/foo"),
        Box::new([String::from("foo")]),
        Box::new([(PipeSpec::extra(), pin)]));
    // TODO(iceboy): Cleanup sandbox.
    pool.put(judge_sandbox);
    let user_result = user_thread.join().unwrap();
    //pool.put(user_sandbox);
    // TODO(iceboy)
    println!("User return code: {}", user_result.unwrap());
    println!("Judge return code: {}", judge_result.unwrap());
}

impl Pipe {
    pub fn new() -> (Pipe, Pipe) {
        let state = Arc::new(PipeState {
            path: Mutex::new(None),
            condvar: Condvar::new(),
        });
        (Pipe(state.clone()), Pipe(state))
    }

    pub fn into_fifo(self, path: &Path) {
        let mut maybe_path = self.0.path.lock().unwrap();
        match maybe_path.clone() {
            Some(existing_path) => {
                fs::hard_link(existing_path, path).unwrap();
            },
            None => {
                unistd::mkfifo(path, Mode::S_IWUSR).unwrap();
                *maybe_path = Some(path.to_path_buf());
                self.0.condvar.notify_all();
            },
        }
    }

    /* TODO: pub fn into_reader(self) -> File {

    }

    pub fn into_writer(self) -> File {

    }*/
}

impl PipeSpec {
    pub fn stdin() -> PipeSpec {
        PipeSpec(String::from("stdin"), OpenFile::new(
            PathBuf::from("/in/stdin"), Box::new([0]), OpenMode::ReadOnly))
    }

    pub fn stdout() -> PipeSpec {
        PipeSpec(String::from("stdout"), OpenFile::new(
            PathBuf::from("/in/stdout"), Box::new([1]), OpenMode::WriteOnly))
    }

    pub fn stderr() -> PipeSpec {
        PipeSpec(String::from("stderr"), OpenFile::new(
            PathBuf::from("/in/stderr"), Box::new([2]), OpenMode::WriteOnly))
    }

    pub fn extra() -> PipeSpec {
        PipeSpec(String::from("extra"), OpenFile::new(
            PathBuf::from("/in/extra"), Box::new([3]), OpenMode::ReadOnly))
    }
}

pub fn piped_run(
    sandbox: &mut Sandbox,
    file: PathBuf,
    args: Box<[String]>,
    pipes: Box<[(PipeSpec, Pipe)]>) -> ExecuteResult {
    let mut open_files = Vec::new();
    for (pipe_spec, pipe) in pipes.into_vec().into_iter() {
        let PipeSpec(name, open_file) = pipe_spec;
        pipe.into_fifo(&sandbox.in_dir().join(name));
        open_files.push(open_file);
    }
    sandbox.execute(
        file,
        args,
        sandbox::default_envs(),
        open_files.into_boxed_slice(),
        None)
}
