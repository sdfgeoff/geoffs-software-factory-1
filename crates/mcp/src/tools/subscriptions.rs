use multi_agent_board_protocol::{
    SetAgentChannelSubscriptionsRequest, SetAgentProjectSubscriptionsRequest,
};
use schemars::JsonSchema;
use serde::Deserialize;

use super::EmptyArguments;
use crate::{
    McpError,
    tool::{ToolDefinition, ToolRegistry},
};

#[derive(Deserialize, JsonSchema)]
struct ChannelListArguments {
    channels: Vec<String>,
}

impl From<ChannelListArguments> for SetAgentChannelSubscriptionsRequest {
    fn from(arguments: ChannelListArguments) -> Self {
        Self {
            channels: arguments.channels,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct ProjectListArguments {
    project_ids: Vec<i64>,
}

impl From<ProjectListArguments> for SetAgentProjectSubscriptionsRequest {
    fn from(arguments: ProjectListArguments) -> Self {
        Self {
            project_ids: arguments.project_ids,
        }
    }
}

pub fn register(registry: &mut ToolRegistry) -> Result<(), McpError> {
    registry.register(ToolDefinition::<EmptyArguments, Vec<String>>::new(
        "list_my_channel_subscriptions",
        "List this agent's message-board channel subscriptions",
        |board, _| Box::pin(async move { board.channel_subscriptions().await }),
    ))?;
    registry.register(ToolDefinition::<ChannelListArguments, Vec<String>>::new(
        "add_my_channel_subscriptions",
        "Add message-board channel subscriptions for this agent",
        |board, arguments| {
            Box::pin(async move { board.add_channel_subscriptions(&arguments.into()).await })
        },
    ))?;
    registry.register(ToolDefinition::<ChannelListArguments, Vec<String>>::new(
        "remove_my_channel_subscriptions",
        "Remove message-board channel subscriptions for this agent",
        |board, arguments| {
            Box::pin(async move { board.remove_channel_subscriptions(&arguments.into()).await })
        },
    ))?;
    registry.register(ToolDefinition::<ChannelListArguments, Vec<String>>::new(
        "replace_my_channel_subscriptions",
        "Replace all message-board channel subscriptions for this agent",
        |board, arguments| {
            Box::pin(async move { board.replace_channel_subscriptions(&arguments.into()).await })
        },
    ))?;
    registry.register(ToolDefinition::<EmptyArguments, Vec<i64>>::new(
        "list_my_project_subscriptions",
        "List this agent's preferred project roots",
        |board, _| Box::pin(async move { board.project_subscriptions().await }),
    ))?;
    registry.register(ToolDefinition::<ProjectListArguments, Vec<i64>>::new(
        "add_my_project_subscriptions",
        "Add preferred project roots for this agent",
        |board, arguments| {
            Box::pin(async move { board.add_project_subscriptions(&arguments.into()).await })
        },
    ))?;
    registry.register(ToolDefinition::<ProjectListArguments, Vec<i64>>::new(
        "remove_my_project_subscriptions",
        "Remove preferred project roots for this agent",
        |board, arguments| {
            Box::pin(async move { board.remove_project_subscriptions(&arguments.into()).await })
        },
    ))?;
    registry.register(ToolDefinition::<ProjectListArguments, Vec<i64>>::new(
        "replace_my_project_subscriptions",
        "Replace all preferred project roots for this agent",
        |board, arguments| {
            Box::pin(async move { board.replace_project_subscriptions(&arguments.into()).await })
        },
    ))
}
