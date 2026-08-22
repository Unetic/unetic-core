use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownExtender {
    pub mac: String,
    pub ip: String,
    pub model: String,
}
