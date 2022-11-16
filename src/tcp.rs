use std::{
    cell::RefCell,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream, ToSocketAddrs},
    os::unix::prelude::AsRawFd,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, Stream};
use socket2::{Domain, Protocol, Socket, Type};

use crate::{executor::Executor, reactor::Reactor};

#[derive(Debug)]
pub struct TcpListener {
    listener: StdTcpListener,
}

impl TcpListener {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, io::Error> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "empty address"))?;

        let domain = match addr {
            SocketAddr::V4(_) => Domain::IPV4,
            SocketAddr::V6(_) => Domain::IPV6,
        };
        let sk = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        let addr = socket2::SockAddr::from(addr);
        sk.set_nonblocking(true)?;
        sk.set_reuse_address(true)?;
        sk.bind(&addr)?;
        sk.listen(1024)?;

        // add fd to reactor
        let reactor = Executor::reactor();
        reactor.borrow_mut().add(sk.as_raw_fd());

        println!("[listener] bind fd {}", sk.as_raw_fd());
        Ok(Self {
            listener: sk.into(),
        })
    }
}

impl Stream for TcpListener {
    type Item = io::Result<(TcpStream, SocketAddr)>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let fd = self.listener.as_raw_fd();
        match self.listener.accept() {
            Ok((stream, addr)) => {
                println!("[listener] fd {}; accepted from {}", fd, addr);

                Poll::Ready(Some(Ok((stream.into(), addr))))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("[listener] fd {}; WouldBlock", fd);
                // register read interest to reactor
                let reactor = Executor::reactor();
                reactor
                    .borrow_mut()
                    .set_readable(self.listener.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => {
                println!("[listener] fd {}; err", fd);
                Poll::Ready(Some(Err(e)))
            }
        }
    }
}

#[derive(Debug)]
pub struct TcpStream {
    stream: StdTcpStream,
}

impl From<StdTcpStream> for TcpStream {
    fn from(stream: StdTcpStream) -> Self {
        let reactor = Executor::reactor();
        reactor.borrow_mut().add(stream.as_raw_fd());
        Self { stream }
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        let fd = self.stream.as_raw_fd();
        println!("[stream] fd {}; drop", fd);
        let reactor = Executor::reactor();
        reactor.borrow_mut().delete(self.stream.as_raw_fd());
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let fd = self.stream.as_raw_fd();
        match self.stream.read(buf) {
            Ok(n) => {
                println!("[stream] read fd {}; len {}", fd, n);
                Poll::Ready(Ok(n))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("[stream] read fd {}; WouldBlock", fd);
                // register read interest to reactor
                let reactor = Executor::reactor();
                reactor
                    .borrow_mut()
                    .set_readable(self.stream.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => {
                println!("[stream] read fd {}; err", fd);
                Poll::Ready(Err(e))
            }
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let fd = self.stream.as_raw_fd();
        match self.stream.write(buf) {
            Ok(n) => {
                println!("[stream] wrote fd {}; len {}", fd, n);
                Poll::Ready(Ok(n))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("[stream] wrote fd {}; WouldBlock", fd);
                // register write interest to reactor
                let reactor = Executor::reactor();
                reactor
                    .borrow_mut()
                    .set_writable(self.stream.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => {
                println!("[stream] wrote fd {}; err", fd);
                Poll::Ready(Err(e))
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let fd = self.stream.as_raw_fd();
        match self.stream.flush() {
            Ok(()) => {
                println!("[stream] flushed fd {}", fd);
                Poll::Ready(Ok(()))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("[stream] flushed fd {}; WouldBlock", fd);
                // register write interest to reactor
                let reactor = Executor::reactor();
                reactor
                    .borrow_mut()
                    .set_writable(self.stream.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => {
                println!("[stream] flushed fd {}; err", fd);
                Poll::Ready(Err(e))
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let fd = self.stream.as_raw_fd();
        println!("[stream] close write fd {}", fd);
        self.stream.shutdown(std::net::Shutdown::Write)?;
        Poll::Ready(Ok(()))
    }
}
