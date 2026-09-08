use multi_agent_board_protocol::{Reminder, ScheduleSelfReminderRequest};
use schemars::JsonSchema;
use serde::Deserialize;

use super::EmptyArguments;
use crate::{
    McpError,
    tool::{ToolDefinition, ToolRegistry},
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend(
    "oneOf" = [
        {"required": ["deliver_at"]},
        {"required": ["delay_seconds"]}
    ]
))]
struct ScheduleReminderArguments {
    #[schemars(length(min = 1, max = 50000))]
    message: String,
    #[schemars(extend("format" = "date-time"))]
    deliver_at: Option<String>,
    #[schemars(range(min = 1, max = 31536000))]
    delay_seconds: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReminderIdArguments {
    reminder_id: i64,
}

fn validate(arguments: &ScheduleReminderArguments) -> Result<(), McpError> {
    if arguments.message.is_empty() || arguments.message.len() > 50_000 {
        return Err(McpError::bad_request(
            "invalid tool arguments: message length is out of range",
        ));
    }
    if arguments.deliver_at.is_some() == arguments.delay_seconds.is_some() {
        return Err(McpError::bad_request(
            "invalid tool arguments: provide exactly one of deliver_at or delay_seconds",
        ));
    }
    if arguments
        .delay_seconds
        .is_some_and(|delay| !(1..=31_536_000).contains(&delay))
    {
        return Err(McpError::bad_request(
            "invalid tool arguments: delay_seconds is out of range",
        ));
    }
    Ok(())
}

pub fn register(registry: &mut ToolRegistry) -> Result<(), McpError> {
    registry.register(ToolDefinition::<ScheduleReminderArguments, Reminder>::new(
        "schedule_self_reminder",
        "Schedule a durable one-shot reminder for this agent",
        |board, arguments| {
            Box::pin(async move {
                validate(&arguments)?;
                board
                    .schedule_reminder(&ScheduleSelfReminderRequest {
                        message: arguments.message,
                        deliver_at: arguments.deliver_at,
                        delay_seconds: arguments.delay_seconds,
                    })
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<EmptyArguments, Vec<Reminder>>::new(
        "list_my_reminders",
        "List this agent's reminders",
        |board, _| Box::pin(async move { board.reminders().await }),
    ))?;
    registry.register(ToolDefinition::<ReminderIdArguments, Reminder>::new(
        "cancel_my_reminder",
        "Cancel one of this agent's scheduled reminders",
        |board, arguments| {
            Box::pin(async move { board.cancel_reminder(arguments.reminder_id).await })
        },
    ))
}
