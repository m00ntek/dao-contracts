use cosmwasm_std::{CosmosMsg, Empty};
use cw_utils::Expiration;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use schemars::JsonSchema;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MultipleChoiceProposeMsg {
    pub title: String,
    pub description: String,
    pub choices: Vec<String>,
    pub msgs: HashMap<u64, Vec<CosmosMsg<Empty>>>,
    // note: we ignore API-spec'd earliest if passed, always opens immediately
    pub latest: Option<Expiration>,
}