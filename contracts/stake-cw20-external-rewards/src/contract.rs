use crate::msg::{
    ExecuteMsg, InfoResponse, InstantiateMsg, PendingRewardsResponse, QueryMsg, ReceiveMsg,
};
use crate::state::{
    Config, RewardConfig, CONFIG, LAST_UPDATE_BLOCK, PENDING_REWARDS, REWARD_CONFIG,
    REWARD_PER_TOKEN, USER_REWARD_PER_TOKEN,
};
use crate::ContractError;
use crate::ContractError::{NoRewardsClaimable, Unauthorized};
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_binary, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Empty, Env,
    MessageInfo, Response, StdError, StdResult, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw20::{Cw20ReceiveMsg, Denom};
use stake_cw20::hooks::StakeChangedHookMsg;

use std::cmp::{max, min};

const CONTRACT_NAME: &str = "crates.io:stake_cw20";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response<Empty>, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let admin = match msg.admin {
        Some(admin) => Some(deps.api.addr_validate(admin.as_str())?),
        None => None,
    };

    let config = Config {
        admin,
        staking_contract: msg.staking_contract,
        reward_token: msg.reward_token,
    };
    CONFIG.save(deps.storage, &config)?;

    let reward_config = RewardConfig {
        periodFinish: 0,
        rewardRate: Default::default(),
        rewardDuration: 100000,
    };
    REWARD_CONFIG.save(deps.storage, &reward_config);

    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response<Empty>, ContractError> {
    match msg {
        ExecuteMsg::StakeChangeHook(msg) => execute_stake_changed(deps, env, info, msg),
        ExecuteMsg::Claim {} => execute_claim(deps, env, info),
        ExecuteMsg::Fund {} => execute_fund_native(deps, env, info),
        ExecuteMsg::Receive(msg) => execute_receive(deps, env, info, msg),
    }
}

pub fn execute_receive(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    wrapper: Cw20ReceiveMsg,
) -> Result<Response<Empty>, ContractError> {
    let msg: ReceiveMsg = from_binary(&wrapper.msg)?;
    let config = CONFIG.load(deps.storage)?;
    let sender = deps.api.addr_validate(&*wrapper.sender)?;
    if config.reward_token != Denom::Cw20(info.sender) {
        return Err(Unauthorized {});
    };
    if config.admin != Some(sender.clone()) {
        return Err(Unauthorized {});
    };
    match msg {
        ReceiveMsg::Fund { .. } => execute_fund(deps, env, sender, wrapper.amount),
    }
}

pub fn execute_fund_native(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response<Empty>, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if config.admin != Some(info.sender.clone()) {
        return Err(Unauthorized {});
    };
    // TODO: Better error handling here
    let coin = info.funds.first().unwrap();
    let amount = coin.clone().amount;
    let denom = coin.clone().denom;
    if config.reward_token != Denom::Native(denom) {
        return Err(Unauthorized {});
    };
    execute_fund(deps, env, info.sender, amount)
}

pub fn execute_fund(
    mut deps: DepsMut,
    env: Env,
    sender: Addr,
    amount: Uint128,
) -> Result<Response<Empty>, ContractError> {
    update_rewards(&mut deps, &env, &sender)?;

    let reward_config = REWARD_CONFIG.load(deps.storage)?;
    println!(
        "periodFinish: {}, blockHeight: {}",
        reward_config.periodFinish, env.block.height
    );
    let new_reward_config = if reward_config.periodFinish <= env.block.height {
        println!("new");
        RewardConfig {
            periodFinish: env.block.height + reward_config.rewardDuration,
            rewardRate: amount / Uint128::from(reward_config.rewardDuration),
            rewardDuration: reward_config.rewardDuration,
        }
    } else {
        RewardConfig {
            periodFinish: reward_config.periodFinish,
            rewardRate: reward_config.rewardRate
                + (amount / Uint128::from(reward_config.periodFinish - env.block.height)),
            rewardDuration: reward_config.rewardDuration,
        }
    };

    println!("rate {}", new_reward_config.rewardRate);
    println!("duration {}", new_reward_config.rewardDuration);
    println!("period_finish {}", new_reward_config.periodFinish);

    REWARD_CONFIG.save(deps.storage, &new_reward_config);

    Ok(Response::new()
        .add_attribute("action", "fund")
        .add_attribute("amount", amount))
}

pub fn execute_stake_changed(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: StakeChangedHookMsg,
) -> Result<Response<Empty>, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    if info.sender != config.staking_contract {
        return Err(ContractError::Unauthorized {});
    };
    match msg {
        StakeChangedHookMsg::Stake { addr, .. } => execute_stake(deps, env, addr),
        StakeChangedHookMsg::Unstake { addr, .. } => execute_unstake(deps, env, addr),
    }
}

pub fn execute_stake(
    mut deps: DepsMut,
    env: Env,
    addr: Addr,
) -> Result<Response<Empty>, ContractError> {
    update_rewards(&mut deps, &env, &addr)?;
    Ok(Response::new().add_attribute("action", "stake"))
}

pub fn execute_unstake(
    mut deps: DepsMut,
    env: Env,
    addr: Addr,
) -> Result<Response<Empty>, ContractError> {
    update_rewards(&mut deps, &env, &addr)?;
    Ok(Response::new().add_attribute("action", "unstake"))
}

pub fn execute_claim(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response<Empty>, ContractError> {
    update_rewards(&mut deps, &env, &info.sender)?;
    let rewards = PENDING_REWARDS
        .load(deps.storage, info.sender.clone())
        .map_err(|_| NoRewardsClaimable {})?;
    PENDING_REWARDS.save(deps.storage, info.sender.clone(), &Uint128::zero());
    let config = CONFIG.load(deps.storage)?;
    let transfer_msg = get_transfer_msg(info.sender, rewards, config.reward_token)?;
    Ok(Response::new()
        .add_message(transfer_msg)
        .add_attribute("action", "claim")
        .add_attribute("amount", rewards))
}

pub fn get_transfer_msg(recipient: Addr, amount: Uint128, denom: Denom) -> StdResult<CosmosMsg> {
    match denom {
        Denom::Native(denom) => Ok(BankMsg::Send {
            to_address: recipient.into_string(),
            amount: vec![Coin {
                denom,
                amount,
            }],
        }
        .into()),
        Denom::Cw20(addr) => {
            let cw20_msg = to_binary(&cw20::Cw20ExecuteMsg::Transfer {
                recipient: recipient.into_string(),
                amount,
            })?;
            Ok(WasmMsg::Execute {
                contract_addr: addr.into_string(),
                msg: cw20_msg,
                funds: vec![],
            }
            .into())
        }
    }
}

pub fn update_rewards(deps: &mut DepsMut, env: &Env, addr: &Addr) -> StdResult<()> {
    let config = CONFIG.load(deps.storage)?;
    let reward_per_token = get_reward_per_token(deps.as_ref(), env, &config.staking_contract)?;
    REWARD_PER_TOKEN.save(deps.storage, &reward_per_token);

    let earned_rewards = get_rewards_earned(
        deps.as_ref(),
        env,
        addr,
        reward_per_token,
        &config.staking_contract,
    )?;
    PENDING_REWARDS.update::<_, StdError>(deps.storage, addr.clone(), |r| {
        Ok(r.unwrap_or_default() + earned_rewards)
    });

    USER_REWARD_PER_TOKEN.save(deps.storage, addr.clone(), &reward_per_token);
    LAST_UPDATE_BLOCK.save(deps.storage, &env.block.height)?;
    Ok({})
}

pub fn get_reward_per_token(deps: Deps, env: &Env, staking_contract: &Addr) -> StdResult<Uint128> {
    let reward_config = REWARD_CONFIG.load(deps.storage)?;
    let total_staked = get_total_staked(deps, staking_contract)?;
    let current_block = min(env.block.height, reward_config.periodFinish);
    let last_update_block = LAST_UPDATE_BLOCK.load(deps.storage).unwrap_or_default();
    let prev_reward_per_token = REWARD_PER_TOKEN.load(deps.storage).unwrap_or_default();
    let additional_reward_per_token = if total_staked == Uint128::zero() {
        Uint128::zero()
    } else {
        (reward_config.rewardRate
            * max(
                Uint128::from(current_block - last_update_block),
                Uint128::zero(),
            ))
            / total_staked
    };

    Ok(prev_reward_per_token + additional_reward_per_token)
}

pub fn get_rewards_earned(
    deps: Deps,
    _env: &Env,
    addr: &Addr,
    reward_per_token: Uint128,
    staking_contract: &Addr,
) -> StdResult<Uint128> {
    let _config = CONFIG.load(deps.storage)?;
    let staked_balance = get_staked_balance(deps, staking_contract, addr)?;
    let user_reward_per_token = USER_REWARD_PER_TOKEN
        .load(deps.storage, addr.clone())
        .unwrap_or_default();

    Ok((reward_per_token - user_reward_per_token) * staked_balance)
}

fn get_total_staked(deps: Deps, contract_addr: &Addr) -> StdResult<Uint128> {
    let msg = stake_cw20::msg::QueryMsg::TotalStakedAtHeight { height: None };
    let resp: stake_cw20::msg::TotalStakedAtHeightResponse =
        deps.querier.query_wasm_smart(contract_addr, &msg)?;
    Ok(resp.total)
}

fn get_staked_balance(deps: Deps, contract_addr: &Addr, addr: &Addr) -> StdResult<Uint128> {
    let msg = stake_cw20::msg::QueryMsg::StakedBalanceAtHeight {
        address: addr.into(),
        height: None,
    };
    let resp: stake_cw20::msg::StakedBalanceAtHeightResponse =
        deps.querier.query_wasm_smart(contract_addr, &msg)?;
    Ok(resp.balance)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Info {} => Ok(to_binary(&query_info(deps, env)?)?),
        QueryMsg::GetPendingRewards { address } => {
            Ok(to_binary(&query_pending_rewards(deps, env, address)?)?)
        }
    }
}

pub fn query_info(deps: Deps, _env: Env) -> StdResult<InfoResponse> {
    let config = CONFIG.load(deps.storage)?;
    let reward = REWARD_CONFIG.load(deps.storage)?;
    Ok(InfoResponse { config, reward })
}

pub fn query_pending_rewards(
    deps: Deps,
    env: Env,
    addr: Addr,
) -> StdResult<PendingRewardsResponse> {
    let config = CONFIG.load(deps.storage)?;
    let reward_per_token = get_reward_per_token(deps, &env, &config.staking_contract)?;
    let earned_rewards = get_rewards_earned(
        deps,
        &env,
        &addr,
        reward_per_token,
        &config.staking_contract,
    )?;

    let existing_rewards = PENDING_REWARDS
        .load(deps.storage, addr.clone())
        .unwrap_or_default();
    let pending_rewards = earned_rewards + existing_rewards;
    Ok(PendingRewardsResponse {
        address: addr,
        pending_rewards,
        denom: config.reward_token,
    })
}

#[cfg(test)]
mod tests {
    use std::borrow::{Borrow, BorrowMut};

    use crate::ContractError;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coin, to_binary, Addr, CosmosMsg, Empty, MessageInfo, Uint128, Coin};
    use cw20::{Cw20Coin, Cw20CoinVerified, Cw20Contract, Cw20ExecuteMsg, Denom};
    use cw_utils::Duration;

    use cw_multi_test::{
        next_block, App, AppResponse, BankSudo, Contract, ContractWrapper, Executor, SudoMsg,
    };

    use anyhow::Result as AnyResult;
    use cosmwasm_std::OverflowOperation::Add;
    use cw20::Balance::Cw20;

    use crate::msg::QueryMsg::Info;
    use crate::msg::{ExecuteMsg, InfoResponse, InstantiateMsg, PendingRewardsResponse, QueryMsg, ReceiveMsg};
    use cw_controllers::{Claim, ClaimsResponse};
    use cw_utils::Expiration::AtHeight;

    const ADMIN: &str = "admin";
    const ADDR1: &str = "addr0001";
    const ADDR2: &str = "addr0002";
    const ADDR3: &str = "addr0003";
    const ADDR4: &str = "addr0004";

    pub fn contract_rewards() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        );
        Box::new(contract)
    }

    pub fn contract_staking() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            stake_cw20::contract::execute,
            stake_cw20::contract::instantiate,
            stake_cw20::contract::query,
        );
        Box::new(contract)
    }

    pub fn contract_cw20() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            cw20_base::contract::execute,
            cw20_base::contract::instantiate,
            cw20_base::contract::query,
        );
        Box::new(contract)
    }

    fn mock_app() -> App {
        App::default()
    }

    fn get_balance<T: Into<String>, U: Into<String>>(
        app: &App,
        contract_addr: T,
        address: U,
    ) -> Uint128 {
        let msg = cw20::Cw20QueryMsg::Balance {
            address: address.into(),
        };
        let result: cw20::BalanceResponse =
            app.wrap().query_wasm_smart(contract_addr, &msg).unwrap();
        result.balance
    }

    fn instantiate_cw20(app: &mut App, initial_balances: Vec<Cw20Coin>) -> Addr {
        let cw20_id = app.store_code(contract_cw20());
        let msg = cw20_base::msg::InstantiateMsg {
            name: String::from("Test"),
            symbol: String::from("TEST"),
            decimals: 6,
            initial_balances,
            mint: None,
            marketing: None,
        };

        app.instantiate_contract(cw20_id, Addr::unchecked(ADDR1), &msg, &[], "cw20", None)
            .unwrap()
    }

    fn instantiate_staking(
        app: &mut App,
        cw20: Addr,
        unstaking_duration: Option<Duration>,
    ) -> Addr {
        let staking_code_id = app.store_code(contract_staking());
        let msg = stake_cw20::msg::InstantiateMsg {
            owner: Some(Addr::unchecked(ADMIN)),
            manager: Some(Addr::unchecked("manager")),
            token_address: cw20,
            unstaking_duration,
        };
        app.instantiate_contract(
            staking_code_id,
            Addr::unchecked(ADDR1),
            &msg,
            &[],
            "staking",
            None,
        )
        .unwrap()
    }

    fn stake_tokens <T: Into<String>>(
        app: &mut App,
        staking_addr: &Addr,
        cw20_addr: &Addr,
        sender: T,
        amount: u128,
    ) {
        let msg = cw20::Cw20ExecuteMsg::Send {
            contract: staking_addr.to_string(),
            amount: Uint128::new(amount),
            msg: to_binary(&stake_cw20::msg::ReceiveMsg::Stake {}).unwrap(),
        };
        app.execute_contract(Addr::unchecked(sender), cw20_addr.clone(), &msg, &[]).unwrap();
    }

    fn unstake_tokens(
        app: &mut App,
        staking_addr: &Addr,
        address: &str,
        amount: u128,
    ) {
        let msg = stake_cw20::msg::ExecuteMsg::Unstake { amount: Uint128::new(amount) };
        app.execute_contract(Addr::unchecked(address), staking_addr.clone(), &msg, &[]).unwrap();
    }

    fn setup_staking_contract(app: &mut App, initial_balances: Vec<Cw20Coin>) -> (Addr, Addr) {
        // Instantiate cw20 contract
        let cw20_addr = instantiate_cw20(app, initial_balances.clone());
        app.update_block(next_block);
        // Instantiate staking contract
        let staking_addr = instantiate_staking(app, cw20_addr.clone(), None);
        app.update_block(next_block);
        for coin in initial_balances {
            stake_tokens(app, &staking_addr, &cw20_addr, coin.address, coin.amount.u128());
        }
        (staking_addr, cw20_addr)
    }

    fn setup_reward_contract(
        app: &mut App,
        staking_contract: Addr,
        reward_token: Denom,
        admin: Addr,
    ) -> Addr {
        let reward_code_id = app.store_code(contract_rewards());
        let msg = crate::msg::InstantiateMsg {
            admin: Some(admin.clone()),
            staking_contract: staking_contract.clone(),
            reward_token,
        };
        let reward_addr = app.instantiate_contract(reward_code_id, admin, &msg, &[], "reward", None).unwrap();
        let msg = stake_cw20::msg::ExecuteMsg::AddHook { addr: reward_addr.clone() };
        app.execute_contract(Addr::unchecked(ADMIN), staking_contract, &msg, &[]);
        reward_addr
    }

    fn get_balance_cw20<T: Into<String>, U: Into<String>>(
        app: &App,
        contract_addr: T,
        address: U,
    ) -> Uint128 {
        let msg = cw20::Cw20QueryMsg::Balance {
            address: address.into(),
        };
        let result: cw20::BalanceResponse =
            app.wrap().query_wasm_smart(contract_addr, &msg).unwrap();
        result.balance
    }

    fn get_balance_native<T: Into<String>, U: Into<String>>(
        app: &App,
        address: T,
        denom: U,
    ) -> Uint128 {
        app.wrap().query_balance(address, denom).unwrap().amount
    }

    fn assert_pending_rewards(app: &mut App, reward_addr: &Addr, address: &str, expected: u128) {
        let res: PendingRewardsResponse = app
            .borrow_mut()
            .wrap()
            .query_wasm_smart(
                reward_addr,
                &QueryMsg::GetPendingRewards {
                    address: Addr::unchecked(address),
                },
            )
            .unwrap();
        assert_eq!(res.pending_rewards, Uint128::new(expected));
    }

    fn claim_rewards(app: &mut App, reward_addr: Addr, address: &str) {
        let msg = ExecuteMsg::Claim {};
       app.borrow_mut().execute_contract(Addr::unchecked(address), reward_addr, &msg, &[]).unwrap();
    }

    fn fund_rewards_cw20(app: &mut App, admin: &Addr, reward_token: Addr, reward_addr: &Addr, amount: u128) {
        let fund_sub_msg = to_binary(&ReceiveMsg::Fund {}).unwrap();
        let fund_msg = Cw20ExecuteMsg::Send {
            contract: reward_addr.clone().into_string(),
            amount: Uint128::new(amount),
            msg: fund_sub_msg
        };
        let _res = app
            .borrow_mut()
            .execute_contract(admin.clone(), reward_token.clone(), &fund_msg, &[])
            .unwrap();
    }

    #[test]
    fn test_native_rewards() {
        let mut app = mock_app();
        let admin = Addr::unchecked(ADMIN);
        app.borrow_mut().update_block(|b| b.height = 0);
        let amount1 = Uint128::from(100u128);
        let _token_address = Addr::unchecked("token_address");
        let initial_balances = vec![
            Cw20Coin {
                address: ADDR1.to_string(),
                amount: Uint128::new(100),
            },
            Cw20Coin {
                address: ADDR2.to_string(),
                amount: Uint128::new(50),
            },
            Cw20Coin {
                address: ADDR3.to_string(),
                amount: Uint128::new(50),
            },
        ];
        let denom = "utest".to_string();
        let (staking_addr, cw20_addr) = setup_staking_contract(&mut app, initial_balances);
        let reward_funding = vec![coin(100000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();
        let reward_addr =
            setup_reward_contract(&mut app, staking_addr.clone(), Denom::Native(denom.clone()), admin.clone());

        app.borrow_mut().update_block(|b| b.height = 1000);

        let fund_msg = ExecuteMsg::Fund {};

        let _res = app
            .borrow_mut()
            .execute_contract(admin.clone(), reward_addr.clone(), &fund_msg, &*reward_funding)
            .unwrap();

        let res: InfoResponse = app
            .borrow_mut()
            .wrap()
            .query_wasm_smart(&reward_addr, &QueryMsg::Info {})
            .unwrap();

        assert_eq!(res.reward.rewardRate, Uint128::new(1000));
        assert_eq!(res.reward.periodFinish, 101000);
        assert_eq!(res.reward.rewardDuration, 100000);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 250);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 250);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 1000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 500);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 1500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 750);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 750);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 2000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 1000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 1000);

        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::zero());
        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::new(2000));
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 0);

        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 5000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 3500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 3500);

        unstake_tokens(&mut app, &staking_addr, ADDR2, 50);
        unstake_tokens(&mut app, &staking_addr, ADDR3, 50);

        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 15000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 3500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 3500);

        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::new(17000));

        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_native(&mut app, ADDR2, &denom), Uint128::new(3500));

        stake_tokens(&mut app, &staking_addr, &cw20_addr, ADDR2, 50);
        stake_tokens(&mut app, &staking_addr, &cw20_addr, ADDR3, 50);


        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 5000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 2500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 6000);

        app.borrow_mut().update_block(|b| b.height = 101000);
        // TODO: check these expected number are correct
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 49988000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 24994000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 24997500);


        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::new(50005000));
        assert_eq!(get_balance_native(&mut app, ADDR2, &denom), Uint128::new(24997500));
        assert_eq!(get_balance_native(&mut app, ADDR3, &denom), Uint128::new(0));
        assert_eq!(get_balance_native(&mut app, &reward_addr, &denom), Uint128::new(24997500));

        app.borrow_mut().update_block(|b| b.height = 200000);
        let fund_msg = ExecuteMsg::Fund {};

        // Add more rewards
        let reward_funding = vec![coin(200000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();

        let _res = app
            .borrow_mut()
            .execute_contract(admin.clone(), reward_addr.clone(), &fund_msg, &*reward_funding)
            .unwrap();

        app.borrow_mut().update_block(|b| b.height = 300000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 100000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 50000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 74997500);


        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::new(150005000));
        assert_eq!(get_balance_native(&mut app, ADDR2, &denom), Uint128::new(74997500));
        assert_eq!(get_balance_native(&mut app, ADDR3, &denom), Uint128::zero());
        assert_eq!(get_balance_native(&mut app, &reward_addr, &denom), Uint128::new(74997500));

        // Add more rewards, then add even more on top
        let reward_funding = vec![coin(100000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();

        let _res = app
            .borrow_mut()
            .execute_contract(admin.clone(), reward_addr.clone(), &fund_msg, &*reward_funding)
            .unwrap();

        let reward_funding = vec![coin(100000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();

        let _res = app
            .borrow_mut()
            .execute_contract(admin.clone(), reward_addr.clone(), &fund_msg, &*reward_funding)
            .unwrap();

        app.borrow_mut().update_block(|b| b.height = 400000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 100000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 50000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 124997500);

        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        claim_rewards(&mut app, reward_addr.clone(), ADDR3);
        assert_eq!(get_balance_native(&mut app, ADDR1, &denom), Uint128::new(250005000));
        assert_eq!(get_balance_native(&mut app, ADDR2, &denom), Uint128::new(124997500));
        assert_eq!(get_balance_native(&mut app, ADDR3, &denom), Uint128::new(124997500));
        assert_eq!(get_balance_native(&mut app, &reward_addr, &denom), Uint128::zero());
    }

    #[test]
    fn test_cw20_rewards() {
        let mut app = mock_app();
        let admin = Addr::unchecked(ADMIN);
        app.borrow_mut().update_block(|b| b.height = 0);
        let amount1 = Uint128::from(100u128);
        let _token_address = Addr::unchecked("token_address");
        let initial_balances = vec![
            Cw20Coin {
                address: ADDR1.to_string(),
                amount: Uint128::new(100),
            },
            Cw20Coin {
                address: ADDR2.to_string(),
                amount: Uint128::new(50),
            },
            Cw20Coin {
                address: ADDR3.to_string(),
                amount: Uint128::new(50),
            },
        ];
        let denom = "utest".to_string();
        let (staking_addr, cw20_addr) = setup_staking_contract(&mut app, initial_balances);
        let reward_funding = vec![coin(100000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();
        let reward_token = instantiate_cw20(&mut app, vec![Cw20Coin{
            address: ADMIN.to_string(),
            amount: Uint128::new(500000000)
        }]);
        let reward_addr =
            setup_reward_contract(&mut app, staking_addr.clone(), Denom::Cw20(reward_token.clone()), admin.clone());

        app.borrow_mut().update_block(|b| b.height = 1000);

        fund_rewards_cw20(&mut app, &admin, reward_token.clone(), &reward_addr, 100000000);

        let res: InfoResponse = app
            .borrow_mut()
            .wrap()
            .query_wasm_smart(&reward_addr, &QueryMsg::Info {})
            .unwrap();

        assert_eq!(res.reward.rewardRate, Uint128::new(1000));
        assert_eq!(res.reward.periodFinish, 101000);
        assert_eq!(res.reward.rewardDuration, 100000);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 250);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 250);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 1000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 500);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 1500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 750);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 750);

        app.borrow_mut().update_block(next_block);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 2000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 1000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 1000);

        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::zero());
        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::new(2000));
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 0);

        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 5000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 3500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 3500);

        unstake_tokens(&mut app, &staking_addr, ADDR2, 50);
        unstake_tokens(&mut app, &staking_addr, ADDR3, 50);

        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 15000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 3500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 3500);

        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::new(17000));

        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR2), Uint128::new(3500));

        stake_tokens(&mut app, &staking_addr, &cw20_addr, ADDR2, 50);
        stake_tokens(&mut app, &staking_addr, &cw20_addr, ADDR3, 50);


        app.borrow_mut().update_block(|b| b.height += 10);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 5000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 2500);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 6000);


        app.borrow_mut().update_block(|b| b.height = 101000);
        // TODO: check these expected number are correct
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 49988000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 24994000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 24997500);


        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::new(50005000));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR2), Uint128::new(24997500));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR3), Uint128::new(0));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, &reward_addr), Uint128::new(24997500));

        app.borrow_mut().update_block(|b| b.height = 200000);
        let fund_msg = ExecuteMsg::Fund {};

        // Add more rewards
        let reward_funding = vec![coin(200000000, denom.clone())];
        app.sudo(SudoMsg::Bank({
            BankSudo::Mint {
                to_address: admin.to_string(),
                amount: reward_funding.clone(),
            }
        }))
            .unwrap();

        fund_rewards_cw20(&mut app, &admin, reward_token.clone(), &reward_addr, 200000000);

        app.borrow_mut().update_block(|b| b.height = 300000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 100000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 50000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 74997500);


        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::new(150005000));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR2), Uint128::new(74997500));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR3), Uint128::zero());
        assert_eq!(get_balance_cw20(&mut app, &reward_token, &reward_addr), Uint128::new(74997500));

        // Add more rewards, then add even more on top
        fund_rewards_cw20(&mut app, &admin, reward_token.clone(), &reward_addr, 100000000);
        fund_rewards_cw20(&mut app, &admin, reward_token.clone(), &reward_addr, 100000000);

        app.borrow_mut().update_block(|b| b.height = 400000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR1, 100000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR2, 50000000);
        assert_pending_rewards(&mut app, &reward_addr, ADDR3, 124997500);

        claim_rewards(&mut app, reward_addr.clone(), ADDR1);
        claim_rewards(&mut app, reward_addr.clone(), ADDR2);
        claim_rewards(&mut app, reward_addr.clone(), ADDR3);
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR1), Uint128::new(250005000));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR2), Uint128::new(124997500));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, ADDR3), Uint128::new(124997500));
        assert_eq!(get_balance_cw20(&mut app, &reward_token, &reward_addr), Uint128::zero());
    }

}