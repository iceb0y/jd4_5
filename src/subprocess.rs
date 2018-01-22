use std::io::{self, Read};
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::net;
use std::process::{self, Command};
use bincode::{self, Infinite};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use futures::{Future, Stream};
use futures::sink::Sink;
use nix::unistd;
use nix::sys::socket;
use sandbox;
use serde::{Serialize, Deserialize};
use tokio_core::reactor::Handle;
use tokio_io::codec::length_delimited::{self, Framed};
use tokio_serde_bincode::{ReadBincode, WriteBincode};
use tokio_uds;

pub struct Subprocess(Framed<tokio_uds::UnixStream>);

#[derive(Serialize, Deserialize)]
enum Request {
    Backdoor,
    Close,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BackdoorResult {
    Exited(i32),
    Signaled,
    SpawnError,
    WaitError,
}

const LENGTH_FIELD_LENGTH: usize = 2;
const MAX_FRAME_LENGTH: usize = 4096;

impl Subprocess {
    // TODO(iceboy): close existing fds
    pub fn new(handle: &Handle) -> Subprocess {
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
        let net_stream = unsafe { net::UnixStream::from_raw_fd(parent_fd) };
        let async_stream = tokio_uds::UnixStream::from_stream(
            net_stream, handle).unwrap();
        let framed_stream = length_delimited::Builder::new()
            .little_endian()
            .length_field_length(LENGTH_FIELD_LENGTH)
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_framed(async_stream);
        Subprocess(framed_stream)
    }

    pub fn backdoor(self)
        -> impl Future<Item = (BackdoorResult, Subprocess), Error = io::Error> {
        self.call::<BackdoorResult>(Request::Backdoor)
    }

    pub fn close(self) -> impl Future<Item = (), Error = io::Error> {
        self.call::<()>(Request::Close).map(|_| ())
    }

    fn call<ResponseT: for<'a> Deserialize<'a>>(self, request: Request)
        -> impl Future<Item = (ResponseT, Subprocess), Error = io::Error> {
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
                        Ok((response, Subprocess(source.into_inner())))
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
    sandbox::init();
    let mut stream = unsafe { net::UnixStream::from_raw_fd(child_fd) };
    let mut buffer = [0; MAX_FRAME_LENGTH];
    loop {
        let size = stream.read_u16::<LittleEndian>().unwrap() as usize;
        assert!(size <= MAX_FRAME_LENGTH);
        stream.read_exact(&mut buffer[..size]).unwrap();
        match bincode::deserialize(&buffer[..size]).unwrap() {
            Request::Backdoor => do_backdoor(&mut stream),
            Request::Close => do_close(&mut stream),
        };
    }
}

fn do_backdoor(stream: &mut net::UnixStream) {
    let response = match Command::new("bash").spawn() {
        Ok(mut child) => match child.wait() {
            Ok(status) => match status.code() {
                Some(code) => BackdoorResult::Exited(code),
                None => BackdoorResult::Signaled,
            },
            Err(_) => BackdoorResult::WaitError,
        },
        Err(_) => BackdoorResult::SpawnError,
    };
    write_response(stream, &response);
}

fn do_close(stream: &mut net::UnixStream) -> ! {
    write_response(stream, &());
    process::exit(0)
}

fn write_response<ResponseT: Serialize>(
    stream: &mut net::UnixStream, response: &ResponseT) {
    let size = bincode::serialized_size(response) as usize;
    assert!(size <= MAX_FRAME_LENGTH);
    stream.write_u16::<LittleEndian>(size as u16).unwrap();
    bincode::serialize_into(stream, response, Infinite).unwrap();
}
