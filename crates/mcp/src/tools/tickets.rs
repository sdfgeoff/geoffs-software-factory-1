use std::collections::BTreeSet;

use multi_agent_board_protocol::{
    ClaimTicketRequest, CreateTicketRequest, Patch, RequestChangesRequest, ReserveTicketsRequest,
    ReviewRevisionRequest, Ticket, TicketActivity, TicketClaimResponse, TicketState,
    UpdateTicketRequest,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};

use super::EmptyArguments;
use crate::{
    McpError,
    tool::{ToolDefinition, ToolRegistry},
};

#[derive(Deserialize, JsonSchema)]
struct CreateTicketArguments {
    project_id: Option<i64>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    blocker_ids: Vec<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct TicketIdArguments {
    ticket_id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct ReserveTicketsArguments {
    #[schemars(length(min = 1), extend("uniqueItems" = true))]
    ticket_ids: Vec<i64>,
}

#[derive(Default)]
enum OptionalField<T> {
    #[default]
    Missing,
    Value(Option<T>),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OptionalField<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Self::Value)
    }
}

impl<T> From<OptionalField<T>> for Patch<T> {
    fn from(field: OptionalField<T>) -> Self {
        match field {
            OptionalField::Missing => Self::Missing,
            OptionalField::Value(None) => Self::Null,
            OptionalField::Value(Some(value)) => Self::Value(value),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EditableTicketState {
    Todo,
    InProgress,
}

impl From<EditableTicketState> for TicketState {
    fn from(state: EditableTicketState) -> Self {
        match state {
            EditableTicketState::Todo => Self::Todo,
            EditableTicketState::InProgress => Self::InProgress,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct SetTicketDetailsArguments {
    ticket_id: i64,
    title: Option<String>,
    description: Option<String>,
    state: Option<EditableTicketState>,
    #[serde(default)]
    #[schemars(with = "Option<i64>")]
    assignee_id: OptionalField<i64>,
    blocker_ids: Option<Vec<i64>>,
    expected_revision: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct TicketRevisionArguments {
    ticket_id: i64,
    expected_revision: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct TicketChangesArguments {
    ticket_id: i64,
    feedback: String,
    expected_revision: Option<i64>,
}

fn ensure_unique_ticket_ids(ticket_ids: &[i64]) -> Result<(), McpError> {
    if ticket_ids.is_empty() || ticket_ids.iter().collect::<BTreeSet<_>>().len() != ticket_ids.len()
    {
        return Err(McpError::bad_request(
            "invalid tool arguments: ticket_ids must be non-empty and unique",
        ));
    }
    Ok(())
}

pub fn register(registry: &mut ToolRegistry) -> Result<(), McpError> {
    registry.register(ToolDefinition::<EmptyArguments, Vec<Ticket>>::new(
        "list_tickets",
        "List all board tickets",
        |board, _| Box::pin(async move { board.list_tickets().await }),
    ))?;
    registry.register(ToolDefinition::<CreateTicketArguments, Ticket>::new(
        "create_ticket",
        "Create a board ticket",
        |board, arguments| {
            Box::pin(async move {
                board
                    .create_ticket(&CreateTicketRequest {
                        project_id: arguments.project_id,
                        title: arguments.title,
                        description: arguments.description,
                        blocker_ids: arguments.blocker_ids,
                    })
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<TicketIdArguments, Ticket>::new(
        "get_ticket_details",
        "Get one ticket",
        |board, arguments| Box::pin(async move { board.ticket(arguments.ticket_id).await }),
    ))?;
    registry.register(
        ToolDefinition::<TicketIdArguments, Vec<TicketActivity>>::new(
            "get_ticket_activity",
            "List a ticket's activity log",
            |board, arguments| {
                Box::pin(async move { board.ticket_activity(arguments.ticket_id).await })
            },
        ),
    )?;
    registry.register(ToolDefinition::<ReserveTicketsArguments, Vec<Ticket>>::new(
        "reserve_tickets",
        "Reserve todo tickets for the calling agent without claiming them",
        |board, arguments| {
            Box::pin(async move {
                ensure_unique_ticket_ids(&arguments.ticket_ids)?;
                board
                    .reserve_tickets(&ReserveTicketsRequest {
                        ticket_ids: arguments.ticket_ids,
                        agent_id: None,
                        administrative_override: false,
                    })
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<SetTicketDetailsArguments, Ticket>::new(
        "set_ticket_details",
        "Update ticket fields",
        |board, arguments| {
            Box::pin(async move {
                board
                    .update_ticket(
                        arguments.ticket_id,
                        &UpdateTicketRequest {
                            title: arguments.title,
                            description: arguments.description,
                            state: arguments.state.map(Into::into),
                            assignee_id: arguments.assignee_id.into(),
                            reserved_for_agent_id: Patch::Missing,
                            blocker_ids: arguments.blocker_ids,
                            expected_revision: arguments.expected_revision,
                        },
                    )
                    .await
            })
        },
    ))?;
    registry.register(
        ToolDefinition::<TicketRevisionArguments, TicketClaimResponse>::new(
            "claim_ticket",
            "Atomically claim a ticket for this agent",
            |board, arguments| {
                Box::pin(async move {
                    board
                        .claim_ticket(
                            arguments.ticket_id,
                            &ClaimTicketRequest {
                                agent_id: None,
                                expected_revision: arguments.expected_revision,
                            },
                        )
                        .await
                })
            },
        ),
    )?;
    registry.register(ToolDefinition::<TicketRevisionArguments, Ticket>::new(
        "submit_ticket_for_review",
        "Submit this agent's ticket commit for review",
        |board, arguments| {
            Box::pin(async move {
                board
                    .submit_review(
                        arguments.ticket_id,
                        &ReviewRevisionRequest {
                            expected_revision: arguments.expected_revision,
                        },
                    )
                    .await
            })
        },
    ))?;
    registry.register(
        ToolDefinition::<TicketRevisionArguments, TicketClaimResponse>::new(
            "claim_ticket_review",
            "Claim a review; requires approve_review scope",
            |board, arguments| {
                Box::pin(async move {
                    board
                        .claim_review(
                            arguments.ticket_id,
                            &ReviewRevisionRequest {
                                expected_revision: arguments.expected_revision,
                            },
                        )
                        .await
                })
            },
        ),
    )?;
    registry.register(ToolDefinition::<TicketChangesArguments, Ticket>::new(
        "request_ticket_changes",
        "Return a claimed review to its worker with feedback; requires approve_review scope",
        |board, arguments| {
            Box::pin(async move {
                board
                    .request_changes(
                        arguments.ticket_id,
                        &RequestChangesRequest {
                            feedback: arguments.feedback,
                            expected_revision: arguments.expected_revision,
                        },
                    )
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<TicketRevisionArguments, Ticket>::new(
        "approve_ticket_review",
        "Approve and merge a claimed review; requires approve_review scope",
        |board, arguments| {
            Box::pin(async move {
                board
                    .approve_review(
                        arguments.ticket_id,
                        &ReviewRevisionRequest {
                            expected_revision: arguments.expected_revision,
                        },
                    )
                    .await
            })
        },
    ))
}
