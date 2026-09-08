use multi_agent_board_protocol::{Project, RegisterProjectRequest};
use schemars::JsonSchema;
use serde::Deserialize;

use super::EmptyArguments;
use crate::{
    McpError,
    tool::{ToolDefinition, ToolRegistry},
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RegisterProjectArguments {
    name: String,
    path: String,
    default_branch: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectIdArguments {
    project_id: i64,
}

pub fn register(registry: &mut ToolRegistry) -> Result<(), McpError> {
    registry.register(ToolDefinition::<EmptyArguments, Vec<Project>>::new(
        "list_projects",
        "List registered project roots",
        |board, _| Box::pin(async move { board.list_projects().await }),
    ))?;
    registry.register(ToolDefinition::<RegisterProjectArguments, Project>::new(
        "register_project",
        "Register a Git project root",
        |board, arguments| {
            Box::pin(async move {
                board
                    .register_project(&RegisterProjectRequest {
                        name: arguments.name,
                        path: arguments.path,
                        default_branch: arguments.default_branch,
                    })
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<ProjectIdArguments, Project>::new(
        "archive_project",
        "Archive a project with no active work",
        |board, arguments| {
            Box::pin(async move { board.archive_project(arguments.project_id).await })
        },
    ))
}
