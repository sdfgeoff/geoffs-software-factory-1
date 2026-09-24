mod messages;
mod projects;
mod reminders;
mod subscriptions;
mod tickets;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::{McpError, tool::ToolRegistry};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EmptyArguments {}

pub fn registry() -> Result<ToolRegistry, McpError> {
    let mut registry = ToolRegistry::default();
    projects::register(&mut registry)?;
    tickets::register(&mut registry)?;
    messages::register(&mut registry)?;
    subscriptions::register(&mut registry)?;
    reminders::register(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::BoardClient;

    fn definition<'a>(definitions: &'a [Value], name: &str) -> &'a Value {
        definitions
            .iter()
            .find(|definition| definition["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
    }

    #[test]
    fn registry_advertises_every_external_tool_once() {
        let definitions = registry().unwrap().definitions();
        let mut names = definitions
            .iter()
            .map(|definition| definition["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "add_my_channel_subscriptions",
                "add_my_project_subscriptions",
                "add_ticket_comment",
                "approve_ticket_review",
                "archive_project",
                "cancel_my_reminder",
                "claim_ticket",
                "claim_ticket_review",
                "complete_external_merge",
                "create_message_board_channel",
                "create_ticket",
                "get_ticket_activity",
                "get_ticket_details",
                "list_message_board_channels",
                "list_my_channel_subscriptions",
                "list_my_project_subscriptions",
                "list_my_reminders",
                "list_projects",
                "list_tickets",
                "materialize_attachment",
                "post_to_message_board",
                "read_message_board_channel",
                "register_project",
                "remove_my_channel_subscriptions",
                "remove_my_project_subscriptions",
                "replace_my_channel_subscriptions",
                "replace_my_project_subscriptions",
                "request_ticket_changes",
                "reserve_tickets",
                "return_external_merge",
                "schedule_self_reminder",
                "set_external_change",
                "set_project_completion_workflow",
                "set_ticket_details",
                "submit_ticket_for_review",
            ]
        );
    }

    #[tokio::test]
    async fn generated_constraints_match_boundary_validation() {
        let registry = registry().unwrap();
        let definitions = registry.definitions();
        let read = definition(&definitions, "read_message_board_channel");
        assert_eq!(
            read.pointer("/inputSchema/properties/limit/minimum"),
            Some(&json!(1))
        );
        let post = definition(&definitions, "post_to_message_board");
        assert_eq!(
            post.pointer("/inputSchema/properties/image_paths/type"),
            Some(&json!("array"))
        );
        assert_eq!(
            post.pointer("/inputSchema/properties/image_paths/maxItems"),
            Some(&json!(5))
        );
        for name in ["add_ticket_comment", "request_ticket_changes"] {
            assert_eq!(
                definition(&definitions, name)
                    .pointer("/inputSchema/properties/image_paths/maxItems"),
                Some(&json!(5))
            );
        }
        assert_eq!(
            read.pointer("/inputSchema/properties/limit/maximum"),
            Some(&json!(100))
        );
        let reserve = definition(&definitions, "reserve_tickets");
        assert_eq!(
            reserve.pointer("/inputSchema/properties/ticket_ids/minItems"),
            Some(&json!(1))
        );
        assert_eq!(
            reserve.pointer("/inputSchema/properties/ticket_ids/uniqueItems"),
            Some(&json!(true))
        );
        let reminder = definition(&definitions, "schedule_self_reminder");
        assert_eq!(
            reminder.pointer("/inputSchema/oneOf"),
            Some(&json!([
                {"required": ["deliver_at"]},
                {"required": ["delay_seconds"]}
            ]))
        );

        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        assert!(
            registry
                .call(
                    &board,
                    "read_message_board_channel",
                    json!({"channel_name": "general", "limit": 101}),
                )
                .await
                .is_err()
        );
        assert!(
            registry
                .call(&board, "reserve_tickets", json!({"ticket_ids": [1, 1]}))
                .await
                .is_err()
        );
        assert!(
            registry
                .call(
                    &board,
                    "schedule_self_reminder",
                    json!({
                        "message": "wake me",
                        "deliver_at": "2026-09-08T00:00:00Z",
                        "delay_seconds": 10
                    }),
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unknown_fields_are_rejected_for_typed_and_empty_arguments() {
        let registry = registry().unwrap();
        let definitions = registry.definitions();
        for name in ["get_ticket_details", "list_projects"] {
            assert_eq!(
                definition(&definitions, name).pointer("/inputSchema/additionalProperties"),
                Some(&json!(false))
            );
        }

        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        assert!(
            registry
                .call(
                    &board,
                    "get_ticket_details",
                    json!({"ticket_id": 1, "bogus": true}),
                )
                .await
                .is_err()
        );
        assert!(
            registry
                .call(&board, "list_projects", json!({"bogus": true}))
                .await
                .is_err()
        );
    }
}
