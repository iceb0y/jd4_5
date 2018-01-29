use std::collections::VecDeque;
use futures::{Future, Sink};
use futures::stream::Stream;
use futures::sync::mpsc;
use futures::sync::oneshot;
use tokio_core::reactor::Handle;

pub struct Pool<T>(mpsc::Sender<Operation<T>>);

enum Operation<T> {
    Put(T),
    Get(usize, oneshot::Sender<Vec<T>>),
}

impl<T: 'static> Pool<T> {
    pub fn new(handle: &Handle) -> Pool<T> {
        let (tx, rx) = mpsc::channel(1);
        handle.spawn(do_pipe(rx));
        Pool(tx)
    }

    pub fn put(&self, item: T) -> impl Future<Item = (), Error = ()> {
        self.0.clone().send(Operation::Put(item))
            .then(|result| { result.unwrap(); Ok(()) })
    }

    pub fn get(&self, amt: usize) -> impl Future<Item = Vec<T>, Error = ()> {
        let (tx, rx) = oneshot::channel();
        self.0.clone().send(Operation::Get(amt, tx))
            .then(|result| { result.unwrap(); rx })
            .then(|result| Ok(result.unwrap()))
    }
}

fn do_pipe<T>(rx: mpsc::Receiver<Operation<T>>)
    -> impl Future<Item = (), Error = ()> {
    let mut put_stack = Vec::new();
    let mut get_queue = VecDeque::new();
    rx.for_each(move |operation| {
        match operation {
            Operation::Put(item) => put_stack.push(item),
            Operation::Get(amt, tx) => get_queue.push_back((amt, tx)),
        }
        match get_queue.pop_front() {
            Some((amt, tx)) => {
                if put_stack.len() >= amt {
                    let at = put_stack.len() - amt;
                    tx.send(put_stack.split_off(at))
                        .unwrap_or_else(|_| panic!());
                } else {
                    get_queue.push_front((amt, tx));
                }
            },
            None => (),
        }
        Ok(())
    })
}
