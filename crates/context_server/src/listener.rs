use ::serde::{Deserialize, Serialize};
use anyhow::{Context as _, Result};
use collections::HashMap;
use futures::AsyncReadExt;
use futures::stream::StreamExt;
use futures::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
    io::BufReader,
    select_biased,
};
use gpui::{App, AppContext, AsyncApp, Task};
use net::async_net::{UnixListener, UnixStream};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{json, value::RawValue};
use std::{
    any::TypeId,
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};
use util::ResultExt;

use crate::{
    client::{CspResult, RequestId, Response},
    types::{
        CallToolParams, CallToolResponse, ListToolsResponse, Request, Tool, ToolAnnotations,
        ToolResponseContent,
        requests::{CallTool, ListTools},
    },
};

pub struct McpServer {
    socket_path: PathBuf,
    tools: Rc<RefCell<HashMap<&'static str, RegisteredTool>>>,
    handlers: Rc<RefCell<HashMap<&'static str, RequestHandler>>>,
    _server_task: Task<()>,
    _connection_tasks: Rc<RefCell<Vec<Task<()>>>>,
}

struct RegisteredTool {
    tool: Tool,
    handler: ToolHandler,
}

type ToolHandler = Box<
    dyn Fn(
        Option<serde_json::Value>,
        &mut AsyncApp,
    ) -> Task<Result<ToolResponse<serde_json::Value>>>,
>;
type RequestHandler = Box<dyn Fn(RequestId, Option<Box<RawValue>>, &App) -> Task<String>>;

impl McpServer {
    pub fn new(cx: &AsyncApp) -> Task<Result<Self>> {
        let task = cx.background_spawn(async move {
            let temp_dir = tempfile::Builder::new().prefix("fanta-mcp").tempdir()?;
            let socket_path = temp_dir.path().join("mcp.sock");
            let listener = UnixListener::bind(&socket_path).context("creating mcp socket")?;

            anyhow::Ok((temp_dir, socket_path, listener))
        });

        cx.spawn(async move |cx| {
            let (temp_dir, socket_path, listener) = task.await?;
            let tools = Rc::new(RefCell::new(HashMap::default()));
            let handlers = Rc::new(RefCell::new(HashMap::default()));
            let connection_tasks = Rc::new(RefCell::new(Vec::new()));
            let server_task = cx.spawn({
                let tools = tools.clone();
                let handlers = handlers.clone();
                let connection_tasks = Rc::downgrade(&connection_tasks);
                async move |cx| {
                    while let Ok((stream, _)) = listener.accept().await {
                        let Some(connection_tasks) = connection_tasks.upgrade() else {
                            break;
                        };
                        let mut connection_tasks = connection_tasks.borrow_mut();
                        connection_tasks.retain(|task: &Task<()>| !task.is_ready());
                        connection_tasks.extend(Self::serve_connection(
                            stream,
                            tools.clone(),
                            handlers.clone(),
                            cx,
                        ));
                    }
                    drop(temp_dir)
                }
            });
            Ok(Self {
                socket_path,
                _server_task: server_task,
                _connection_tasks: connection_tasks,
                tools,
                handlers,
            })
        })
    }

    pub fn add_tool<T: McpServerTool + Clone + 'static>(&mut self, tool: T) {
        let input_schema = tool_input_schema::<T>();
        let description = schema_description(&input_schema);
        if description.is_none() {
            // An agent only learns what a tool does from this text.
            log::error!("MCP tool `{}` has no description", T::NAME);
        }
        debug_assert!(
            description.is_some(),
            "Input schema struct must include a doc comment for the tool description"
        );

        let registered_tool = RegisteredTool {
            tool: Tool {
                name: T::NAME.into(),
                title: None,
                description,
                input_schema: input_schema.into(),
                output_schema: if TypeId::of::<T::Output>() == TypeId::of::<()>() {
                    None
                } else {
                    Some(schema_generator().root_schema_for::<T::Output>().into())
                },
                annotations: Some(tool.annotations()),
            },
            handler: Box::new({
                move |input_value, cx| {
                    let input = match input_value {
                        Some(input) => serde_json::from_value(input),
                        None => serde_json::from_value(serde_json::Value::Null),
                    };

                    let tool = tool.clone();
                    match input {
                        Ok(input) => cx.spawn(async move |cx| {
                            let output = tool.run(input, cx).await?;

                            Ok(ToolResponse {
                                content: output.content,
                                structured_content: serde_json::to_value(output.structured_content)
                                    .unwrap_or_default(),
                            })
                        }),
                        Err(err) => Task::ready(Err(err.into())),
                    }
                }
            }),
        };

        self.tools.borrow_mut().insert(T::NAME, registered_tool);
    }

    pub fn handle_request<R: Request>(
        &mut self,
        f: impl Fn(R::Params, &App) -> Task<Result<R::Response>> + 'static,
    ) {
        let f = Box::new(f);
        self.handlers.borrow_mut().insert(
            R::METHOD,
            Box::new(move |req_id, opt_params, cx| {
                let result = match opt_params {
                    Some(params) => serde_json::from_str(params.get()),
                    None => serde_json::from_value(serde_json::Value::Null),
                };

                let params: R::Params = match result {
                    Ok(params) => params,
                    Err(e) => {
                        return Task::ready(
                            serde_json::to_string(&Response::<R::Response> {
                                jsonrpc: "2.0",
                                id: req_id,
                                value: CspResult::Error(Some(crate::client::Error {
                                    message: format!("{e}"),
                                    code: -32700,
                                })),
                            })
                            .unwrap(),
                        );
                    }
                };
                let task = f(params, cx);
                cx.background_spawn(async move {
                    match task.await {
                        Ok(result) => serde_json::to_string(&Response {
                            jsonrpc: "2.0",
                            id: req_id,
                            value: CspResult::Ok(Some(result)),
                        })
                        .unwrap(),
                        Err(e) => serde_json::to_string(&Response {
                            jsonrpc: "2.0",
                            id: req_id,
                            value: CspResult::Error::<R::Response>(Some(crate::client::Error {
                                message: format!("{e}"),
                                code: -32603,
                            })),
                        })
                        .unwrap(),
                    }
                })
            }),
        );
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn serve_connection(
        stream: UnixStream,
        tools: Rc<RefCell<HashMap<&'static str, RegisteredTool>>>,
        handlers: Rc<RefCell<HashMap<&'static str, RequestHandler>>>,
        cx: &mut AsyncApp,
    ) -> [Task<()>; 2] {
        let (read, write) = stream.split();
        let (incoming_tx, mut incoming_rx) = unbounded();
        let (outgoing_tx, outgoing_rx) = unbounded();

        let io_task = cx.background_spawn(async move {
            Self::handle_io(outgoing_rx, incoming_tx, write, read)
                .await
                .log_err();
        });

        let dispatch_task = cx.spawn(async move |cx| {
            let mut request_tasks = Vec::new();
            while let Some(request) = incoming_rx.next().await {
                request_tasks.retain(|task: &Task<()>| !task.is_ready());
                let Some(request_id) = request.id.clone() else {
                    continue;
                };

                if request.method == CallTool::METHOD {
                    if let Some(task) =
                        Self::handle_call_tool(request_id, request.params, &tools, &outgoing_tx, cx)
                    {
                        request_tasks.push(task);
                    }
                } else if request.method == ListTools::METHOD {
                    Self::handle_list_tools(request_id, &tools, &outgoing_tx);
                } else if let Some(handler) = handlers.borrow().get(&request.method.as_ref()) {
                    let outgoing_tx = outgoing_tx.clone();

                    let task = cx.update(|cx| handler(request_id, request.params, cx));
                    request_tasks.push(cx.spawn(async move |_| {
                        let response = task.await;
                        outgoing_tx.unbounded_send(response).log_err();
                    }));
                } else {
                    Self::send_err(
                        request_id,
                        format!("unhandled method {}", request.method),
                        &outgoing_tx,
                    );
                }
            }
            for task in request_tasks {
                task.await;
            }
        });
        [io_task, dispatch_task]
    }

    fn handle_list_tools(
        request_id: RequestId,
        tools: &Rc<RefCell<HashMap<&'static str, RegisteredTool>>>,
        outgoing_tx: &UnboundedSender<String>,
    ) {
        let response = ListToolsResponse {
            tools: tools.borrow().values().map(|t| t.tool.clone()).collect(),
            next_cursor: None,
            meta: None,
        };

        outgoing_tx
            .unbounded_send(
                serde_json::to_string(&Response {
                    jsonrpc: "2.0",
                    id: request_id,
                    value: CspResult::Ok(Some(response)),
                })
                .unwrap_or_default(),
            )
            .ok();
    }

    fn handle_call_tool(
        request_id: RequestId,
        params: Option<Box<RawValue>>,
        tools: &Rc<RefCell<HashMap<&'static str, RegisteredTool>>>,
        outgoing_tx: &UnboundedSender<String>,
        cx: &mut AsyncApp,
    ) -> Option<Task<()>> {
        let result: Result<CallToolParams, serde_json::Error> = match params.as_ref() {
            Some(params) => serde_json::from_str(params.get()),
            None => serde_json::from_value(serde_json::Value::Null),
        };

        match result {
            Ok(params) => {
                if let Some(tool) = tools.borrow().get(&params.name.as_ref()) {
                    let outgoing_tx = outgoing_tx.clone();

                    let task = (tool.handler)(params.arguments, cx);
                    Some(cx.spawn(async move |_| {
                        let response = match task.await {
                            Ok(result) => CallToolResponse {
                                content: result.content,
                                is_error: Some(false),
                                meta: None,
                                structured_content: if result.structured_content.is_null() {
                                    None
                                } else {
                                    Some(result.structured_content)
                                },
                            },
                            Err(err) => CallToolResponse {
                                content: vec![ToolResponseContent::Text {
                                    text: format!("{err:#}"),
                                }],
                                is_error: Some(true),
                                meta: None,
                                structured_content: None,
                            },
                        };

                        outgoing_tx
                            .unbounded_send(
                                serde_json::to_string(&Response {
                                    jsonrpc: "2.0",
                                    id: request_id,
                                    value: CspResult::Ok(Some(response)),
                                })
                                .unwrap_or_default(),
                            )
                            .ok();
                    }))
                } else {
                    Self::send_err(
                        request_id,
                        format!("Tool not found: {}", params.name),
                        outgoing_tx,
                    );
                    None
                }
            }
            Err(err) => {
                Self::send_err(request_id, err.to_string(), outgoing_tx);
                None
            }
        }
    }

    fn send_err(
        request_id: RequestId,
        message: impl Into<String>,
        outgoing_tx: &UnboundedSender<String>,
    ) {
        outgoing_tx
            .unbounded_send(
                serde_json::to_string(&Response::<()> {
                    jsonrpc: "2.0",
                    id: request_id,
                    value: CspResult::Error(Some(crate::client::Error {
                        message: message.into(),
                        code: -32601,
                    })),
                })
                .unwrap(),
            )
            .ok();
    }

    async fn handle_io(
        mut outgoing_rx: UnboundedReceiver<String>,
        incoming_tx: UnboundedSender<RawRequest>,
        mut outgoing_bytes: impl Unpin + AsyncWrite,
        incoming_bytes: impl Unpin + AsyncRead,
    ) -> Result<()> {
        let mut output_reader = BufReader::new(incoming_bytes);
        let mut incoming_line = String::new();
        loop {
            select_biased! {
                message = outgoing_rx.next().fuse() => {
                    if let Some(message) = message {
                        log::trace!("send: {}", &message);
                        outgoing_bytes.write_all(message.as_bytes()).await?;
                        outgoing_bytes.write_all(&[b'\n']).await?;
                        outgoing_bytes.flush().await?;
                    } else {
                        break;
                    }
                }
                bytes_read = output_reader.read_line(&mut incoming_line).fuse() => {
                    if bytes_read? == 0 {
                        drop(incoming_tx);
                        while let Some(message) = outgoing_rx.next().await {
                            outgoing_bytes.write_all(message.as_bytes()).await?;
                            outgoing_bytes.write_all(&[b'\n']).await?;
                            outgoing_bytes.flush().await?;
                        }
                        return Ok(());
                    }
                    log::trace!("recv: {}", &incoming_line);
                    match serde_json::from_str(&incoming_line) {
                        Ok(message) => {
                            incoming_tx.unbounded_send(message).log_err();
                        }
                        Err(error) => {
                            outgoing_bytes.write_all(serde_json::to_string(&json!({
                                "jsonrpc": "2.0",
                                "error": json!({
                                    "code": -32700,
                                    "message": format!("Failed to parse: {error}"),
                                }),
                                "id": null,
                            }))?.as_bytes()).await?;
                            outgoing_bytes.write_all(&[b'\n']).await?;
                            outgoing_bytes.flush().await?;
                            log::error!("failed to parse incoming message: {error}. Raw: {incoming_line}");
                        }
                    }
                    incoming_line.clear();
                }
            }
        }
        Ok(())
    }
}

fn schema_generator() -> schemars::SchemaGenerator {
    let mut settings = schemars::generate::SchemaSettings::draft07();
    settings.inline_subschemas = true;
    settings.into_generator()
}

fn tool_input_schema<T: McpServerTool>() -> schemars::Schema {
    schema_generator().root_schema_for::<T::Input>()
}

fn schema_description(schema: &schemars::Schema) -> Option<String> {
    schema
        .get("description")
        .and_then(|description| description.as_str())
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_string)
}

/// The description a tool is advertised with: its input type's doc comment.
/// Exposed so servers can test that none of their tools ships without one.
pub fn tool_description<T: McpServerTool>() -> Option<String> {
    schema_description(&tool_input_schema::<T>())
}

pub trait McpServerTool {
    type Input: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;

    const NAME: &'static str;

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations {
            title: None,
            read_only_hint: None,
            destructive_hint: None,
            idempotent_hint: None,
            open_world_hint: None,
        }
    }

    fn run(
        &self,
        input: Self::Input,
        cx: &mut AsyncApp,
    ) -> impl Future<Output = Result<ToolResponse<Self::Output>>>;
}

#[derive(Debug)]
pub struct ToolResponse<T> {
    pub content: Vec<ToolResponseContent>,
    pub structured_content: T,
}

#[derive(Debug, Serialize, Deserialize)]
struct RawRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<RequestId>,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Box<serde_json::value::RawValue>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[gpui::test]
    async fn stopping_the_server_closes_existing_client_connections(cx: &mut gpui::TestAppContext) {
        // Socket readiness comes from the kernel rather than the deterministic dispatcher.
        cx.executor().allow_parking();
        let async_cx = cx.to_async();
        let mut server = McpServer::new(&async_cx).await.expect("start MCP server");
        server.handle_request::<crate::types::requests::Ping>(|_, _| {
            Task::ready(Ok(Default::default()))
        });
        let socket = server.socket_path().to_path_buf();
        let client = cx
            .background_spawn(async move {
                let stream = UnixStream::connect(socket).await?;
                let mut client = BufReader::new(stream);
                client
                    .get_mut()
                    .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
                    .await?;
                client.get_mut().flush().await?;
                let mut response = String::new();
                client.read_line(&mut response).await?;
                let response: serde_json::Value = serde_json::from_str(&response)?;
                anyhow::ensure!(
                    response["id"] == 1,
                    "the connected client did not receive a response"
                );
                anyhow::ensure!(
                    response["result"] == json!({}),
                    "MCP ping must return an empty object"
                );
                Ok::<_, anyhow::Error>(client)
            })
            .await
            .expect("connect and ping the server");
        drop(server);
        cx.run_until_parked();
        let closed = cx
            .background_spawn(async move {
                let mut client = client;
                let mut byte = [0u8];
                client.read(&mut byte).await
            })
            .await
            .expect("read socket closure");
        assert_eq!(
            closed, 0,
            "stopping the server must disconnect existing clients"
        );
    }

    #[test]
    fn pending_responses_are_drained_after_request_eof() -> Result<()> {
        let (incoming_tx, mut incoming_rx) = unbounded();
        let (outgoing_tx, outgoing_rx) = unbounded();
        let input = futures::io::Cursor::new(
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".to_vec(),
        );
        let mut output = futures::io::Cursor::new(Vec::new());
        let (io_result, response_result) = futures::executor::block_on(async {
            futures::join!(
                McpServer::handle_io(outgoing_rx, incoming_tx, &mut output, input),
                async move {
                    let mut requests = Vec::new();
                    while let Some(request) = incoming_rx.next().await {
                        requests.push(request);
                    }
                    for request in requests {
                        outgoing_tx.unbounded_send(serde_json::to_string(&json!({
                            "jsonrpc": "2.0", "id": request.id, "result": {},
                        }))?)?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
            )
        });
        io_result?;
        response_result?;
        let bytes = output.into_inner();
        assert!(bytes.ends_with(b"\n"));
        let response: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"], json!({}));
        Ok(())
    }

    #[test]
    fn malformed_requests_return_a_framed_parse_error() -> Result<()> {
        let (incoming_tx, mut incoming_rx) = unbounded();
        let (outgoing_tx, outgoing_rx) = unbounded();
        let mut output = futures::io::Cursor::new(Vec::new());
        let result = futures::executor::block_on(async {
            futures::join!(
                McpServer::handle_io(
                    outgoing_rx,
                    incoming_tx,
                    &mut output,
                    futures::io::Cursor::new(b"not json\n".to_vec())
                ),
                async move {
                    while incoming_rx.next().await.is_some() {}
                    drop(outgoing_tx);
                }
            )
            .0
        });
        result?;
        let bytes = output.into_inner();
        assert!(bytes.ends_with(b"\n"));
        let response: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(response["error"]["code"], -32700);
        assert!(response["id"].is_null());
        Ok(())
    }
}
