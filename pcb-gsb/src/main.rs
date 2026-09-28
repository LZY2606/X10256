mod geometry;
mod model;
mod parser;
mod project;
mod seed;
mod service;
mod storage;

use crate::seed::build_seed_events;
use crate::service::{router, AppState, PROJECT_ID};
use crate::storage::Store;
use std::{env, sync::Arc};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut addr = "127.0.0.1:5362".to_string();
    let mut db_path = "pcb-gsb.db".to_string();
    let mut reseed = false;
    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--addr" => addr = args.next().unwrap_or(addr),
            "--db" => db_path = args.next().unwrap_or(db_path),
            "--reseed" => reseed = true,
            other if other.starts_with("--addr=") => addr = other["--addr=".len()..].to_string(),
            _ => {}
        }
    }

    let (mut store, recovery) = Store::open(&db_path)?;
    if let Some(rep) = &recovery {
        eprintln!("[recovery] rolled back {} half-written tx(s), removed {} orphan event(s)",
            rep.rolled_back_tx, rep.pending_events_removed);
    }

    let existing = store.list_projects()?;
    if existing.is_empty() || reseed {
        store.ensure_project(PROJECT_ID, "PCB connectivity review", 0)?;
        let events = build_seed_events(PROJECT_ID);
        store.append_events(&events, false)?;
        eprintln!("[seed] loaded {} offline case-pack events", events.len());
    }

    let state = AppState {
        store: Arc::new(Mutex::new(store)),
        recovery: Arc::new(Mutex::new(recovery)),
    };

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("[pcb-gsb] review bench on http://{addr}");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
