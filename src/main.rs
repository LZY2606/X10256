use std::net::SocketAddr;

use pcb_connectivity_workbench::web::build_app;

#[tokio::main]
async fn main() {
    let mut addr: SocketAddr = "127.0.0.1:5362".parse().expect("valid default addr");
    let mut db_path = "pcb-workbench.sqlite".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => {
                if let Some(v) = args.next() {
                    addr = v.parse().expect("--addr must be host:port");
                }
            }
            "--db" => {
                if let Some(v) = args.next() {
                    db_path = v;
                }
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
    }
    let app = build_app(&db_path).expect("build app");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind addr");
    println!("PCB connectivity workbench on http://{addr} (db {db_path})");
    axum::serve(listener, app).await.expect("server");
}
