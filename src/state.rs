// author: kodeholic (powered by Claude)
//! Global application state shared across handlers

use std::sync::Arc;

use crate::room::room::RoomHub;
use crate::transport::dtls::ServerCert;

/// Shared application state (passed to all handlers via Axum's State extractor)
#[derive(Clone)]
pub struct AppState {
    pub rooms:  Arc<RoomHub>,
    pub cert:   Arc<ServerCert>,
}

impl AppState {
    pub fn new(cert: ServerCert) -> Self {
        Self {
            rooms:  Arc::new(RoomHub::new()),
            cert:   Arc::new(cert),
        }
    }
}
