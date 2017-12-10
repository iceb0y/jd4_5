use std;
use std::net::Shutdown;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use bincode;
use nix::unistd;
use nix::sys::socket;
use nix::sys::wait;

#[derive(Serialize, Deserialize, Debug)]
enum Command {
    Backdoor,
}

const PIPE_LIMIT: bincode::Bounded = bincode::Bounded(16384);

pub fn fork_and_communicate() {
    let (parent_fd, child_fd) = socket::socketpair(
        socket::AddressFamily::Unix,
        socket::SockType::Stream,
        0,
        socket::SockFlag::empty()).unwrap();

    match unistd::fork().unwrap() {
        unistd::ForkResult::Parent { child } => {
            unistd::close(child_fd).unwrap();
            handle_parent(
                &mut unsafe { UnixStream::from_raw_fd(parent_fd) }, child);
        },
        unistd::ForkResult::Child => {
            unistd::close(parent_fd).unwrap();
            handle_child(&mut unsafe { UnixStream::from_raw_fd(child_fd) });
        },
    }
}

fn handle_parent(writer: &mut UnixStream, child: unistd::Pid) {
    send_command(writer, &Command::Backdoor);
    writer.shutdown(Shutdown::Write).unwrap();
    wait::waitpid(child, Option::None).unwrap();
}

fn send_command(writer: &mut UnixStream, command: &Command) {
    bincode::serialize_into(writer, command, PIPE_LIMIT).unwrap();
}

fn handle_child(reader: &mut UnixStream) {
    loop {
        match bincode::deserialize_from(reader, PIPE_LIMIT) {
            Ok::<Command, _>(command) => handle_command(&command),
            Err(e) => {
                match *e {
                    bincode::ErrorKind::Io(ref e)
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                        std::process::exit(0),
                    _ => panic!("{:?}", e),
                }
            },
        };
    }
}

fn handle_command(command: &Command) {
    println!("{:?}", command);
}
