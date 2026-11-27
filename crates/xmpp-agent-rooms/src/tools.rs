//! The three actions, shared by the CLI and the MCP server so both behave the same.

use crate::api::{self, Created};
use crate::client::Transport;
use crate::config::Capabilities;
use crate::errors::RoomsError;
use crate::validate::{Affiliation, BareJid, RoomName};

/// One action a caller can ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Create a room.
    Create {
        /// The room name.
        name: String,
    },
    /// Make a JID a member of a room.
    Assign {
        /// The room name.
        room: String,
        /// The member's bare JID.
        jid: String,
    },
    /// Remove a JID's membership of a room.
    Unassign {
        /// The room name.
        room: String,
        /// The member's bare JID.
        jid: String,
    },
}

/// The actions bound to a transport and to the granted capabilities.
#[derive(Debug)]
pub struct Rooms<T> {
    transport: T,
    capabilities: Capabilities,
}

impl<T: Transport> Rooms<T> {
    /// Bind the actions to a transport and the granted capabilities.
    pub fn new(transport: T, capabilities: Capabilities) -> Self {
        Self {
            transport,
            capabilities,
        }
    }

    /// Run one action and return a one-line result for the caller.
    pub fn run(&self, action: &Action) -> Result<String, RoomsError> {
        if !self.capabilities.allows_rooms() {
            return Err(RoomsError::Disabled);
        }
        match action {
            Action::Create { name } => {
                let name = RoomName::parse(name)?;
                let request = api::create_request(&name);
                let response = self.transport.post(&request.path, &request.body)?;
                let Created { room, created } = api::map_create(response.status, &response.body)?;
                Ok(if created {
                    format!("created {room}")
                } else {
                    format!("{room} already exists")
                })
            }
            Action::Assign { room, jid } => self.set(room, jid, Affiliation::Member, "assigned"),
            Action::Unassign { room, jid } => self.set(room, jid, Affiliation::None, "unassigned"),
        }
    }

    fn set(
        &self,
        room: &str,
        jid: &str,
        affiliation: Affiliation,
        done: &str,
    ) -> Result<String, RoomsError> {
        let room = RoomName::parse(room)?;
        let jid = BareJid::parse(jid)?;
        let request = api::assign_request(&room, &jid, affiliation);
        let response = self.transport.post(&request.path, &request.body)?;
        api::map_assign(response.status, &response.body)?;
        Ok(format!("{done} {} in {}", jid.as_str(), room.as_str()))
    }
}
