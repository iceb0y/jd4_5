// TODO(iceboy): distinguish traits and other names?
use std::io::{self, Read};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::{self, Command};
use bincode;
use byteorder::{BigEndian, ReadBytesExt};
use futures::{future, Future};
use futures::Stream;
use futures::sink::Sink;
use nix::unistd;
use nix::sys::socket;
use nix::sys::wait;
use tokio_core::reactor::Handle;
use tokio_io::codec::length_delimited;
use tokio_serde_bincode::WriteBincode;
use tokio_uds;

#[derive(Serialize, Deserialize, Debug)]
enum Request {
    Backdoor,
}

pub struct Subprocess {
    sink: Box<Sink<SinkItem = Request, SinkError = io::Error>>,
    child: unistd::Pid,
}

impl Subprocess {
    // TODO(iceboy): close existing fds
    pub fn new(handle: &Handle) -> Subprocess {
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
        let stream = unsafe { UnixStream::from_raw_fd(parent_fd) };
        let async_stream =
            tokio_uds::UnixStream::from_stream(stream, handle).unwrap();
        let framed = length_delimited::Framed::new(async_stream);
        let (sink, source) = framed.split();
        drop(source);
        Subprocess {
            sink: Box::new(WriteBincode::new(sink)),
            child: child,
        }
    }

    // TODO(iceboy): replace with impl trait
    pub fn backdoor(self) -> Box<Future<Item = Subprocess, Error = io::Error>> {
        let child = self.child;
        Box::new(self.sink.send(Request::Backdoor)
            .and_then(move |sink| future::ok(Subprocess {
                sink: sink,
                child: child,
            })))
    }

    pub fn wait_close(self) {
        drop(self.sink);
        wait::waitpid(self.child, Option::None).unwrap();
    }
}

fn handle_child(mut stream: UnixStream) -> ! {
    loop {
        // TODO(iceboy): limit size?
        let size = stream.read_u32::<BigEndian>().unwrap() as usize;
        // TODO(iceboy): share buffer?
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
