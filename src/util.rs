use std::collections::VecDeque;
use futures::Future;
use futures::sync::oneshot;

pub struct Pool<T> {
    puts: Vec<T>,
    gets: VecDeque<(usize, oneshot::Sender<Vec<T>>)>,
}

impl<T> Pool<T> {
    pub fn new() -> Pool<T> {
        Pool { gets: VecDeque::new(), puts: Vec::new() }
    }

    pub fn put(&mut self, item: T) {
        self.puts.push(item);
        self.do_marry();
    }

    pub fn get(&mut self, amt: usize)
        -> impl Future<Item = Vec<T>, Error = ()> {
        let (tx, rx) = oneshot::channel();
        self.gets.push_back((amt, tx));
        self.do_marry();
        rx.map_err(|_| panic!())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_core::reactor::Core;

    #[test]
    fn one() {
        let mut pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let mut core = Core::new().unwrap();
        let result = core.run(pool.get(1)).unwrap();
        assert_eq!(result, vec!["B"]);
    }

    #[test]
    fn two() {
        let mut pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let mut core = Core::new().unwrap();
        let result = core.run(pool.get(2)).unwrap();
        assert_eq!(result, vec!["A", "B"]);
    }
}
