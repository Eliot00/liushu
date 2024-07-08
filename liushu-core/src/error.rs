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

impl From<bincode::Error> for LiushuError {
    fn from(value: bincode::Error) -> Self {
        LiushuError::Other(format!("bincode error: {}", value))
    }
}

impl From<std::io::Error> for LiushuError {
    fn from(value: std::io::Error) -> Self {
        LiushuError::Other(format!("io error: {}", value))
    }
}
