use std::io::Read;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::{self, Command};
use bincode;
use byteorder::{BigEndian, ReadBytesExt};
use futures::Stream;
use futures::sink::Sink;
use nix::unistd;
use nix::sys::socket;
use nix::sys::wait;
use tokio_core::reactor::Core;
use tokio_io::codec::length_delimited;
use tokio_serde_bincode::WriteBincode;
use tokio_uds;

#[derive(Serialize, Deserialize, Debug)]
enum Request {
    Backdoor,
}

pub fn fork_and_communicate() {
    let (parent_fd, child_fd) = socket::socketpair(
        socket::AddressFamily::Unix,
        socket::SockType::Stream,
        0,
        socket::SockFlag::empty()).unwrap();

    let child = match unistd::fork().unwrap() {
        unistd::ForkResult::Parent { child } => {
            unistd::close(child_fd).unwrap();
            child
        },
        unistd::ForkResult::Child => {
            unistd::close(parent_fd).unwrap();
            handle_child(unsafe { UnixStream::from_raw_fd(child_fd) });
        },
    };

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let stream = tokio_uds::UnixStream::from_stream(
        unsafe { UnixStream::from_raw_fd(parent_fd) },
        &handle).unwrap();
    let framed = length_delimited::Framed::new(stream);
    let (frame_writer, frame_reader) = framed.split();
    let writer = WriteBincode::new(frame_writer);
    core.run(writer.send(&Request::Backdoor)).unwrap();
    wait::waitpid(child, Option::None).unwrap();
}

fn handle_child(mut stream: UnixStream) -> ! {
    loop {
        // TODO(iceboy): limit size?
        let size = stream.read_u32::<BigEndian>().unwrap() as usize;
        let mut buffer = vec![0; size];
        stream.read_exact(&mut buffer).unwrap();
        let request: Request = bincode::deserialize(&buffer).unwrap();
        match request {
            Request::Backdoor => handle_backdoor(),
        }
        if size == 0 { process::exit(0); }
    }
}

fn handle_backdoor() {
    Command::new("bash").spawn().unwrap().wait().unwrap();
}
