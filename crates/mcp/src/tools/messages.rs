use multi_agent_board_protocol::{
    Channel, CreateChannelRequest, Message, MessagePageQuery, PostMessageRequest,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EmptyArguments;
use crate::{
    McpError,
    tool::{ToolDefinition, ToolRegistry},
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateChannelArguments {
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadChannelArguments {
    channel_name: String,
    #[schemars(range(min = 1))]
    before_id: Option<i64>,
    #[schemars(range(min = 1))]
    after_id: Option<i64>,
    #[schemars(range(min = 1, max = 100))]
    limit: Option<i64>,
}

#[derive(Serialize)]
struct ReadChannelOutput {
    channel: Channel,
    messages: Vec<Message>,
    has_more: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PostMessageArguments {
    channel_name: String,
    author: String,
    message: String,
    #[serde(default)]
    #[schemars(length(max = 5))]
    image_paths: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MaterializeAttachmentArguments {
    attachment_id: String,
}

#[derive(Serialize)]
struct MaterializedAttachment {
    path: String,
}

fn valid_page(arguments: &ReadChannelArguments) -> Result<(), McpError> {
    for (name, value, maximum) in [
        ("before_id", arguments.before_id, None),
        ("after_id", arguments.after_id, None),
        ("limit", arguments.limit, Some(100)),
    ] {
        if value.is_some_and(|value| value < 1 || maximum.is_some_and(|max| value > max)) {
            return Err(McpError::bad_request(format!(
                "invalid tool arguments: {name} is out of range"
            )));
        }
    }
    Ok(())
}

pub fn register(registry: &mut ToolRegistry) -> Result<(), McpError> {
    registry.register(ToolDefinition::<EmptyArguments, Vec<Channel>>::new(
        "list_message_board_channels",
        "List channels with descriptions and latest message times",
        |board, _| Box::pin(async move { board.list_channels().await }),
    ))?;
    registry.register(ToolDefinition::<CreateChannelArguments, Channel>::new(
        "create_message_board_channel",
        "Create a message-board channel",
        |board, arguments| {
            Box::pin(async move {
                board
                    .create_channel(&CreateChannelRequest {
                        name: arguments.name,
                        description: arguments.description,
                    })
                    .await
            })
        },
    ))?;
    registry.register(
        ToolDefinition::<ReadChannelArguments, ReadChannelOutput>::new(
            "read_message_board_channel",
            "Read a cursor-paginated message-board channel",
            |board, arguments| {
                Box::pin(async move {
                    valid_page(&arguments)?;
                    let page = board
                        .messages(
                            &arguments.channel_name,
                            &MessagePageQuery {
                                before_id: arguments.before_id,
                                after_id: arguments.after_id,
                                limit: arguments.limit,
                            },
                        )
                        .await?;
                    Ok(ReadChannelOutput {
                        channel: board.channel(&arguments.channel_name).await?,
                        messages: page.messages,
                        has_more: page.has_more,
                    })
                })
            },
        ),
    )?;
    registry.register(ToolDefinition::<PostMessageArguments, Message>::new(
        "post_to_message_board",
        "Post a message to an existing channel",
        |board, arguments| {
            Box::pin(async move {
                if arguments.image_paths.len() > 5 {
                    return Err(McpError::bad_request(
                        "a message may contain at most 5 images",
                    ));
                }
                let mut attachment_ids = Vec::with_capacity(arguments.image_paths.len());
                for path in arguments.image_paths {
                    attachment_ids.push(board.upload_attachment(&path).await?.id);
                }
                board
                    .post_message(
                        &arguments.channel_name,
                        &PostMessageRequest {
                            author: arguments.author,
                            body: arguments.message,
                            attachment_ids,
                        },
                    )
                    .await
            })
        },
    ))?;
    registry.register(ToolDefinition::<
        MaterializeAttachmentArguments,
        MaterializedAttachment,
    >::new(
        "materialize_attachment",
        "Download a received image attachment by its opaque ID to a board-managed temporary path",
        |board, arguments| {
            Box::pin(async move {
                Ok(MaterializedAttachment {
                    path: board
                        .materialize_attachment(&arguments.attachment_id)
                        .await?,
                })
            })
        },
    ))
}
