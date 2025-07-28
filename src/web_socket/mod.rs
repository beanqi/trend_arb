pub mod ws_connection;
pub mod utils;

/// websocket 帧的操作码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Ping,
    Pong,
    Text,
    Binary,
    Close,
}