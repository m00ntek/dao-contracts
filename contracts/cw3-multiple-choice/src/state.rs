use std::{collections::HashMap, convert::TryInto};

use cosmwasm_std::{Addr, BlockInfo, CosmosMsg, Empty, Uint128, StdResult, StdError, Storage};
use cw3::{Status};
use cw_storage_plus::{Map, Item};
use cw_utils::{Expiration, Duration};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{helpers::votes_needed, msg::Threshold};

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Config {
    pub threshold: Threshold,
    pub max_voting_period: Duration,
    pub proposal_deposit: Uint128,
    pub refund_failed_proposals: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Proposal {
    pub title: String,
    pub description: String,
    pub choices: Vec<String>,
    pub proposer: Addr,
    pub start_height: u64,
    pub expires: Expiration,
    pub msgs: HashMap<u64, Vec<CosmosMsg<Empty>>>,
    pub status: Status,
    /// Pass requirements
    pub threshold: Threshold,
    /// The total weight when the proposal started (used to calculate percentages)
    pub total_weight: Uint128,
    /// summary of existing votes
    pub votes: Votes,
    /// Amount of the native governance token required for voting
    pub deposit: Uint128,
}

impl Proposal {
    /// current_status is non-mutable and returns what the status should be.
    /// (designed for queries)
    pub fn current_status(&self, block: &BlockInfo) -> Status {
        let mut status = self.status;

        // if open, check if voting is passed or timed out
        if status == Status::Open && self.is_passed(block) {
            status = Status::Passed;
        }
        if status == Status::Open && self.expires.is_expired(block) {
            status = Status::Rejected;
        }

        status
    }

    /// update_status sets the status of the proposal to current_status.
    /// (designed for handler logic)
    pub fn update_status(&mut self, block: &BlockInfo) {
        self.status = self.current_status(block);
    }

   // returns true iff this proposal is sure to pass (even before expiration if no future
    // sequence of possible votes can cause it to fail)
    pub fn is_passed(&self, block: &BlockInfo) -> bool {
        match self.threshold {
            Threshold::AbsolutePercentage {
                percentage: percentage_needed,
            } => {
                // Check if any of the choices has reached the threshold
                for v in self.votes.vote_counts.values() {
                    if *v >= votes_needed(self.total_weight, percentage_needed) {
                        return true
                    }
                }
                return false
            }
            Threshold::ThresholdQuorum { threshold, quorum } => {
                // we always require the quorum
                if self.votes.total() < votes_needed(self.total_weight, quorum) {
                    return false;
                }
                if self.expires.is_expired(block) {
                    // If expired, we compare Yes votes against the total number of votes
                    for v in self.votes.vote_counts.values() {
                        if *v >= votes_needed(self.votes.total(), threshold) {
                            return true
                        }
                    }
                } else {
                    // If not expired, we must assume all non-votes will be cast as No.
                    // We compare threshold against the total weight
                    for v in self.votes.vote_counts.values() {
                        if *v >= votes_needed(self.total_weight, threshold) {
                            return true
                        }
                    }
                }
                return false
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Vote {
    // A vote is a vector indicating which option(s) the user has selected.
    pub votes: HashMap<u64, bool>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Votes {
    // Vote counts is a vector of integers indicating the vote weights for each option.
    pub vote_counts: HashMap<u64, Uint128>,
}

impl Votes {
    /// sum of all votes
    pub fn total(&self) -> Uint128 {
        self.vote_counts.values().sum()
    }

    pub fn add_vote(&mut self, vote: Vote, mut weight: Uint128) {
        // for each vote selected in the votes array, add the vote weight
        for v in vote.votes {
            if v.1 {
                if self.vote_counts.contains_key(&v.0) {
                    weight += self.vote_counts.get(&v.0).unwrap();
                }
                self.vote_counts.insert(v.0, weight);
            }
        }
    }
}

/// Returns the vote (opinion as well as weight counted) as well as
/// the address of the voter who submitted it
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct VoteInfo {
    pub voter: String,
    pub vote: Vote,
    pub weight: Uint128,
}

pub fn parse_id(data: &[u8]) -> StdResult<u64> {
    match data[0..8].try_into() {
        Ok(bytes) => Ok(u64::from_be_bytes(bytes)),
        Err(_) => Err(StdError::generic_err(
            "Corrupted data found. 8 byte expected.",
        )),
    }
}

// we cast a ballot with our chosen vote and a given weight
// stored under the key that voted
#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
pub struct Ballot {
    pub weight: Uint128,
    pub vote: Vote,
}

pub fn next_id(store: &mut dyn Storage) -> StdResult<u64> {
    let id: u64 = PROPOSAL_COUNT.may_load(store)?.unwrap_or_default() + 1;
    PROPOSAL_COUNT.save(store, &id)?;
    Ok(id)
}

pub const BALLOTS: Map<(u64, &Addr), Ballot> = Map::new("multiple-choice-votes");
pub const PROPOSALS: Map<u64, Proposal> = Map::new("multiple-choice-proposals");

pub const CONFIG: Item<Config> = Item::new("config");
pub const PROPOSAL_COUNT: Item<u64> = Item::new("proposal_count");
pub const DAO_PAUSED: Item<Expiration> = Item::new("dao_paused"); 

// Total weight and voters are queried from this contract
pub const STAKING_CONTRACT: Item<Addr> = Item::new("staking_contract");

pub const GOV_TOKEN: Item<Addr> = Item::new("gov_token");
