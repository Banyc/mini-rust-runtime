use std::{
    cell::RefCell,
    os::unix::prelude::{AsRawFd, RawFd},
    rc::Rc,
    task::{Context, Waker},
};

use polling::{Event, Poller};

#[derive(Debug)]
pub struct Reactor {
    poller: Poller,
    wakers: rustc_hash::FxHashMap<usize, Waker>,

    ready: Vec<Event>,
}

impl Default for Reactor {
    fn default() -> Self {
        Self {
            poller: Poller::new().unwrap(),
            wakers: Default::default(),

            ready: Vec::with_capacity(2048),
        }
    }
}

impl Reactor {
    // Epoll related
    pub fn add(&mut self, fd: RawFd) {
        println!("[reactor] add fd {}", fd);

        self.poller.add(fd, Event::none(fd as usize)).unwrap();
    }

    pub fn set_readable(&mut self, fd: RawFd, cx: &mut Context) {
        println!(
            "[reactor] set_readable fd {}; add waker {}",
            fd,
            key_read(fd as usize)
        );

        self.add_waker(key_read(fd as usize), cx);
        let event = Event::readable(fd as usize);
        self.poller.modify(fd, event);
    }

    pub fn set_writable(&mut self, fd: RawFd, cx: &mut Context) {
        println!(
            "[reactor] set_writable fd {}; add waker {}",
            fd,
            key_write(fd as usize)
        );

        self.add_waker(key_write(fd as usize), cx);
        let event = Event::writable(fd as usize);
        self.poller.modify(fd, event);
    }

    pub fn wait(&mut self) {
        println!("[reactor] waiting");
        self.poller.wait(&mut self.ready, None);
        println!("[reactor] woken up");

        for event in self.ready.drain(..) {
            if event.readable {
                if let Some(waker) = self.wakers.remove(&key_read(event.key)) {
                    println!(
                        "[reactor wakers] fd {}; read waker {} removed and to be woken up",
                        event.key,
                        key_read(event.key)
                    );
                    waker.wake();
                }
            }
            if event.writable {
                if let Some(waker) = self.wakers.remove(&key_write(event.key)) {
                    println!(
                        "[reactor wakers] fd {}; write waker {} removed and to be woken up",
                        event.key,
                        key_write(event.key)
                    );
                    waker.wake();
                }
            }
        }
    }

    pub fn delete(&mut self, fd: RawFd) {
        println!("[reactor] delete fd {}", fd);

        self.wakers.remove(&key_read(fd as usize));
        self.wakers.remove(&key_write(fd as usize));
        println!(
            "[reactor wakers] fd {}; wakers {}, {} removed",
            fd,
            key_read(fd as usize),
            key_write(fd as usize)
        );
    }

    fn add_waker(&mut self, key: usize, cx: &mut Context) {
        println!("[reactor wakers] waker {} saved", key);

        self.wakers.insert(key, cx.waker().clone());
    }
}

impl crate::reactor::Reactor for Reactor {
    fn wait(&mut self) {
        Reactor::wait(self);
    }
}

#[inline]
fn key_read(fd: usize) -> usize {
    fd * 2
}

#[inline]
fn key_write(fd: usize) -> usize {
    fd * 2 + 1
}
