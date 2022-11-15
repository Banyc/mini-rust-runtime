//! Echo example.
//! Use `nc 127.0.0.1 30000` to connect.

use futures::{AsyncReadExt, AsyncWriteExt, StreamExt};
use mini_rust_runtime::executor::Executor;
use mini_rust_runtime::tcp::TcpListener;

fn main() {
    let ex = Executor::default();
    ex.block_on(serve);
}

async fn serve() {
    let mut listener = TcpListener::bind("127.0.0.1:30000").unwrap();
    while let Some(ret) = listener.next().await {
        if let Ok((mut stream, _)) = ret {
            let f = async move {
                let mut buf = [0; 4096];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(n) => {
                            if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            return;
                        }
                    }
                }
            };
            Executor::spawn(f);
        }
    }
}
