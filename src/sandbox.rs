use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix;
use std::os::unix::io::{FromRawFd, RawFd};
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
use tokio_core::reactor::Handle;
use tokio_io::codec::length_delimited::{self, Framed};
use tokio_serde_bincode::{ReadBincode, WriteBincode};
use tokio_uds;

pub struct Sandbox(Framed<tokio_uds::UnixStream>);

pub type ExecuteResult = Result<i32, ExecuteError>;

#[derive(Serialize, Deserialize, Debug)]
pub enum ExecuteError {
    Signaled(i32),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum OpenMode {
    Read,
    Write,
}

#[derive(Serialize, Deserialize)]
enum Request {
    Execute(ExecuteCommand),
    Close,
}

#[derive(Serialize, Deserialize)]
struct ExecuteCommand {
    file: String,
    args: Vec<String>,
    open_files: Vec<(String, OpenMode, Vec<RawFd>)>,
    cgroup_file: Option<String>,
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
        match unistd::fork().unwrap() {
            unistd::ForkResult::Parent { .. } => {
                unistd::close(child_fd).unwrap();
            },
            unistd::ForkResult::Child => {
                unistd::close(parent_fd).unwrap();
                do_child(child_fd);
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
        Sandbox(framed_stream)
    }

    pub fn execute(
        self,
        file: String,
        args: Vec<String>,
        open_files: Vec<(String, OpenMode, Vec<RawFd>)>,
    ) -> impl Future<Item = (ExecuteResult, Sandbox), Error = io::Error> {
        let command = ExecuteCommand {
            file: file,
            args: args,
            open_files: open_files,
            cgroup_file: None,
        };
        self.call::<ExecuteResult>(Request::Execute(command))
    }

    pub fn close(self) -> impl Future<Item = (), Error = io::Error> {
        self.call::<()>(Request::Close).map(|_| ())
    }

    fn call<ResponseT: for<'a> Deserialize<'a>>(self, request: Request)
        -> impl Future<Item = (ResponseT, Sandbox), Error = io::Error> {
        WriteBincode::new(self.0).send(request)
            .and_then(|sink| {
                ReadBincode::new(sink.into_inner()).into_future()
                    .map_err(|(err, _)| {
                        io::Error::new(io::ErrorKind::InvalidData, err)
                    })
            })
            .and_then(|(maybe_response, source)| {
                match maybe_response {
                    Some(response) => {
                        Ok((response, Sandbox(source.into_inner())))
                    },
                    None => {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData, "empty response"))
                    },
                }
            })
    }
}

fn do_child(child_fd: RawFd) -> ! {
    init_sandbox();
    let mut stream = unsafe { unix::net::UnixStream::from_raw_fd(child_fd) };
    let mut buffer = [0; MAX_FRAME_LENGTH];
    loop {
        let size = stream.read_u16::<LittleEndian>().unwrap() as usize;
        assert!(size <= MAX_FRAME_LENGTH);
        stream.read_exact(&mut buffer[..size]).unwrap();
        match bincode::deserialize(&buffer[..size]).unwrap() {
            Request::Execute(command) =>
                do_execute(child_fd, command, &mut stream),
            Request::Close => do_close(&mut stream),
        };
    }
}

pub fn init_sandbox() {
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
                WaitStatus::Exited(_, status) => process::exit(status as i32),
                e => panic!("{:?}", e),
            }
        },
        unistd::ForkResult::Child => (),
    }
    // TODO(iceboy): Use tempdir.
    let mount_dir = "/tmp";
    mount::mount(Some("sandbox_root"),
                 mount_dir,
                 Some("tmpfs"),
                 mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    env::set_current_dir(mount_dir).unwrap();
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
    bind_or_link("/bin", "bin");
    bind_or_link("/etc/alternatives", "etc/alternatives");
    bind_or_link("/lib", "lib");
    bind_or_link("/lib64", "lib64");
    bind_or_link("/usr/bin", "usr/bin");
    bind_or_link("/usr/include", "usr/include");
    bind_or_link("/usr/lib", "usr/lib");
    bind_or_link("/usr/lib64", "usr/lib64");
    bind_or_link("/usr/libexec", "usr/libexec");
    bind_or_link("/usr/share", "usr/share");
    bind_or_link("/var/lib/ghc", "var/lib/ghc");
    // TODO(iceboy): in & out dir.
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

fn bind_or_link(source: &str, target: &str) {
    let file_type = match fs::symlink_metadata(source) {
        Ok(attr) => attr.file_type(),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => panic!("{:?}", e),
    };
    if file_type.is_dir() {
        fs::create_dir_all(target).unwrap();
        mount::mount(Some(source),
                     target,
                     None as Option<&[u8]>,
                     mount::MS_BIND | mount::MS_REC | mount::MS_NOSUID,
                     None as Option<&[u8]>).unwrap();
        mount::mount(Some(source),
                     target,
                     None as Option<&[u8]>,
                     mount::MS_BIND | mount::MS_REMOUNT | mount::MS_RDONLY |
                     mount::MS_REC | mount::MS_NOSUID,
                     None as Option<&[u8]>).unwrap();
    } else if file_type.is_symlink() {
        let link = fs::read_link(source).unwrap();
        unix::fs::symlink(link, target).unwrap();
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
            for (file, open_mode, target_fds) in command.open_files {
                let flag = match open_mode {
                    OpenMode::Read => fcntl::O_RDONLY,
                    OpenMode::Write => fcntl::O_WRONLY,
                };
                let source_fd = fcntl::open(
                    file.as_str(), flag, stat::Mode::empty()).unwrap();
                for target_fd in &target_fds {
                    unistd::dup2(source_fd, *target_fd).unwrap();
                }
                if target_fds.into_iter().any(|fd| fd == source_fd) {
                    unistd::close(source_fd).unwrap();
                }
            }
            let file = CString::new(command.file).unwrap();
            let args = command.args.into_iter()
                .map(|arg| CString::new(arg).unwrap())
                .collect::<Vec<_>>();
            unistd::execv(&file, &args).unwrap();
            panic!();
        },
    };
    write_response(stream, &response);
}

fn do_close(stream: &mut unix::net::UnixStream) -> ! {
    write_response(stream, &());
    process::exit(0)
}

fn write_response<ResponseT: Serialize>(
    stream: &mut unix::net::UnixStream, response: &ResponseT) {
    let size = bincode::serialized_size(response) as usize;
    assert!(size <= MAX_FRAME_LENGTH);
    stream.write_u16::<LittleEndian>(size as u16).unwrap();
    bincode::serialize_into(stream, response, Infinite).unwrap();
}
