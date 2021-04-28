use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use crate::rpc::{AllRequests, AllResponses};
use serde::de::DeserializeOwned;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub id: u64,
    #[serde(flatten)]
    pub data: AllRequests,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer<R: Response> {
    pub(crate) id: u64,
    #[serde(flatten, bound = "R: Response")]
    pub(crate) data: AnswerResult<R>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnswerResult<R: Response> {
    Error(AnswerError),
    #[serde(bound = "R: Response")]
    Ok(R),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnswerError {
    pub(crate) error: String,
}

pub trait Request: Serialize + DeserializeOwned + Clone + Debug {
    type Response: Response;

    fn into_enum(self) -> AllRequests;
}

pub trait Response: Serialize + DeserializeOwned + Clone + Debug + Sized {
    fn into_enum(self) -> AllResponses;
}

