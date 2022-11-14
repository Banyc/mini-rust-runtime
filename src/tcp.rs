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

        println!("tcp bind with fd {}", sk.as_raw_fd());
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
        match self.listener.accept() {
            Ok((stream, addr)) => Poll::Ready(Some(Ok((stream.into(), addr)))),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // modify reactor to register interest
                let reactor = self.reactor.upgrade().unwrap();
                reactor
                    .borrow_mut()
                    .modify_readable(self.listener.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => std::task::Poll::Ready(Some(Err(e))),
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
        unsafe {
            println!("read for fd {}", fd);
            match self.stream.read(buf) {
                Ok(n) => {
                    println!("read for fd {} done, {}", fd, n);
                    Poll::Ready(Ok(n))
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    println!("read for fd {} done WouldBlock", fd);
                    // modify reactor to register interest
                    let reactor = get_reactor();
                    reactor
                        .borrow_mut()
                        .modify_readable(self.stream.as_raw_fd(), cx);
                    Poll::Pending
                }
                Err(e) => {
                    println!("read for fd {} done err", fd);
                    Poll::Ready(Err(e))
                }
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
        match self.stream.write(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let reactor = get_reactor();
                reactor
                    .borrow_mut()
                    .modify_writable(self.stream.as_raw_fd(), cx);
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
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
