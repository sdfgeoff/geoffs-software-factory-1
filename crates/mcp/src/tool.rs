use std::{future::Future, marker::PhantomData, pin::Pin};

use schemars::{JsonSchema, schema_for};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{BoardClient, McpError};

pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, McpError>> + Send + 'a>>;

pub trait Tool: Send + Sync + 'static {
    type Arguments: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize;

    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn execute<'a>(
        &'a self,
        board: &'a BoardClient,
        arguments: Self::Arguments,
    ) -> ToolFuture<'a, Self::Output>;
}

type Execute<A, O> = dyn for<'a> Fn(&'a BoardClient, A) -> ToolFuture<'a, O> + Send + Sync;

pub struct ToolDefinition<A, O> {
    name: &'static str,
    description: &'static str,
    execute: Box<Execute<A, O>>,
    marker: PhantomData<fn(A) -> O>,
}

impl<A, O> ToolDefinition<A, O> {
    pub fn new(
        name: &'static str,
        description: &'static str,
        execute: impl for<'a> Fn(&'a BoardClient, A) -> ToolFuture<'a, O> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            description,
            execute: Box::new(execute),
            marker: PhantomData,
        }
    }
}

impl<A, O> Tool for ToolDefinition<A, O>
where
    A: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + 'static,
{
    type Arguments = A;
    type Output = O;

    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn execute<'a>(
        &'a self,
        board: &'a BoardClient,
        arguments: Self::Arguments,
    ) -> ToolFuture<'a, Self::Output> {
        (self.execute)(board, arguments)
    }
}

trait ErasedTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn definition(&self) -> Value;
    fn call<'a>(&'a self, board: &'a BoardClient, arguments: Value) -> ToolFuture<'a, Value>;
}

impl<T: Tool> ErasedTool for T {
    fn name(&self) -> &'static str {
        Tool::name(self)
    }

    fn definition(&self) -> Value {
        json!({
            "name": Tool::name(self),
            "description": self.description(),
            "inputSchema": schema_for!(T::Arguments),
        })
    }

    fn call<'a>(&'a self, board: &'a BoardClient, arguments: Value) -> ToolFuture<'a, Value> {
        Box::pin(async move {
            let arguments = serde_json::from_value(arguments).map_err(|error| {
                McpError::bad_request(format!("invalid tool arguments: {error}"))
            })?;
            serde_json::to_value(self.execute(board, arguments).await?)
                .map_err(|error| McpError::internal(format!("invalid typed response: {error}")))
        })
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn ErasedTool>>,
}

impl ToolRegistry {
    pub fn register<T: Tool>(&mut self, tool: T) -> Result<(), McpError> {
        if self
            .tools
            .iter()
            .any(|existing| existing.name() == tool.name())
        {
            return Err(McpError::bad_request(format!(
                "duplicate tool: {}",
                tool.name()
            )));
        }
        self.tools.push(Box::new(tool));
        Ok(())
    }

    pub fn definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    pub async fn call(
        &self,
        board: &BoardClient,
        name: &str,
        arguments: Value,
    ) -> Result<Value, McpError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == name)
            .ok_or_else(|| McpError::bad_request(format!("unknown tool: {name}")))?;
        tool.call(board, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    #[derive(Deserialize, JsonSchema)]
    struct EchoArguments {
        message: String,
        mode: Mode,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        Quiet,
        Loud,
    }

    #[derive(Serialize)]
    struct EchoOutput {
        message: String,
    }

    fn echo() -> ToolDefinition<EchoArguments, EchoOutput> {
        ToolDefinition::new(
            "echo",
            "Echo a message",
            |_board, arguments: EchoArguments| {
                Box::pin(async move {
                    let message = match arguments.mode {
                        Mode::Quiet => arguments.message,
                        Mode::Loud => arguments.message.to_uppercase(),
                    };
                    Ok(EchoOutput { message })
                })
            },
        )
    }

    #[tokio::test]
    async fn one_registration_drives_schema_and_calls() {
        let mut registry = ToolRegistry::default();
        registry.register(echo()).unwrap();
        let definition = &registry.definitions()[0];
        assert_eq!(definition["name"], "echo");
        assert_eq!(definition["description"], "Echo a message");
        assert_eq!(
            definition["inputSchema"]["required"],
            json!(["message", "mode"])
        );
        assert_eq!(
            definition.pointer("/inputSchema/$defs/Mode/enum").unwrap(),
            &json!(["quiet", "loud"])
        );

        let board = BoardClient::new("http://127.0.0.1:1".into(), 1);
        let output = registry
            .call(&board, "echo", json!({"message": "hello", "mode": "loud"}))
            .await
            .unwrap();
        assert_eq!(output, json!({"message": "HELLO"}));
        assert!(registry.call(&board, "echo", json!({})).await.is_err());
        assert!(registry.call(&board, "missing", json!({})).await.is_err());
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let mut registry = ToolRegistry::default();
        registry.register(echo()).unwrap();
        assert_eq!(
            registry.register(echo()).unwrap_err().to_string(),
            "duplicate tool: echo"
        );
    }
}
