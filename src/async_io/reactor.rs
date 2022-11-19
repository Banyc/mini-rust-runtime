use io_uring::IoUring;

pub struct URingReactor {
    pub ring: IoUring,
}

impl Default for URingReactor {
    fn default() -> Self {
        Self {
            ring: IoUring::new(8).unwrap(),
        }
    }
}

impl URingReactor {
    pub fn wait(&mut self) {
        self.ring.submit_and_wait(1).unwrap();
    }
}
