use std::{collections::HashMap, cmp::Ordering};

use cosmwasm_std::{
    to_binary, entry_point, Empty, Env, MessageInfo, Order, Response, DepsMut, CosmosMsg, Uint128, Deps, Decimal, StdResult, Binary, from_slice, Addr, 
};

use cw2::set_contract_version;
use cw20::Cw20Contract;
use cw3::{Status, Cw3Contract};
use cw3_dao::{constants::DAO_PAUSED_KEY};
use cw_storage_plus::Bound;
use cw_utils::{Expiration, maybe_addr};

use crate::{ContractError, state::{CONFIG, next_id, Config}, helpers::{get_staked_balance, get_total_staked_supply, get_deposit_message, get_voting_power_at_height, get_proposal_deposit_refund_message, get_and_check_limit, DEFAULT_LIMIT, MAX_LIMIT, map_proposal}, query::{ProposalResponse, VoteTallyResponse, ProposalListResponse, VoteListResponse, VoterResponse, VoteResponse, ThresholdResponse, ConfigResponse}, msg::{ExecuteMsg, ProposeMsg, VoteMsg, QueryMsg, InstantiateMsg}};

use super::state::{Proposal, Vote, Votes, PROPOSALS, BALLOTS, Ballot, VoteInfo};

const CONTRACT_NAME: &str = "crates.io:cw3-multiple-choice";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    msg.threshold.validate()?;

    let gov_token_addr = Cw20Contract(
                deps.api
                    .addr_validate(&msg.gov_token_address)
                    .map_err(|_| ContractError::InvalidCw20 {addr: msg.gov_token_address.to_string() })?,
            );

    let parent_dao_contract_addr = Cw3Contract(
                deps.api
                    .addr_validate(&msg.parent_dao_contract_address)
                    .map_err(|_| ContractError::InvalidCw3 { addr: msg.parent_dao_contract_address.to_string() })?,
            );

    let cfg = Config {
        threshold: msg.threshold,
        max_voting_period: msg.max_voting_period,
        proposal_deposit: msg.proposal_deposit_amount,
        refund_failed_proposals: msg.refund_failed_proposals,
        gov_token_address: gov_token_addr.addr(),
        parent_dao_contract_address: parent_dao_contract_addr.addr(),
    };

    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response<Empty>, ContractError> {
    match msg {
        ExecuteMsg::Propose(ProposeMsg {
            title,
            description,
            choices,
            msgs,
            latest,
        }) => execute_propose(deps, env, info, title, choices, description, msgs, latest),
        ExecuteMsg::Vote(VoteMsg { proposal_id, vote }) => {
            execute_vote(deps, env, info, proposal_id, vote)
        }
        ExecuteMsg::Execute { proposal_id } => execute_execute(deps, env, info, proposal_id),
        ExecuteMsg::Close { proposal_id } => execute_close(deps, env, info, proposal_id),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Threshold {} => to_binary(&query_threshold(deps)?),
        QueryMsg::Proposal { proposal_id } => to_binary(&query_proposal(deps, env, proposal_id)?),
        QueryMsg::Vote { proposal_id, voter } => to_binary(&query_vote(deps, proposal_id, voter)?),
        QueryMsg::ListProposals { start_after, limit } => {
            to_binary(&query_list_proposals(deps, env, start_after, limit)?)
        }
        QueryMsg::ReverseProposals {
            start_before,
            limit,
        } => to_binary(&query_reverse_proposals(deps, env, start_before, limit)?),
        QueryMsg::ProposalCount {} => to_binary(&query_proposal_count(deps)),
        QueryMsg::ListVotes {
            proposal_id,
            start_after,
            limit,
        } => to_binary(&query_list_votes(deps, proposal_id, start_after, limit)?),
        QueryMsg::GetConfig {} => to_binary(&query_config(deps)?),
        QueryMsg::Voter { address } => to_binary(&query_voter(deps, address)?),
        QueryMsg::Tally { proposal_id } => {
            to_binary(&query_proposal_tally(deps, env, proposal_id)?)
        }
    }
}

pub fn execute_propose(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    title: String,
    choices: Vec<String>,
    description: String,
    msgs: HashMap<u64, Vec<CosmosMsg<Empty>>>,
    // we ignore earliest
    latest: Option<Expiration>,
) -> Result<Response<Empty>, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Check if DAO is Paused
    let res = check_is_paused(&env, &deps, cfg.parent_dao_contract_address)?;
    if res {return Err(ContractError::Paused {});}

    // Only owners of the gov token can create a proposal
    let balance = get_staked_balance(deps.as_ref(), info.sender.clone())?;
    if balance == Uint128::zero() {
        return Err(ContractError::Unauthorized {});
    }

    // Max expires also used as default
    let max_expires = cfg.max_voting_period.after(&env.block);
    let mut expires = latest.unwrap_or(max_expires);
    let comp = expires.partial_cmp(&max_expires);
    if let Some(Ordering::Greater) = comp {
        expires = max_expires;
    } else if comp.is_none() {
        return Err(ContractError::WrongExpiration {});
    }

    // Get total supply
    let total_supply = get_total_staked_supply(deps.as_ref())?;

    let num_choices = choices.capacity();

    // Create a proposal
    let mut prop = Proposal {
        title,
        description,
        choices,
        proposer: info.sender.clone(),
        start_height: env.block.height,
        expires,
        msgs,
        status: Status::Open,
        votes: Votes {
            vote_counts: HashMap::with_capacity(num_choices),
        },
        threshold: cfg.threshold.clone(),
        total_weight: total_supply,
        deposit: cfg.proposal_deposit,
    };
    prop.update_status(&env.block);
    let id = next_id(deps.storage)?;
    PROPOSALS.save(deps.storage, id, &prop)?;

    let gov_token = cfg.gov_token_address;
    let deposit_msg = get_deposit_message(&env, &info, &cfg.proposal_deposit, &gov_token)?;

    Ok(Response::new()
        .add_messages(deposit_msg)
        .add_attribute("action", "propose")
        .add_attribute("sender", info.sender)
        .add_attribute("proposal_id", id.to_string())
        .add_attribute("status", format!("{:?}", prop.status)))
}

pub fn execute_vote(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proposal_id: u64,
    vote: Vote,
) -> Result<Response<Empty>, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Check if DAO is Paused
    let res = check_is_paused(&env, &deps, cfg.parent_dao_contract_address)?;
    if res {return Err(ContractError::Paused {});}

    // Ensure proposal exists and can be voted on
    let mut prop = PROPOSALS.load(deps.storage, proposal_id)?;
    if prop.status != Status::Open {
        return Err(ContractError::NotOpen {});
    }
    if prop.expires.is_expired(&env.block) {
        return Err(ContractError::Expired {});
    }

    // Get voter balance at proposal start
    let vote_power =
        get_voting_power_at_height(deps.as_ref(), info.sender.clone(), prop.start_height)?;

    if vote_power == Uint128::zero() {
        return Err(ContractError::Unauthorized {});
    }

    // Cast vote if no vote previously cast
    BALLOTS.update(deps.storage, (proposal_id, &info.sender), |bal| match bal {
        Some(_) => Err(ContractError::AlreadyVoted {}),
        None => Ok(Ballot {
            weight: vote_power,
            vote,
        }),
    })?;

    // Update vote tally
    // prop.votes.add_vote(vote, vote_power);
    prop.update_status(&env.block);
    PROPOSALS.save(deps.storage, proposal_id, &prop)?;

    Ok(Response::new()
        .add_attribute("action", "vote")
        .add_attribute("sender", info.sender)
        .add_attribute("proposal_id", proposal_id.to_string())
        .add_attribute("status", format!("{:?}", prop.status)))
}

pub fn execute_execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proposal_id: u64,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Check if DAO is Paused
    let res = check_is_paused(&env, &deps, cfg.parent_dao_contract_address)?;
    if res {return Err(ContractError::Paused {});}

    // Anyone can trigger this if the vote passed
    let mut prop = PROPOSALS.load(deps.storage, proposal_id)?;
    // We allow execution even after the proposal "expiration" as long as all vote come in before
    // that point. If it was approved on time, it can be executed any time.
    if prop.current_status(&env.block) != Status::Passed {
        return Err(ContractError::WrongExecuteStatus {});
    }

    // Set it to executed
    prop.status = Status::Executed;
    PROPOSALS.save(deps.storage, proposal_id, &prop)?;
    let gov_token = cfg.gov_token_address;
    let refund_msg =
        get_proposal_deposit_refund_message(&prop.proposer, &prop.deposit, &gov_token)?;

    let highest_vote_count = prop.votes.vote_counts.iter().max_by( |a, b| {
        a.1.cmp(b.1)
    });

    // A proposal with no vote counts should not be in passed state 
    if highest_vote_count == None {
        return Err(ContractError::WrongExecuteStatus {});
    }

    let selected_choice = highest_vote_count.unwrap().0;

    // Dispatch all proposed messages
    Ok(Response::new()
        .add_messages(refund_msg)
        .add_messages(prop.msgs[selected_choice].clone())
        .add_attribute("action", "execute")
        .add_attribute("sender", info.sender)
        .add_attribute("proposal_id", proposal_id.to_string()))
}

pub fn execute_close(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    proposal_id: u64,
) -> Result<Response<Empty>, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Check if DAO is Paused
    let res = check_is_paused(&env, &deps, cfg.parent_dao_contract_address)?;
    if res {return Err(ContractError::Paused {});}

    // Anyone can trigger this if the vote passed
    let mut prop = PROPOSALS.load(deps.storage, proposal_id)?;
    if [Status::Executed, Status::Rejected, Status::Passed]
        .iter()
        .any(|x| *x == prop.status)
    {
        return Err(ContractError::WrongCloseStatus {});
    }
    if !prop.expires.is_expired(&env.block) {
        return Err(ContractError::NotExpired {});
    }

    // Set it to failed
    prop.status = Status::Rejected;
    PROPOSALS.save(deps.storage, proposal_id, &prop)?;

    let gov_token = cfg.gov_token_address;

    let response_with_optional_refund = match cfg.refund_failed_proposals {
        Some(true) => Response::new().add_messages(get_proposal_deposit_refund_message(
            &prop.proposer,
            &prop.deposit,
            &gov_token,
        )?),
        _ => Response::new(),
    };

    Ok(response_with_optional_refund
        .add_attribute("action", "close")
        .add_attribute("sender", info.sender)
        .add_attribute("proposal_id", proposal_id.to_string()))
}

fn query_proposal(deps: Deps, env: Env, id: u64) -> StdResult<ProposalResponse> {
    let prop = PROPOSALS.load(deps.storage, id)?;
    let status = prop.current_status(&env.block);
    let total_supply = get_total_staked_supply(deps)?;
    let threshold = prop.threshold.to_response(total_supply);
    Ok( ProposalResponse { 
        id,
        title: prop.title,
        description: prop.description,
        proposer: prop.proposer,
        msgs: prop.msgs,
        status,
        expires: prop.expires,
        threshold,
        deposit_amount: prop.deposit,
        start_height: prop.start_height,
    })
}

fn query_proposal_tally(deps: Deps, env: Env, id: u64) -> StdResult<VoteTallyResponse> {
    let prop = PROPOSALS.load(deps.storage, id)?;
    let status = prop.current_status(&env.block);
    let total_weight = prop.total_weight;
    let threshold = prop.threshold.to_response(total_weight);

    let total_votes = prop.votes.total();
    let quorum = Decimal::from_ratio(total_votes, total_weight);

    Ok(VoteTallyResponse {
        status,
        threshold,
        quorum,
        total_weight,
        votes: prop.votes.clone(),
        total_votes: prop.votes.vote_counts.clone(),
    })
}

fn query_list_proposals(
    deps: Deps,
    env: Env,
    start_after: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalListResponse> {
    let limit = get_and_check_limit(limit, MAX_LIMIT, DEFAULT_LIMIT)? as usize;
    let start = start_after.map(Bound::exclusive_int);
    let props: StdResult<Vec<_>> = PROPOSALS
        .range_raw(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|p| map_proposal(&env.block, p))
        .collect();

    Ok(ProposalListResponse { proposals: props? })
}

fn query_reverse_proposals(
    deps: Deps,
    env: Env,
    start_before: Option<u64>,
    limit: Option<u32>,
) -> StdResult<ProposalListResponse> {
    let limit = get_and_check_limit(limit, MAX_LIMIT, DEFAULT_LIMIT)? as usize;
    let end = start_before.map(Bound::exclusive_int);
    let props: StdResult<Vec<_>> = PROPOSALS
        .range_raw(deps.storage, None, end, Order::Descending)
        .take(limit)
        .map(|p| map_proposal(&env.block, p))
        .collect();

    Ok(ProposalListResponse { proposals: props? })
}

fn query_threshold(deps: Deps) -> StdResult<ThresholdResponse> {
    let cfg = CONFIG.load(deps.storage)?;
    let total_supply = get_total_staked_supply(deps)?;
    Ok(cfg.threshold.to_response(total_supply))
}

fn query_proposal_count(deps: Deps) -> u64 {
    PROPOSALS
        .keys(deps.storage, None, None, Order::Descending)
        .count() as u64
}

fn query_vote(deps: Deps, proposal_id: u64, voter: String) -> StdResult<VoteResponse> {
    let voter_addr = deps.api.addr_validate(&voter)?;
    let prop = BALLOTS.may_load(deps.storage, (proposal_id, &voter_addr))?;
    let vote = prop.map(|b| VoteInfo {
        voter,
        vote: b.vote,
        weight: b.weight,
    });
    Ok(VoteResponse { vote })
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        config,
    })
}

fn query_list_votes(
    deps: Deps,
    proposal_id: u64,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<VoteListResponse> {
    let limit = get_and_check_limit(limit, MAX_LIMIT, DEFAULT_LIMIT)? as usize;
    let addr = maybe_addr(deps.api, start_after)?;
    let start = addr.map(|addr| Bound::exclusive(addr.as_ref()));

    let votes: StdResult<Vec<_>> = BALLOTS
        .prefix(proposal_id)
        .range_raw(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (voter, ballot) = item?;
            Ok(VoteInfo {
                voter: String::from_utf8(voter)?,
                vote: ballot.vote,
                weight: ballot.weight,
            })
        })
        .collect();

    Ok(VoteListResponse { votes: votes? })
}

fn query_voter(deps: Deps, voter: String) -> StdResult<VoterResponse> {
    let voter_addr = deps.api.addr_validate(&voter)?;
    let weight = get_staked_balance(deps, voter_addr)?;

    Ok(VoterResponse {
        weight: Some(weight),
    })
}

// Query parent dao contract storage 
fn check_is_paused(env: &Env, deps: &DepsMut, parent_contract_address: Addr) -> Result<bool, ContractError> {
    let res = deps.querier.query_wasm_raw(parent_contract_address, DAO_PAUSED_KEY.as_bytes());
    if res.is_err() {
        return Err(ContractError::ExecuteFailed {});
    }

    let paused = res.unwrap();
    if let Some(bytes) = paused {
        let exp = from_slice::<Expiration>(&bytes)?;
        if !exp.is_expired(&env.block) { return Ok(true); }
        }

    return Ok(false);
}
