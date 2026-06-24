//! Watch-at-source redirect handler (`GET /api/titles/{id}/watch/{service}`).
//!
//! 302s to the title's page on Plex (self-hosted /web) or a streaming service
//! (MOTN link). Real URLs — machineId, internal Plex base, MOTN links — never
//! enter any client payload.

/// Browser-facing Plex base URL for building watch links. Held in app data so
/// the internal Plex address never reaches the client.
#[derive(Clone)]
pub struct WatchConfig {
    pub plex_web_url: Option<String>,
}
