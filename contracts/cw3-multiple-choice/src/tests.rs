use cosmwasm_std::{Addr, Decimal, Uint128};
use cw_multi_test::{next_block, App, AppBuilder, Contract, ContractWrapper, Executor};
use cw_utils::Duration;

use crate::msg::{InstantiateMsg, Threshold};

#[test]
fn test_instantiate_success() {
    let mut app = App::default();

    let max_voting_period = Duration::Time(1234567);

    let threshold = Threshold::ThresholdQuorum {
        percentage: Decimal::percent(51),
        quorum: Decimal::percent(10),
    };

    let instantiate_msg = InstantiateMsg {
        threshold: threshold,
        max_voting_period: max_voting_period,
        proposal_deposit_amount: Uint128::zero(),
        gov_token_address: "gov_token".to_string(),
        refund_failed_proposals: None,
        parent_dao_contract_address: "parent".to_string(),
    };

    let res = app
        .instantiate_contract(
            0,
            ,
            &instantiate_msg,
            &[],
            "zero required weight",
            None,
        )
        .unwrap_err();
    //     assert_eq!(ContractError::InvalidThreshold {}, err.downcast().unwrap());

    //     // Total weight less than required weight not allowed
    //     let instantiate_msg = InstantiateMsg {
    //         name: "fishsig".to_string(),
    //         description: "🐟".to_string(),
    //         group: GroupMsg::UseExistingGroup {
    //             addr: group_addr.to_string(),
    //         },
    //         threshold: Threshold::AbsoluteCount { weight: 100 },
    //         max_voting_period,
    //         image_url: None,
    //         only_members_execute: true,
    //     };
    //     let err = app
    //         .instantiate_contract(
    //             multisig_id,
    //             Addr::unchecked(OWNER),
    //             &instantiate_msg,
    //             &[],
    //             "high required weight",
    //             None,
    //         )
    //         .unwrap_err();
    //     assert_eq!(ContractError::UnreachableWeight {}, err.downcast().unwrap());

    //     // All valid
    //     let instantiate_msg = InstantiateMsg {
    //         name: "fishsig".to_string(),
    //         description: "🐟".to_string(),
    //         group: GroupMsg::UseExistingGroup {
    //             addr: group_addr.to_string(),
    //         },
    //         threshold: Threshold::AbsoluteCount { weight: 1 },
    //         max_voting_period,
    //         image_url: None,
    //         only_members_execute: true,
    //     };
    //     let multisig_addr = app
    //         .instantiate_contract(
    //             multisig_id,
    //             Addr::unchecked(OWNER),
    //             &instantiate_msg,
    //             &[],
    //             "all good",
    //             None,
    //         )
    //         .unwrap();

    //     // Verify contract version set properly
    //     let version = query_contract_info(&app, multisig_addr.clone()).unwrap();
    //     assert_eq!(
    //         ContractVersion {
    //             contract: CONTRACT_NAME.to_string(),
    //             version: CONTRACT_VERSION.to_string(),
    //         },
    //         version,
    //     );

    //     // Verify contract config set properly.
    //     let config: ConfigResponse = app
    //         .wrap()
    //         .query_wasm_smart(&multisig_addr, &QueryMsg::GetConfig {})
    //         .unwrap();

    //     assert_eq!(
    //         config,
    //         ConfigResponse {
    //             config: Config {
    //                 name: "fishsig".to_string(),
    //                 description: "🐟".to_string(),
    //                 threshold: Threshold::AbsoluteCount { weight: 1 },
    //                 max_voting_period,
    //                 image_url: None,
    //                 only_members_execute: true
    //             },
    //             group_address: Cw4Contract::new(Addr::unchecked(group_addr)),
    //         }
    //     );

    //     // Get voters query
    //     let voters: VoterListResponse = app
    //         .wrap()
    //         .query_wasm_smart(
    //             &multisig_addr,
    //             &QueryMsg::ListVoters {
    //                 start_after: None,
    //                 limit: None,
    //             },
    //         )
    //         .unwrap();
    //     assert_eq!(
    //         voters.voters,
    //         vec![VoterDetail {
    //             addr: OWNER.into(),
    //             weight: 1
    //         }]
    //     );

    //     // Test instantiation with the contract creating it's own group
    //     let group_id = app.store_code(contract_group());
    //     // All valid
    //     let instantiate_msg = InstantiateMsg {
    //         name: "fishsig".to_string(),
    //         description: "🐟".to_string(),
    //         group: GroupMsg::InstantiateNewGroup {
    //             code_id: group_id,
    //             label: String::from("Test Instantiating New Group"),
    //             voters: vec![member(OWNER, 1)],
    //         },
    //         threshold: Threshold::AbsoluteCount { weight: 1 },
    //         max_voting_period,
    //         image_url: Some("https://imgur.com/someElmo.png".to_string()),
    //         only_members_execute: true,
    //     };
    //     let res = app.instantiate_contract(
    //         multisig_id,
    //         Addr::unchecked(OWNER),
    //         &instantiate_msg,
    //         &[],
    //         "all good",
    //         None,
    //     );
    //     assert!(res.is_ok());
}
