use thiserror::Error;

#[derive(Error, Debug)]
pub enum LiushuError {
    #[error("{0}")]
    Other(String),
}

impl From<csv::Error> for LiushuError {
    fn from(value: csv::Error) -> Self {
        LiushuError::Other(format!("csv error: {}", value))
    }
}

impl From<redb::Error> for LiushuError {
    fn from(value: redb::Error) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<redb::StorageError> for LiushuError {
    fn from(value: redb::StorageError) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<redb::TableError> for LiushuError {
    fn from(value: redb::TableError) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<redb::TransactionError> for LiushuError {
    fn from(value: redb::TransactionError) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<redb::CommitError> for LiushuError {
    fn from(value: redb::CommitError) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<redb::DatabaseError> for LiushuError {
    fn from(value: redb::DatabaseError) -> Self {
        LiushuError::Other(format!("redb error: {}", value))
    }
}

impl From<bincode_next::error::EncodeError> for LiushuError {
    fn from(value: bincode_next::error::EncodeError) -> Self {
        LiushuError::Other(format!("bincode encode error: {}", value))
    }
}

impl From<bincode_next::error::DecodeError> for LiushuError {
    fn from(value: bincode_next::error::DecodeError) -> Self {
        LiushuError::Other(format!("bincode decode error: {}", value))
    }
}

impl From<std::io::Error> for LiushuError {
    fn from(value: std::io::Error) -> Self {
        LiushuError::Other(format!("io error: {}", value))
    }
}
