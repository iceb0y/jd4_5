use std::io::{self, Read, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixStream;
use std::process::{self, Command};
use std::result;
use bincode::{self, Bounded};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use futures::{Future, Stream};
use futures::sink::Sink;
use nix;
use nix::unistd;
use nix::sys::socket;
use tokio_core::reactor::Handle;
use tokio_io::codec::length_delimited;
use tokio_serde_bincode::{ReadBincode, WriteBincode};
use tokio_uds;

pub struct Subprocess {
    stream: tokio_uds::UnixStream,
}

#[derive(Debug)]
pub struct Error;

impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self { Error }
}

impl From<nix::Error> for Error {
    fn from(_: nix::Error) -> Self { Error }
}

pub type Result<T> = result::Result<T, Error>;

#[derive(Serialize, Deserialize, Debug)]
enum Request {
    Backdoor,
    Close,
}

#[derive(Serialize, Deserialize, Debug)]
enum Response {
    Backdoor,
    Close,
}

const LENGTH_FIELD_LENGTH: usize = 2;
const MAX_FRAME_LENGTH: usize = 4096;

impl Subprocess {
    // TODO(iceboy): close existing fds
    pub fn new(handle: &Handle) -> Result<Subprocess> {
        let (parent_fd, child_fd) = socket::socketpair(
            socket::AddressFamily::Unix,
            socket::SockType::Stream,
            0,
            socket::SockFlag::empty())?;
        match unistd::fork()? {
            unistd::ForkResult::Parent { .. } => {
                unistd::close(child_fd)?;
            },
            unistd::ForkResult::Child => {
                unistd::close(parent_fd).unwrap();
                handle_child(unsafe { UnixStream::from_raw_fd(child_fd) });
            },
        };
        let stream = tokio_uds::UnixStream::from_stream(
            unsafe { UnixStream::from_raw_fd(parent_fd) },
            handle)?;
        Ok(Subprocess { stream: stream })
    }

    fn call(self, request: Request)
        -> impl Future<Item = (Response, Subprocess), Error = io::Error> {
        let Subprocess { stream } = self;
        let framed = length_delimited::Builder::new()
            .little_endian()
            .length_field_length(LENGTH_FIELD_LENGTH)
            .max_frame_length(MAX_FRAME_LENGTH)
            .new_framed(stream);
        let (sink, source) = framed.split();
        let rsink = WriteBincode::new(sink);
        rsink.send(request)
            .and_then(move |rsink| {
                let rsource = ReadBincode::new(source);
                (
                    Ok(rsink),
                    rsource.into_future().map_err(|(err, _)| {
                        io::Error::new(io::ErrorKind::InvalidData, err)
                    }),
                )
            })
            .and_then(move |(rsink, (maybe_response, rsource))| {
                match maybe_response {
                    Some(response) => {
                        let source = rsource.into_inner();
                        let sink = rsink.into_inner();
                        let framed = source.reunite(sink).unwrap();
                        let stream = framed.into_inner();
                        Ok((response, Subprocess { stream: stream }))
                    },
                    None => {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData, "empty response"))
                    },
                }
            })
    }

    pub fn backdoor(self) -> impl Future<Item = Subprocess, Error = io::Error> {
        self.call(Request::Backdoor)
            .and_then(|(_, subprocess)| {
                // TODO(iceboy): Check response.
                Ok(subprocess)
            })
    }

    pub fn close(self) -> impl Future<Item = (), Error = io::Error> {
        self.call(Request::Close)
            .and_then(|(_, _)| {
                // TODO(iceboy): Check response.
                Ok(())
            })
    }
}

fn handle_child(mut stream: UnixStream) -> ! {
    let mut closed = false;
    let mut buffer = Vec::new();
    buffer.reserve_exact(MAX_FRAME_LENGTH);
    while !closed {
        let size = stream.read_u16::<LittleEndian>().unwrap() as usize;
        buffer.resize(size, 0);
        stream.read_exact(&mut buffer).unwrap();
        let request = bincode::deserialize(&buffer).unwrap();
        let response = match request {
            Request::Backdoor => handle_backdoor(),
            Request::Close => {
                closed = true;
                Response::Close
            }
        };
        buffer.resize(0, 0);
        bincode::serialize_into(
            &mut buffer, &response, Bounded(MAX_FRAME_LENGTH as u64)).unwrap();
        stream.write_u16::<LittleEndian>(buffer.len() as u16).unwrap();
        stream.write_all(&buffer).unwrap();
    }
    process::exit(0)
}

fn handle_backdoor() -> Response {
    // TODO(iceboy): Send back errors.
    Command::new("bash").spawn().unwrap().wait().unwrap();
    Response::Backdoor
}
