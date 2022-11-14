use std::{
    cell::RefCell,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream, ToSocketAddrs},
    os::unix::prelude::AsRawFd,
    rc::{Rc, Weak},
    task::Poll,
};

use futures::{AsyncRead, AsyncWrite, Stream};
use socket2::{Domain, Protocol, Socket, Type};

use crate::{reactor::get_reactor, reactor::Reactor};

#[derive(Debug)]
pub struct TcpListener {
    reactor: Weak<RefCell<Reactor>>,
    listener: StdTcpListener,
}

impl TcpListener {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, io::Error> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "empty address"))?;

        let domain = if addr.is_ipv6() {
            Domain::IPV6
        } else {
            Domain::IPV4
        };
        let sk = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        let addr = socket2::SockAddr::from(addr);
        sk.set_nonblocking(true)?;
        sk.set_reuse_address(true)?;
        sk.bind(&addr)?;
        sk.listen(1024)?;

        // add fd to reactor
        let reactor = get_reactor();
        reactor.borrow_mut().add(sk.as_raw_fd());

        println!("[listener] bind fd {}", sk.as_raw_fd());
        Ok(Self {
            reactor: Rc::downgrade(&reactor),
            listener: sk.into(),
        })
    }
}

impl Stream for TcpListener {
    type Item = std::io::Result<(TcpStream, SocketAddr)>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let fd = self.listener.as_raw_fd();
        match self.listener.accept() {
            Ok((stream, addr)) => {
                println!("[listener] fd {}; accepted from {}", fd, addr);

                Poll::Ready(Some(Ok((stream.into(), addr))))
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                println!("[listener] fd {}; WouldBlock", fd);
                // register read interest to reactor
                let reactor = self.reactor.upgrade().unwrap();
                reactor
                    .borrow_mut()
                    .set_readable(self.listener.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => {
                println!("[listener] fd {}; err", fd);
                std::task::Poll::Ready(Some(Err(e)))
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
        let reactor = get_reactor();
        reactor.borrow_mut().add(stream.as_raw_fd());
        Self { stream }
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        println!("drop");
        let reactor = get_reactor();
        reactor.borrow_mut().delete(self.stream.as_raw_fd());
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let fd = self.stream.as_raw_fd();
        match self.stream.read(buf) {
            Ok(n) => {
                println!("[stream] read fd {}; len {}", fd, n);
                Poll::Ready(Ok(n))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                println!("[stream] read fd {}; WouldBlock", fd);
                // register read interest to reactor
                let reactor = get_reactor();
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
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let fd = self.stream.as_raw_fd();
        match self.stream.write(buf) {
            Ok(n) => {
                println!("[stream] wrote fd {}; len {}", fd, n);
                Poll::Ready(Ok(n))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                println!("[stream] wrote fd {}; WouldBlock", fd);
                // register write interest to reactor
                let reactor = get_reactor();
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

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        self.stream.shutdown(std::net::Shutdown::Write)?;
        Poll::Ready(Ok(()))
    }
}
