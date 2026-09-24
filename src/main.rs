use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let mut addr: SocketAddr = "127.0.0.1:5362".parse().expect("valid addr");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--addr" {
            if let Some(v) = args.next() {
                addr = v.parse().expect("valid --addr socket address");
            }
        }
    }
    let path = std::env::var("PCB_DB").unwrap_or_else(|_| "pcb_connect.db".to_string());
    pcb_connect::service::run_server(addr, &path).await.expect("server runs");
}
