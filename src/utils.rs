use std::sync::{Mutex, Condvar, Arc};

pub fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:02X}", byte).expect("Unable to write");
    }
    s
}

pub trait CountDownLatch {
    fn count_up(&self);
    fn count_down(&self);
    fn wait_for_zero(&self);
}

pub struct USizeCountDownLatch {
    pair: Arc<(Mutex<usize>, Condvar)>
}

impl USizeCountDownLatch {
    pub fn new() -> USizeCountDownLatch {
        USizeCountDownLatch {
            pair: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }
}

impl Clone for USizeCountDownLatch {
    fn clone(&self) -> Self {
        Self { pair: self.pair.clone() }
    }
}

impl CountDownLatch for USizeCountDownLatch {

    fn count_up(&self) {
        let (lock, cvar) = &*self.pair;
        let mut counter = lock.lock().unwrap();
        *counter += 1;
        cvar.notify_one();
    }

    fn count_down(&self) {
        let (lock, cvar) = &*self.pair;
        let mut counter = lock.lock().unwrap();
        *counter -= 1;
        cvar.notify_one();
    }

    fn wait_for_zero(&self) {
        let (lock, cvar) = &*self.pair;
        let mut counter = lock.lock().unwrap();
        while *counter > 0 {
            counter = cvar.wait(counter).unwrap();
        }
    }
}