use std::collections::VecDeque;
use futures::Future;
use futures::sync::oneshot;

pub struct Pool<T> {
    puts: Vec<T>,
    gets: VecDeque<(usize, oneshot::Sender<Vec<T>>)>,
}

pub struct WireSource<T: Protocol>(oneshot::Sender<Message<T>>);
pub struct WireSink<T: Protocol>(oneshot::Receiver<Message<T>>);

pub trait Protocol {
    type Source;
    type Sink;

    fn apply(source: Self::Source, sink: Self::Sink);
}

struct Message<T: Protocol>(T::Source, oneshot::Sender<()>);

impl<T: 'static> Pool<T> {
    pub fn new() -> Pool<T> {
        Pool { gets: VecDeque::new(), puts: Vec::new() }
    }

    pub fn put(&mut self, item: T) {
        self.puts.push(item);
        self.do_marry();
    }

    pub fn get(&mut self, amt: usize)
        -> Box<Future<Item = Vec<T>, Error = ()>> {
        let (tx, rx) = oneshot::channel();
        self.gets.push_back((amt, tx));
        self.do_marry();
        Box::new(rx.map_err(|_| panic!()))
    }

    fn do_marry(&mut self) {
        match self.gets.pop_front() {
            Some((amt, tx)) => {
                if self.puts.len() >= amt {
                    let at = self.puts.len() - amt;
                    tx.send(self.puts.split_off(at))
                        .unwrap_or_else(|_| panic!());
                } else {
                    self.gets.push_front((amt, tx));
                }
            },
            None => (),
        }
    }
}

pub fn new_wire<T: Protocol>() -> (WireSource<T>, WireSink<T>) {
    let (tx, rx) = oneshot::channel();
    (WireSource::<T>(tx), WireSink::<T>(rx))
}

impl<T: Protocol> WireSource<T> {
    pub fn ready(self, source: T::Source)
        -> Box<Future<Item = (), Error = ()>> {
        let (tx, rx) = oneshot::channel();
        self.0.send(Message(source, tx)).unwrap_or_else(|_| panic!());
        Box::new(rx.map_err(|_| panic!()))
    }
}

impl<T: Protocol + 'static> WireSink<T> {
    pub fn ready(self, sink: T::Sink)
        -> Box<Future<Item = (), Error = ()>> {
        let future = self.0.map_err(|_| panic!()).and_then(move |message| {
            T::apply(message.0, sink);
            message.1.send(()).unwrap();
            Ok(())
        });
        Box::new(future)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Arc;
    use tokio_core::reactor::Core;

    #[test]
    fn pool_one() {
        let mut pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let mut core = Core::new().unwrap();
        let result = core.run(pool.get(1)).unwrap();
        assert_eq!(result, vec!["B"]);
    }

    #[test]
    fn pool_two() {
        let mut pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let mut core = Core::new().unwrap();
        let result = core.run(pool.get(2)).unwrap();
        assert_eq!(result, vec!["A", "B"]);
    }

    #[test]
    fn wire() {
        struct TestProtocol;

        impl Protocol for TestProtocol {
            type Source = i32;
            type Sink = Arc<RefCell<i32>>;

            fn apply(source: i32, sink: Arc<RefCell<i32>>) {
                *sink.borrow_mut() = source;
            }
        }

        let (wsource, wsink) = new_wire::<TestProtocol>();
        let mut core = Core::new().unwrap();
        let sink = Arc::new(RefCell::new(0));
        let future = wsource.ready(1).join(wsink.ready(sink.clone()));
        let result = core.run(future).unwrap();
        assert_eq!(*sink.borrow(), 1);
    }
}
