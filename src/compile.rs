use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use sandbox::{self, ExecuteResult, Pipe, Port, Sandbox};
use tempdir::TempDir;
use util::{self, Pool};

pub trait Compiler : Sync {
    fn compile(&self, source: &[u8], pool: &Pool<Sandbox>) -> Target;
}

pub struct BinaryCompiler {
    compiler_file: PathBuf,
    compiler_args: Box<[String]>,
    code_file: PathBuf,
    execute_file: PathBuf,
    execute_args: Box<[String]>,
}

pub struct Interpreter {
    code_file: PathBuf,
    execute_file: PathBuf,
    execute_args: Box<[String]>,
}

pub struct Target {
    package_dir: TempDir,
    execute_file: PathBuf,
    execute_args: Box<[String]>,
}

impl BinaryCompiler {
    pub fn new(
        compiler_file: PathBuf,
        compiler_args: Box<[String]>,
        code_file: PathBuf,
        execute_file: PathBuf,
        execute_args: Box<[String]>,
    ) -> BinaryCompiler {
        BinaryCompiler {
            compiler_file,
            compiler_args,
            code_file,
            execute_file,
            execute_args,
        }
    }
}

impl Compiler for BinaryCompiler {
    fn compile(&self, source: &[u8], pool: &Pool<Sandbox>) -> Target {
        let compiler_file = self.compiler_file.clone();
        let compiler_args = self.compiler_args.clone();
        let mut sandbox = pool.get_one();
        let mut code_file =
            File::create(sandbox.in_dir().join(&self.code_file)).unwrap();
        code_file.write_all(&source).unwrap();
        // TODO(iceboy): stdin, stdout, stderr, cgroup
        let status = sandbox.execute(
            compiler_file,
            compiler_args,
            sandbox::default_envs(),
            PathBuf::from("/out"),
            Box::new([]),
            None).unwrap();
        assert_eq!(status, 0);
        let package_dir = TempDir::new("jd-package").unwrap();
        util::copy_dir(&sandbox.out_dir(), package_dir.path());
        sandbox.cleanup();
        pool.put(sandbox);
        Target {
            package_dir,
            execute_file: self.execute_file.clone(),
            execute_args: self.execute_args.clone(),
        }
    }
}

impl Interpreter {
    pub fn new(
        code_file: PathBuf,
        execute_file: PathBuf,
        execute_args: Box<[String]>,
    ) -> Interpreter {
        Interpreter { code_file, execute_file, execute_args }
    }
}

impl Compiler for Interpreter {
    fn compile(&self, source: &[u8], _: &Pool<Sandbox>) -> Target {
        let package_dir = TempDir::new("jd-package").unwrap();
        let mut file =
            File::create(package_dir.path().join(&self.code_file)).unwrap();
        file.write_all(source).unwrap();
        drop(file);
        Target {
            package_dir,
            execute_file: self.execute_file.clone(),
            execute_args: self.execute_args.clone(),
        }
    }
}

impl Target {
    pub fn execute(
        &self,
        sandbox: &mut Sandbox,
        envs: Box<[String]>,
        pipes: Box<[(Pipe, Port)]>,
        cgroup_file: Option<PathBuf>
    ) -> ExecuteResult {
        let install_dir = sandbox.in_dir().join("package");
        fs::create_dir(&install_dir).unwrap();
        util::copy_dir(self.package_dir.path(), &install_dir);
        sandbox.execute(
            PathBuf::from("/in/package").join(&self.execute_file),
            self.execute_args.clone(),
            envs,
            PathBuf::from("/in/package"),
            pipes,
            cgroup_file)
    }
}

pub fn run(user_target: Target, judge_target: Target, pool: &Pool<Sandbox>) {
    let (mut user_sandbox, mut judge_sandbox) = pool.get_two();
    let (pin, pout) = Pipe::new();
    let user_thread = thread::spawn(move || {
        let user_result = user_target.execute(
            &mut user_sandbox,
            sandbox::default_envs(),
            Box::new([(pout, Port::stdout())]),
            None);
        (user_result, user_sandbox)
    });
    let judge_result = judge_target.execute(
        &mut judge_sandbox,
        sandbox::default_envs(),
        Box::new([(pin, Port::extra())]),
        None);
    judge_sandbox.cleanup();
    pool.put(judge_sandbox);
    let (user_result, mut user_sandbox) = user_thread.join().unwrap();
    user_sandbox.cleanup();
    pool.put(user_sandbox);
    // TODO(iceboy)
    println!("User return code: {}", user_result.unwrap());
    println!("Judge return code: {}", judge_result.unwrap());
}
