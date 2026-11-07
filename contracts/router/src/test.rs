#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    IntoVal,
};

const USDC: i128 = 10_000_000;

struct Setup {
    env: Env,
    router: SettlementRouterClient<'static>,
    token: TokenClient<'static>,
    authority: Address,
    mandate_account: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    let authority = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(issuer).address();
    let router_id = env.register(SettlementRouter, (authority.clone(), token_id.clone()));
    env.mock_all_auths();
    StellarAssetClient::new(&env, &token_id).mint(&router_id, &(1_000 * USDC));
    env.set_auths(&[]);
    Setup {
        router: SettlementRouterClient::new(&env, &router_id),
        token: TokenClient::new(&env, &token_id),
        mandate_account: Address::generate(&env),
        authority,
        env,
    }
}

#[test]
fn settle_releases_funds_once_per_payment_id() {
    let s = setup();
    s.env.mock_all_auths();
    let pid = String::from_str(&s.env, "pay_sandbox_0001");

    s.router.settle(&pid, &s.mandate_account, &(50 * USDC));
    assert_eq!(s.token.balance(&s.mandate_account), 50 * USDC);
    assert!(s.router.settled_at(&pid).is_some());

    let again = s.router.try_settle(&pid, &s.mandate_account, &(50 * USDC));
    assert_eq!(again, Err(Ok(Error::DuplicatePayment)));
    assert_eq!(s.token.balance(&s.mandate_account), 50 * USDC);
}

#[test]
fn only_the_settlement_authority_can_settle() {
    let s = setup();
    let pid = String::from_str(&s.env, "pay_sandbox_0002");
    let intruder = Address::generate(&s.env);
    s.env.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &s.router.address,
            fn_name: "settle",
            args: (pid.clone(), s.mandate_account.clone(), 10 * USDC).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    assert!(s.router.try_settle(&pid, &s.mandate_account, &(10 * USDC)).is_err());
    assert_eq!(s.token.balance(&s.mandate_account), 0);

    s.env.mock_auths(&[MockAuth {
        address: &s.authority,
        invoke: &MockAuthInvoke {
            contract: &s.router.address,
            fn_name: "settle",
            args: (pid.clone(), s.mandate_account.clone(), 10 * USDC).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    s.router.settle(&pid, &s.mandate_account, &(10 * USDC));
    assert_eq!(s.token.balance(&s.mandate_account), 10 * USDC);
}

#[test]
fn rejects_bad_input() {
    let s = setup();
    s.env.mock_all_auths();
    let empty = String::from_str(&s.env, "");
    let pid = String::from_str(&s.env, "pay_sandbox_0003");
    assert_eq!(
        s.router.try_settle(&empty, &s.mandate_account, &USDC),
        Err(Ok(Error::InvalidPaymentId))
    );
    assert_eq!(
        s.router.try_settle(&pid, &s.mandate_account, &0),
        Err(Ok(Error::InvalidAmount))
    );
}
