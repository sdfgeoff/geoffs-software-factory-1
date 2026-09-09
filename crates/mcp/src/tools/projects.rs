use multi_agent_board_protocol::{
    Project, ProjectCompletionWorkflow, RegisterProjectRequest, SetProjectCompletionWorkflowRequest,
};
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
    #[serde(default)]
    completion_workflow: CompletionWorkflow,
}

#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CompletionWorkflow {
    #[default]
    BoardManaged,
    External,
}

impl From<CompletionWorkflow> for ProjectCompletionWorkflow {
    fn from(value: CompletionWorkflow) -> Self {
        match value {
            CompletionWorkflow::BoardManaged => Self::BoardManaged,
            CompletionWorkflow::External => Self::External,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectIdArguments {
    project_id: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetProjectCompletionWorkflowArguments {
    project_id: i64,
    completion_workflow: CompletionWorkflow,
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
                        completion_workflow: arguments.completion_workflow.into(),
                    })
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<
        SetProjectCompletionWorkflowArguments,
        Project,
    >::new(
        "set_project_completion_workflow",
        "Set whether future reviews for a project are merged by the board or externally",
        |board, arguments| {
            Box::pin(async move {
                board
                    .set_project_completion_workflow(
                        arguments.project_id,
                        &SetProjectCompletionWorkflowRequest {
                            completion_workflow: arguments.completion_workflow.into(),
                        },
                    )
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
