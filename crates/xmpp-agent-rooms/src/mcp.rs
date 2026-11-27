//! The MCP server: the same actions as the CLI, over stdio. It lists and runs nothing without the `rooms` capability.

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

use crate::client::Transport;
use crate::errors::RoomsError;
use crate::tools::{Action, Rooms};

/// The MCP handler. Without actions (the `rooms` capability is not granted) it lists no tools and refuses every call.
#[derive(Debug)]
pub struct RoomsServer<T> {
    rooms: Option<Arc<Rooms<T>>>,
}

impl<T> RoomsServer<T> {
    /// Serve these actions.
    pub fn new(rooms: Rooms<T>) -> Self {
        Self { rooms: Some(Arc::new(rooms)) }
    }

    /// Serve nothing: no tools listed, every call refused.
    pub fn disabled() -> Self {
        Self { rooms: None }
    }
}

fn schema(properties: &[(&str, &str)]) -> Arc<Map<String, Value>> {
    let props: Map<String, Value> = properties.iter().map(|(name, text)| ((*name).to_owned(), json!({"type": "string", "description": text}))).collect();
    let required: Vec<&str> = properties.iter().map(|(name, _)| *name).collect();
    let Value::Object(map) = json!({"type": "object", "properties": props, "required": required}) else { unreachable!("a json! object literal") };
    Arc::new(map)
}

/// The tool definitions.
pub fn tool_list() -> Vec<Tool> {
    vec![
        Tool::new("room_create", "Create a chat room, or report that it exists.", schema(&[("name", "Room name, [a-z0-9_]{1,32}")])),
        Tool::new(
            "room_assign",
            "Make a JID a member of a room.",
            schema(&[("room", "Room name"), ("jid", "Bare JID of the member")]),
        ),
        Tool::new(
            "room_unassign",
            "Remove a JID's membership of a room.",
            schema(&[("room", "Room name"), ("jid", "Bare JID of the member")]),
        ),
    ]
}

fn text_arg(arguments: &Option<Map<String, Value>>, key: &str) -> Option<String> {
    arguments.as_ref()?.get(key)?.as_str().map(str::to_owned)
}

impl<T: Transport + Send + Sync + 'static> ServerHandler for RoomsServer<T> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("xmpp-agent-rooms", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tool_list()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let Some(rooms) = &self.rooms else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(RoomsError::Disabled.to_string())]).into());
        };
        let args = &request.arguments;
        let action = match request.name.as_ref() {
            "room_create" => text_arg(args, "name").map(|name| Action::Create { name }),
            "room_assign" => text_arg(args, "room").zip(text_arg(args, "jid")).map(|(room, jid)| Action::Assign { room, jid }),
            "room_unassign" => text_arg(args, "room").zip(text_arg(args, "jid")).map(|(room, jid)| Action::Unassign { room, jid }),
            _ => return Err(McpError::invalid_params("unknown tool", None)),
        };
        let Some(action) = action else {
            return Ok(CallToolResult::error(vec![ContentBlock::text("missing or non-string argument")]).into());
        };
        let rooms = Arc::clone(rooms);
        let outcome = tokio::task::spawn_blocking(move || rooms.run(&action)).await;
        Ok(match outcome {
            Ok(Ok(line)) => CallToolResult::success(vec![ContentBlock::text(line)]),
            Ok(Err(e)) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
            Err(_) => CallToolResult::error(vec![ContentBlock::text("the action did not finish")]),
        }
        .into())
    }
}
