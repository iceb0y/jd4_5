use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process;
use bincode::{self, Infinite};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use futures::{Future, Stream};
use futures::sink::Sink;
use nix::fcntl;
use nix::mount;
use nix::sched;
use nix::sys::socket;
use nix::sys::stat::{self, SFlag};
use nix::sys::wait::{self, WaitStatus};
use nix::unistd::{self, Uid, Gid};
use serde::{Serialize, Deserialize};
use tempdir::TempDir;
use tokio_core::reactor::Handle;
use tokio_io::codec::length_delimited::{self, Framed};
use tokio_serde_bincode::{ReadBincode, WriteBincode};
use tokio_uds;

pub struct Sandbox {
    stream: Framed<tokio_uds::UnixStream>,
    dir: TempDir,
}

pub type ExecuteResult = Result<i32, ExecuteError>;

#[derive(Serialize, Deserialize, Debug)]
pub enum ExecuteError {
    Signaled(i32),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenFile {
    file: PathBuf,
    fds: Vec<RawFd>,
    mode: OpenMode,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    args: Vec<String>,
    env: Vec<String>,
    open_files: Vec<OpenFile>,
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

const LENGTH_FIELD_LENGTH: usize = 2;
const MAX_FRAME_LENGTH: usize = 4096;

impl Sandbox {
    // TODO(iceboy): close existing fds
    pub fn new(handle: &Handle) -> Sandbox {
        let (parent_fd, child_fd) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Stream,
            0,
            socket::SockFlag::empty()).unwrap();
        let sandbox_dir = TempDir::new("sandbox").unwrap();
        let in_dir = sandbox_dir.path().join("in");
        fs::create_dir(&in_dir).unwrap();
        let out_dir = sandbox_dir.path().join("out");
        fs::create_dir(&out_dir).unwrap();
        let mount_dir = sandbox_dir.path().join("mount");
        fs::create_dir(&mount_dir).unwrap();
        let mut binds = Bind::defaults();
        binds.push(Bind::new(&in_dir, "in", AccessMode::ReadOnly));
        binds.push(Bind::new(&out_dir, "out", AccessMode::ReadWrite));
        match unistd::fork().unwrap() {
            unistd::ForkResult::Parent { .. } => {
                unistd::close(child_fd).unwrap();
            },
            unistd::ForkResult::Child => {
                unistd::close(parent_fd).unwrap();
                do_child(&mount_dir, &binds, child_fd);
            },
        };
        let net_stream =
            unsafe { unix::net::UnixStream::from_raw_fd(parent_fd) };
        let async_stream = tokio_uds::UnixStream::from_stream(
            net_stream, handle).unwrap();
        let framed_stream = length_delimited::Builder::new()
            .little_endian()
            .length_field_length(LENGTH_FIELD_LENGTH)
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_framed(async_stream);
        Sandbox { stream: framed_stream, dir: sandbox_dir }
    }

    pub fn in_dir(&self) -> PathBuf { self.dir.path().join("in") }
    pub fn out_dir(&self) -> PathBuf { self.dir.path().join("out") }

    pub fn execute<F: Into<PathBuf>>(
        self,
        file: F,
        args: &[&str],
        env: &[&str],
        open_files: &[OpenFile],
    ) -> impl Future<Item = (ExecuteResult, Sandbox), Error = io::Error> {
        let command = ExecuteCommand {
            file: file.into(),
            args: args.iter().map(|&s| s.to_string()).collect(),
            env: env.iter().map(|&s| s.to_string()).collect(),
            open_files: open_files.iter().cloned().collect(),
            cgroup_file: None,
        };
        self.call::<ExecuteResult>(Request::Execute(command))
    }

    fn call<ResponseT: for<'a> Deserialize<'a>>(self, request: Request)
        -> impl Future<Item = (ResponseT, Sandbox), Error = io::Error> {
        let Sandbox { stream, dir } = self;
        WriteBincode::new(stream).send(request)
            .and_then(|sink| {
                ReadBincode::new(sink.into_inner()).into_future()
                    .map_err(|(err, _)| {
                        io::Error::new(io::ErrorKind::InvalidData, err)
                    })
            })
            .and_then(|(maybe_response, source)| {
                match maybe_response {
                    Some(response) => {
                        let sandbox = Sandbox {
                            stream: source.into_inner(),
                            dir: dir,
                        };
                        Ok((response, sandbox))
                    },
                    None => {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData, "empty response"))
                    },
                }
            })
    }
}

impl OpenFile {
    pub fn new<F: Into<PathBuf>>(
        file: F, fds: Vec<RawFd>, mode: OpenMode) -> OpenFile {
        OpenFile { file: file.into(), fds, mode }
    }
}

impl Bind {
    fn new<S: Into<PathBuf>, T: Into<PathBuf>>(
        source: S,
        target: T,
        mode: AccessMode
    ) -> Bind {
        let source = source.into();
        let target = target.into();
        assert!(source.is_absolute());
        assert!(target.is_relative());
        Bind { source, target, mode }
    }

    fn defaults() -> Vec<Bind> {
        vec![
            Bind::new("/bin", "bin", AccessMode::ReadOnly),
            Bind::new("/etc/alternatives", "etc/alternatives", AccessMode::ReadOnly),
            Bind::new("/lib", "lib", AccessMode::ReadOnly),
            Bind::new("/lib64", "lib64", AccessMode::ReadOnly),
            Bind::new("/usr/bin", "usr/bin", AccessMode::ReadOnly),
            Bind::new("/usr/include", "usr/include", AccessMode::ReadOnly),
            Bind::new("/usr/lib", "usr/lib", AccessMode::ReadOnly),
            Bind::new("/usr/lib64", "usr/lib64", AccessMode::ReadOnly),
            Bind::new("/usr/libexec", "usr/libexec", AccessMode::ReadOnly),
            Bind::new("/usr/share", "usr/share", AccessMode::ReadOnly),
            Bind::new("/var/lib/ghc", "var/lib/ghc", AccessMode::ReadOnly),
        ]
    }
}

fn do_child<M: AsRef<Path>>(
    mount_dir: M,
    binds: &[Bind],
    child_fd: RawFd
) -> ! {
    init_sandbox(mount_dir, binds);
    let mut stream = unsafe { unix::net::UnixStream::from_raw_fd(child_fd) };
    let mut buffer = [0; MAX_FRAME_LENGTH];
    loop {
        let size = match stream.read_u16::<LittleEndian>() {
            Ok(size) => size as usize,
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof =>
                process::exit(0),
            Err(e) => panic!("{:?}", e),
        };
        assert!(size <= MAX_FRAME_LENGTH);
        stream.read_exact(&mut buffer[..size]).unwrap();
        match bincode::deserialize(&buffer[..size]).unwrap() {
            Request::Execute(command) =>
                do_execute(child_fd, command, &mut stream),
        };
    }
}

fn init_sandbox<M: AsRef<Path>>(mount_dir: M, binds: &[Bind]) {
    let host_uid = unistd::geteuid();
    let host_gid = unistd::getegid();
    let guest_uid = Uid::from_raw(1000);
    let guest_gid = Gid::from_raw(1000);
    sched::unshare(sched::CLONE_NEWNS | sched::CLONE_NEWUTS |
                   sched::CLONE_NEWIPC | sched::CLONE_NEWUSER |
                   sched::CLONE_NEWPID | sched::CLONE_NEWNET).unwrap();
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
                 mount_dir.as_ref(),
                 Some("tmpfs"),
                 mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    env::set_current_dir(&mount_dir).unwrap();
    fs::create_dir("proc").unwrap();
    mount::mount(Some("sandbox_proc"),
                 "proc",
                 Some("proc"),
                 mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    fs::create_dir("dev").unwrap();
    bind_dev("/dev/null", "dev/null");
    bind_dev("/dev/urandom", "dev/urandom");
    fs::create_dir("tmp").unwrap();
    mount::mount(Some("sandbox_tmp"),
                 "tmp",
                 Some("tmpfs"),
                 mount::MS_NOSUID,
                 Some("size=16m,nr_inodes=4k")).unwrap();
    binds.iter().for_each(bind_or_link);
    write_file("etc/passwd", "icebox:x:1000:1000:icebox:/:/bin/bash\n");
    fs::create_dir("old_root").unwrap();
    unistd::pivot_root(".", "old_root").unwrap();
    mount::umount2("old_root", mount::MNT_DETACH).unwrap();
    fs::remove_dir("old_root").unwrap();
    mount::mount(Some("/"),
                 "/",
                 None as Option<&[u8]>,
                 mount::MS_BIND | mount::MS_REMOUNT | mount::MS_RDONLY |
                 mount::MS_REC | mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
}

fn write_file(path: &str, data: &str) {
    File::create(path).unwrap().write_all(data.as_bytes()).unwrap();
}

fn bind_dev(source: &str, target: &str) {
    stat::mknod(
        target, SFlag::empty(), stat::S_IRUSR | stat::S_IWUSR, 0).unwrap();
    mount::mount(Some(source),
                 target,
                 None as Option<&[u8]>,
                 mount::MS_BIND | mount::MS_NOSUID,
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
                     mount::MS_BIND | mount::MS_REC | mount::MS_NOSUID,
                     None as Option<&[u8]>).unwrap();
        match bind.mode {
            AccessMode::ReadOnly => mount::mount(
                Some(&bind.source),
                &bind.target,
                None as Option<&[u8]>,
                mount::MS_BIND | mount::MS_REMOUNT | mount::MS_RDONLY |
                mount::MS_REC | mount::MS_NOSUID,
                None as Option<&[u8]>).unwrap(),
            _ => (),
        }
    } else if file_type.is_symlink() {
        let link = fs::read_link(&bind.source).unwrap();
        unix::fs::symlink(link, &bind.target).unwrap();
    }
}

fn do_execute(
    socket_fd: RawFd,
    command: ExecuteCommand,
    stream: &mut unix::net::UnixStream
) {
    // TODO(iceboy): Reap zombies?
    let response: ExecuteResult = match unistd::fork().unwrap() {
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
            for OpenFile { file, fds, mode } in command.open_files {
                let flag = match mode {
                    OpenMode::ReadOnly => fcntl::O_RDONLY,
                    OpenMode::WriteOnly => fcntl::O_WRONLY,
                };
                let source_fd =
                    fcntl::open(&file, flag, stat::Mode::empty()).unwrap();
                for &target_fd in &fds {
                    unistd::dup2(source_fd, target_fd).unwrap();
                }
                if fds.iter().any(|&fd| fd == source_fd) {
                    unistd::close(source_fd).unwrap();
                }
            }
            let file = CString::new(
                command.file.as_os_str().to_str().unwrap()).unwrap();
            let args: Vec<_> = command.args.into_iter()
                .map(|arg| CString::new(arg).unwrap())
                .collect();
            let env: Vec<_> = command.env.into_iter()
                .map(|arg| CString::new(arg).unwrap())
                .collect();
            unistd::execve(&file, &args, &env).unwrap();
            panic!();
        },
    };
    write_response(stream, &response);
}

fn write_response<ResponseT: Serialize>(
    stream: &mut unix::net::UnixStream, response: &ResponseT) {
    let size = bincode::serialized_size(response) as usize;
    assert!(size <= MAX_FRAME_LENGTH);
    stream.write_u16::<LittleEndian>(size as u16).unwrap();
    bincode::serialize_into(stream, response, Infinite).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_core::reactor::Core;

    #[test]
    fn whoami() {
        let mut core = Core::new().unwrap();
        let sandbox = Sandbox::new(&core.handle());
        let stdout_path = sandbox.out_dir().join("stdout");
        File::create(&stdout_path).unwrap();
        let future = sandbox.execute(
            "/usr/bin/whoami",
            &["whoami"],
            &["PATH=/usr/bin:/bin", "HOME=/"],
            &[OpenFile::new("/out/stdout", vec![1], OpenMode::WriteOnly)]);
        let (result, sandbox) = core.run(future).unwrap();
        assert_eq!(result.unwrap(), 0);
        let mut data = String::new();
        File::open(&stdout_path).unwrap().read_to_string(&mut data).unwrap();
        assert_eq!(data, "icebox\n");
        drop(sandbox);
    }

    #[test]
    fn read_only() {
        let mut core = Core::new().unwrap();
        let sandbox = Sandbox::new(&core.handle());
        let future = sandbox.execute(
            "/usr/bin/touch",
            &["touch", "/bin/dummy"],
            &["PATH=/usr/bin:/bin", "HOME=/"],
            &[OpenFile::new("/dev/null", vec![2], OpenMode::WriteOnly)]);
        let (result, _) = core.run(future).unwrap();
        assert_ne!(result.unwrap(), 0);
    }
}
