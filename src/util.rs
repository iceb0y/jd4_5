use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

pub struct Pool<T> {
    tx: Sender<T>,
    rx: Mutex<Receiver<T>>,
}

impl<T> Pool<T> {
    pub fn new() -> Pool<T> {
        let (tx, rx) = mpsc::channel();
        Pool { tx, rx: Mutex::new(rx) }
    }

    pub fn put(&self, item: T) {
        self.tx.send(item).unwrap();
    }

    pub fn get_one(&self) -> T {
        let rx = self.rx.lock().unwrap();
        rx.recv().unwrap()
    }

    pub fn get_two(&self) -> (T, T) {
        let rx = self.rx.lock().unwrap();
        (rx.recv().unwrap(), rx.recv().unwrap())
    }
}

pub fn copy_dir(from: &Path, to: &Path) {
    for result in fs::read_dir(from).unwrap() {
        let entry = result.unwrap();
        let file_type = entry.file_type().unwrap();
        let inner_from = entry.path();
        let inner_to = to.join(&entry.file_name());
        if file_type.is_dir() {
            copy_dir(&inner_from, &inner_to);
        } else {
            fs::copy(&inner_from, &inner_to).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_one() {
        let pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let one = pool.get_one();
        assert_eq!(one, "A");
    }

    #[test]
    fn pool_two() {
        let pool = Pool::new();
        pool.put("A");
        pool.put("B");
        let two = pool.get_two();
        assert_eq!(two, ("A", "B"));
    }
}
