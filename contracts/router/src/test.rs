#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, BytesN as _, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    vec, IntoVal,
};

/// The real allowance account code, so the router's code-hash check runs
/// against the same Wasm that is deployed.
mod allowance {
    soroban_sdk::contractimport!(file = "../../target/wasm32v1-none/release/allowance.wasm");
}

const USDC: i128 = 10_000_000;
const DAY: u64 = 86_400;

struct Setup {
    env: Env,
    router: SettlementRouterClient<'static>,
    token: TokenClient<'static>,
    authority: Address,
    account: allowance::Client<'static>,
    mandate_id: u64,
}

fn setup() -> Setup {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    let authority = Address::generate(&env);
    let issuer = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(issuer.clone()).address();
    let wasm = env.deployer().upload_contract_wasm(allowance::WASM);
    let router_id = env.register(
        SettlementRouter,
        (authority.clone(), token_id.clone(), wasm),
    );
    env.mock_all_auths();
    StellarAssetClient::new(&env, &token_id).mint(&router_id, &(1_000 * USDC));
    let owner = Address::generate(&env);
    let account = allowance::Client::new(&env, &env.register(allowance::WASM, (owner.clone(), issuer)));
    account.set_verified(&owner, &true);
    let mandate_id = new_mandate(&env, &account, &token_id);
    env.set_auths(&[]);
    Setup {
        router: SettlementRouterClient::new(&env, &router_id),
        token: TokenClient::new(&env, &token_id),
        authority,
        account,
        mandate_id,
        env,
    }
}

fn new_mandate(env: &Env, account: &allowance::Client, token: &Address) -> u64 {
    account.create_mandate(
        &BytesN::random(env),
        token,
        &(50 * USDC),
        &DAY,
        &(env.ledger().timestamp() + 30 * DAY),
        &vec![env, Address::generate(env)],
    )
}

fn pid(s: &Setup, id: &str) -> String {
    String::from_str(&s.env, id)
}

#[test]
fn settle_releases_funds_once_per_payment_id() {
    let s = setup();
    s.env.mock_all_auths();
    let pid = pid(&s, "pay_sandbox_0001");
    let acct = &s.account.address;

    s.router.settle(&pid, acct, &s.mandate_id, &(50 * USDC));
    assert_eq!(s.token.balance(acct), 50 * USDC);
    assert!(s.router.settled_at(&pid).is_some());

    let again = s.router.try_settle(&pid, acct, &s.mandate_id, &(50 * USDC));
    assert_eq!(again, Err(Ok(Error::DuplicatePayment)));
    assert_eq!(s.token.balance(acct), 50 * USDC);
}

#[test]
fn only_the_settlement_authority_can_settle() {
    let s = setup();
    let pid = pid(&s, "pay_sandbox_0002");
    let acct = s.account.address.clone();
    let auth_as = |who: &Address| {
        s.env.mock_auths(&[MockAuth {
            address: who,
            invoke: &MockAuthInvoke {
                contract: &s.router.address,
                fn_name: "settle",
                args: (pid.clone(), acct.clone(), s.mandate_id, 10 * USDC).into_val(&s.env),
                sub_invokes: &[],
            },
        }]);
    };
    auth_as(&Address::generate(&s.env));
    assert!(s.router.try_settle(&pid, &acct, &s.mandate_id, &(10 * USDC)).is_err());
    assert_eq!(s.token.balance(&acct), 0);

    auth_as(&s.authority);
    s.router.settle(&pid, &acct, &s.mandate_id, &(10 * USDC));
    assert_eq!(s.token.balance(&acct), 10 * USDC);
}

#[test]
fn rejects_bad_input() {
    let s = setup();
    s.env.mock_all_auths();
    let acct = &s.account.address;
    assert_eq!(
        s.router.try_settle(&pid(&s, ""), acct, &s.mandate_id, &USDC),
        Err(Ok(Error::InvalidPaymentId))
    );
    assert_eq!(
        s.router.try_settle(&pid(&s, "pay_sandbox_0003"), acct, &s.mandate_id, &0),
        Err(Ok(Error::InvalidAmount))
    );
}

/// Pretends to be an allowance account: answers `get_mandate` with a live
/// mandate for any id. Its code is not the allowance code.
#[contract]
struct Impostor;

#[contractimpl]
impl Impostor {
    pub fn __constructor(env: Env, token: Address) {
        env.storage().instance().set(&0u32, &token);
    }
    pub fn get_mandate(env: Env, _mandate_id: u64) -> Mandate {
        Mandate {
            session_key: BytesN::from_array(&env, &[7; 32]),
            token: env.storage().instance().get(&0u32).unwrap(),
            cap_per_epoch: 1_000_000 * USDC,
            epoch_secs: DAY,
            epoch_start: 0,
            spent_this_epoch: 0,
            expires_at: u64::MAX,
            allowed_destinations: Vec::new(&env),
            revoked: false,
        }
    }
}

#[test]
fn never_pays_anything_but_an_allowance_account() {
    let s = setup();
    s.env.mock_all_auths();
    let impostor = s.env.register(Impostor, (s.token.address.clone(),));
    let wallet = Address::generate(&s.env);
    for (i, dest) in [
        wallet.clone(),
        s.token.address.clone(),
        s.router.address.clone(),
        impostor.clone(),
    ]
    .iter()
    .enumerate()
    {
        let id = pid(&s, ["pay_w", "pay_t", "pay_r", "pay_i"][i]);
        assert_eq!(
            s.router.try_settle(&id, dest, &s.mandate_id, &(10 * USDC)),
            Err(Ok(Error::NotAMandateAccount))
        );
        assert!(s.router.settled_at(&id).is_none());
    }
    assert_eq!(s.token.balance(&wallet), 0);
    assert_eq!(s.token.balance(&impostor), 0);
    assert_eq!(s.token.balance(&s.router.address), 1_000 * USDC);
}

#[test]
fn refuses_a_mandate_that_is_unknown_revoked_expired_or_for_another_token() {
    let s = setup();
    s.env.mock_all_auths();
    let acct = &s.account.address;
    let try_settle = |id: &str, m: u64| s.router.try_settle(&pid(&s, id), acct, &m, &(10 * USDC));

    assert_eq!(try_settle("pay_unknown", 99), Err(Ok(Error::UnknownMandate)));

    let other_token = s.env.register_stellar_asset_contract_v2(Address::generate(&s.env)).address();
    let m = new_mandate(&s.env, &s.account, &other_token);
    assert_eq!(try_settle("pay_token", m), Err(Ok(Error::NotAMandateAccount)));

    let m = new_mandate(&s.env, &s.account, &s.token.address);
    s.account.revoke_mandate(&m);
    assert_eq!(try_settle("pay_revoked", m), Err(Ok(Error::MandateRevoked)));

    let m = new_mandate(&s.env, &s.account, &s.token.address);
    s.env.ledger().set_timestamp(s.env.ledger().timestamp() + 31 * DAY);
    assert_eq!(try_settle("pay_expired", m), Err(Ok(Error::MandateExpired)));

    assert_eq!(s.token.balance(acct), 0);
}

#[test]
fn reads_the_allowance_mandate_record_field_for_field() {
    let s = setup();
    let theirs = s.account.get_mandate(&s.mandate_id);
    let ours = AllowanceClient::new(&s.env, &s.account.address).get_mandate(&s.mandate_id);
    assert_eq!(ours.session_key, theirs.session_key);
    assert_eq!(ours.token, theirs.token);
    assert_eq!(ours.cap_per_epoch, theirs.cap_per_epoch);
    assert_eq!(ours.epoch_secs, theirs.epoch_secs);
    assert_eq!(ours.epoch_start, theirs.epoch_start);
    assert_eq!(ours.spent_this_epoch, theirs.spent_this_epoch);
    assert_eq!(ours.expires_at, theirs.expires_at);
    assert_eq!(ours.allowed_destinations, theirs.allowed_destinations);
    assert_eq!(ours.revoked, theirs.revoked);
}
