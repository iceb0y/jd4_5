use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix;
use std::os::unix::net::UnixStream;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process;
use bincode;
use nix::fcntl::{self, OFlag};
use nix::mount::{self, MntFlags, MsFlags};
use nix::sched::{self, CloneFlags};
use nix::sys::socket;
use nix::sys::stat::{self, Mode, SFlag};
use nix::sys::wait::{self, WaitStatus};
use nix::unistd::{self, Uid, Gid};
use tempdir::TempDir;

pub struct Sandbox {
    stream: UnixStream,
    dir: TempDir,
}

pub type ExecuteResult = Result<i32, ExecuteError>;

#[derive(Serialize, Deserialize, Debug)]
pub enum ExecuteError {
    Signaled(i32),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenFile {
    file: PathBuf,
    fds: Box<[RawFd]>,
    mode: OpenMode,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum OpenMode {
    ReadOnly,
    WriteOnly,
}

#[derive(Serialize, Deserialize)]
enum Request {
    Execute(ExecuteCommand),
}

#[derive(Serialize, Deserialize)]
struct ExecuteCommand {
    file: PathBuf,
    args: Box<[String]>,
    envs: Box<[String]>,
    open_files: Box<[OpenFile]>,
    cgroup_file: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Bind {
    source: PathBuf,
    target: PathBuf,
    mode: AccessMode,
}

#[derive(Serialize, Deserialize, Debug)]
enum AccessMode {
    ReadOnly,
    ReadWrite,
}

pub fn default_envs() -> Box<[String]> {
    Box::new([String::from("PATH=/usr/bin:/bin"), String::from("HOME=/")])
}

impl Sandbox {
    // TODO(iceboy): close existing fds
    pub fn new() -> Sandbox {
        let (parent_fd, child_fd) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Stream,
            None,
            socket::SockFlag::empty()).unwrap();
        let sandbox_dir = TempDir::new("sandbox").unwrap();
        let in_dir = sandbox_dir.path().join("in");
        fs::create_dir(&in_dir).unwrap();
        let out_dir = sandbox_dir.path().join("out");
        fs::create_dir(&out_dir).unwrap();
        let mount_dir = sandbox_dir.path().join("mount");
        fs::create_dir(&mount_dir).unwrap();
        let mut binds = Bind::defaults().into_vec();
        binds.push(
            Bind::new(in_dir, PathBuf::from("in"), AccessMode::ReadOnly));
        binds.push(
            Bind::new(out_dir, PathBuf::from("out"), AccessMode::ReadWrite));
        match unistd::fork().unwrap() {
            unistd::ForkResult::Parent { .. } => {
                unistd::close(child_fd).unwrap();
            },
            unistd::ForkResult::Child => {
                unistd::close(parent_fd).unwrap();
                do_child(&mount_dir, &binds, child_fd);
            },
        };
        Sandbox {
            stream: unsafe { UnixStream::from_raw_fd(parent_fd) },
            dir: sandbox_dir,
        }
    }

    pub fn in_dir(&self) -> PathBuf { self.dir.path().join("in") }
    pub fn out_dir(&self) -> PathBuf { self.dir.path().join("out") }

    pub fn execute(
        &mut self,
        file: PathBuf,
        args: Box<[String]>,
        envs: Box<[String]>,
        open_files: Box<[OpenFile]>,
        cgroup_file: Option<PathBuf>,
    ) -> ExecuteResult {
        bincode::serialize_into(&mut self.stream, &Request::Execute(
            ExecuteCommand { file, args, envs, open_files, cgroup_file }))
            .unwrap();
        bincode::deserialize_from(&mut self.stream).unwrap()
    }
}

impl OpenFile {
    pub fn new(file: PathBuf, fds: Box<[RawFd]>, mode: OpenMode) -> OpenFile {
        OpenFile { file, fds, mode }
    }
}

impl Bind {
    fn new(source: PathBuf, target: PathBuf, mode: AccessMode) -> Bind {
        assert!(source.is_absolute());
        assert!(target.is_relative());
        Bind { source, target, mode }
    }

    fn defaults() -> Box<[Bind]> {
        fn ro(source: &str, target: &str) -> Bind {
            Bind::new(
                PathBuf::from(source),
                PathBuf::from(target),
                AccessMode::ReadOnly)
        }
        Box::new([
            ro("/bin", "bin"),
            ro("/etc/alternatives", "etc/alternatives"),
            ro("/lib", "lib"),
            ro("/lib64", "lib64"),
            ro("/usr/bin", "usr/bin"),
            ro("/usr/include", "usr/include"),
            ro("/usr/lib", "usr/lib"),
            ro("/usr/lib64", "usr/lib64"),
            ro("/usr/libexec", "usr/libexec"),
            ro("/usr/share", "usr/share"),
            ro("/var/lib/ghc", "var/lib/ghc"),
        ])
    }
}

fn do_child(mount_dir: &Path, binds: &[Bind], child_fd: RawFd) -> ! {
    init_sandbox(mount_dir, binds);
    let mut stream = unsafe { UnixStream::from_raw_fd(child_fd) };
    loop {
        match bincode::deserialize_from(&mut stream).unwrap() {
            Request::Execute(command) => bincode::serialize_into(
                &mut stream, &do_execute(child_fd, command)).unwrap(),
        };
    }
}

fn init_sandbox(mount_dir: &Path, binds: &[Bind]) {
    let host_uid = unistd::geteuid();
    let host_gid = unistd::getegid();
    let guest_uid = Uid::from_raw(1000);
    let guest_gid = Gid::from_raw(1000);
    sched::unshare(CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWUTS |
                   CloneFlags::CLONE_NEWIPC | CloneFlags::CLONE_NEWUSER |
                   CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNET).unwrap();
    write_file("/proc/self/uid_map", &format!("{} {} 1", guest_uid, host_uid));
    write_file("/proc/self/setgroups", "deny");
    write_file("/proc/self/gid_map", &format!("{} {} 1", guest_gid, host_gid));
    unistd::setresuid(guest_uid, guest_uid, guest_uid).unwrap();
    unistd::setresgid(guest_gid, guest_gid, guest_gid).unwrap();
    unistd::sethostname("icebox").unwrap();
    // TODO(iceboy): Reap zombies?
    match unistd::fork().unwrap() {
        unistd::ForkResult::Parent { child } => {
            match wait::waitpid(child, None).unwrap() {
                WaitStatus::Exited(_, status) => {
                    process::exit(status as i32)
                },
                e => panic!("{:?}", e),
            }
        },
        unistd::ForkResult::Child => (),
    }
    mount::mount(Some("sandbox_root"),
                 mount_dir,
                 Some("tmpfs"),
                 MsFlags::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    env::set_current_dir(mount_dir).unwrap();
    fs::create_dir("proc").unwrap();
    mount::mount(Some("sandbox_proc"),
                 "proc",
                 Some("proc"),
                 MsFlags::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    fs::create_dir("dev").unwrap();
    bind_dev("/dev/null", "dev/null");
    bind_dev("/dev/urandom", "dev/urandom");
    fs::create_dir("tmp").unwrap();
    mount::mount(Some("sandbox_tmp"),
                 "tmp",
                 Some("tmpfs"),
                 MsFlags::MS_NOSUID,
                 Some("size=16m,nr_inodes=4k")).unwrap();
    binds.iter().for_each(bind_or_link);
    write_file("etc/passwd", "icebox:x:1000:1000:icebox:/:/bin/bash\n");
    fs::create_dir("old_root").unwrap();
    unistd::pivot_root(".", "old_root").unwrap();
    mount::umount2("old_root", MntFlags::MNT_DETACH).unwrap();
    fs::remove_dir("old_root").unwrap();
    mount::mount(Some("/"),
                 "/",
                 None as Option<&[u8]>,
                 MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY |
                 MsFlags::MS_REC | MsFlags::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
}

fn write_file(path: &str, data: &str) {
    File::create(path).unwrap().write_all(data.as_bytes()).unwrap();
}

fn bind_dev(source: &str, target: &str) {
    stat::mknod(
        target, SFlag::empty(), Mode::S_IRUSR | Mode::S_IWUSR, 0).unwrap();
    mount::mount(Some(source),
                 target,
                 None as Option<&[u8]>,
                 MsFlags::MS_BIND | MsFlags::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
}

fn bind_or_link(bind: &Bind) {
    let file_type = match fs::symlink_metadata(&bind.source) {
        Ok(attr) => attr.file_type(),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => panic!("{:?}", e),
    };
    if file_type.is_dir() {
        fs::create_dir_all(&bind.target).unwrap();
        mount::mount(Some(&bind.source),
                     &bind.target,
                     None as Option<&[u8]>,
                     MsFlags::MS_BIND | MsFlags::MS_REC | MsFlags::MS_NOSUID,
                     None as Option<&[u8]>).unwrap();
        match bind.mode {
            AccessMode::ReadOnly => mount::mount(
                Some(&bind.source),
                &bind.target,
                None as Option<&[u8]>,
                MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY |
                MsFlags::MS_REC | MsFlags::MS_NOSUID,
                None as Option<&[u8]>).unwrap(),
            _ => (),
        }
    } else if file_type.is_symlink() {
        let link = fs::read_link(&bind.source).unwrap();
        unix::fs::symlink(link, &bind.target).unwrap();
    }
}

fn do_execute(socket_fd: RawFd, command: ExecuteCommand) -> ExecuteResult {
    // TODO(iceboy): Reap zombies?
    match unistd::fork().unwrap() {
        unistd::ForkResult::Parent { child } => {
            match wait::waitpid(child, None).unwrap() {
                WaitStatus::Exited(_, status) => Ok(status as i32),
                WaitStatus::Signaled(_, signal, _) =>
                    Err(ExecuteError::Signaled(signal as i32)),
                e => panic!("{:?}", e),
            }
        },
        unistd::ForkResult::Child => {
            unistd::close(socket_fd).unwrap();
            for open_file in command.open_files.into_iter() {
                let flag = match open_file.mode {
                    OpenMode::ReadOnly => OFlag::O_RDONLY,
                    OpenMode::WriteOnly => OFlag::O_WRONLY,
                };
                let source_fd = fcntl::open(
                    &open_file.file, flag, stat::Mode::empty()).unwrap();
                for &target_fd in open_file.fds.iter() {
                    unistd::dup2(source_fd, target_fd).unwrap();
                }
                if open_file.fds.iter().any(|&fd| fd == source_fd) {
                    unistd::close(source_fd).unwrap();
                }
            }
            let file = CString::new(
                command.file.as_os_str().to_str().unwrap()).unwrap();
            let args: Vec<_> = command.args.into_iter()
                .map(|arg| CString::new(arg.as_str()).unwrap())
                .collect();
            let envs: Vec<_> = command.envs.iter()
                .map(|arg| CString::new(arg.as_str()).unwrap())
                .collect();
            unistd::execve(&file, &args, &envs).unwrap();
            panic!();
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn whoami() {
        let mut sandbox = Sandbox::new();
        let stdout_path = sandbox.out_dir().join("stdout");
        File::create(&stdout_path).unwrap();
        let status = sandbox.execute(
            PathBuf::from("/usr/bin/whoami"),
            Box::new([String::from("whoami")]),
            default_envs(),
            Box::new([OpenFile::new(PathBuf::from("/out/stdout"),
                                    Box::new([1]), OpenMode::WriteOnly)]),
            None).unwrap();
        assert_eq!(status, 0);
        let mut data = String::new();
        File::open(&stdout_path).unwrap().read_to_string(&mut data).unwrap();
        assert_eq!(data, "icebox\n");
        drop(sandbox);
    }

    #[test]
    fn read_only() {
        let mut sandbox = Sandbox::new();
        let status = sandbox.execute(
            PathBuf::from("/usr/bin/touch"),
            Box::new([String::from("touch"), String::from("/bin/dummy")]),
            default_envs(),
            Box::new([OpenFile::new(PathBuf::from("/dev/null"),
                                    Box::new([2]), OpenMode::WriteOnly)]),
            None).unwrap();
        assert_ne!(status, 0);
    }
}
