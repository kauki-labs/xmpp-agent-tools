//! Wiring: turn the flags, the environment and the config file into something that runs actions.

use crate::client::HttpTransport;
use crate::config::{Capabilities, Config, Env};
use crate::errors::RoomsError;
use crate::mcp::RoomsServer;
use crate::tools::Rooms;

/// The MCP server type served by the binary.
pub type Server = RoomsServer<HttpTransport>;

/// The actions for the CLI. Fails with [`RoomsError::Disabled`] before it reads any setting when `rooms` is not granted.
pub fn rooms(
    capabilities_flag: Option<&str>,
    env: &Env,
    file: Option<&str>,
) -> Result<Rooms<HttpTransport>, RoomsError> {
    let capabilities = Capabilities::from_sources(capabilities_flag, env.capabilities.as_deref());
    if !capabilities.allows_rooms() {
        return Err(RoomsError::Disabled);
    }
    let config = Config::load(env, file)?;
    Ok(Rooms::new(HttpTransport::new(config), capabilities))
}

/// The MCP server. Without the `rooms` capability it is a disabled server that needs no settings; with it, missing settings are an error.
pub fn mcp_server(
    capabilities_flag: Option<&str>,
    env: &Env,
    file: Option<&str>,
) -> Result<RoomsServer<HttpTransport>, RoomsError> {
    if !Capabilities::from_sources(capabilities_flag, env.capabilities.as_deref()).allows_rooms() {
        return Ok(RoomsServer::disabled());
    }
    rooms(capabilities_flag, env, file).map(RoomsServer::new)
}
