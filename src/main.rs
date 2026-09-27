mod db;
mod domain;
mod http;

use std::{env, net::SocketAddr};

use db::Database;

#[tokio::main]
async fn main() -> anyhow_lite::Result<()> {
    let mut addr: SocketAddr = "127.0.0.1:5362".parse().unwrap();
    let mut db_path = env::var("PCB_REVIEW_DB").unwrap_or_else(|_| "pcb-review.sqlite3".to_string());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => addr = args.next().expect("missing --addr value").parse().expect("invalid address"),
            "--db" => db_path = args.next().expect("missing --db value"),
            other => panic!("unknown argument: {other}"),
        }
    }

    let database = Database::open(&db_path)?;
    database.seed_standard_case()?;
    let app = http::app(database);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("PCB connectivity review listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
