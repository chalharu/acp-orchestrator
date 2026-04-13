pub mod auth;
pub mod mock_engine;
pub mod models;
pub mod server;
pub mod sessions;

pub use models::{
    CloseSessionResponse, ConversationMessage, CreateSessionResponse, ErrorResponse,
    HealthResponse, MessageRole, PromptRequest, PromptResponse, SessionHistoryResponse,
    SessionSnapshot, SessionStatus, StreamEvent, StreamEventPayload,
};
pub use server::{AppState, ServerConfig, app, serve, serve_with_shutdown};
