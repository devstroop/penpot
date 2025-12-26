//! Error types for Penpot services

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Shape not found: {0}")]
    ShapeNotFound(uuid::Uuid),

    #[error("File not found: {0}")]
    FileNotFound(uuid::Uuid),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn status_code(&self) -> u16 {
        match self {
            Error::Validation(_) => 400,
            Error::ShapeNotFound(_) | Error::FileNotFound(_) => 404,
            Error::Database(_) => 503,
            Error::Serialization(_) => 400,
            Error::Internal(_) => 500,
        }
    }
}
