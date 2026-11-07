#![cfg(test)]

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{
    auth::ContractContext,
    testutils::{Address as _, BytesN as _, Ledger},
    vec, IntoVal,
};

const USDC: i128 = 10_000_000; // 7 decimals
const DAY: u64 = 86_400;
const T0: u64 = 1_000_000;

struct Setup {
    env: Env,
    account: Address,
    client: AllowanceAccountClient<'static>,
    token: Address,
    payee: Address,
    agent: SigningKey,
    mandate_id: u64,
}

/// $50/day mandate, 30-day expiry, one allowed payee.
fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);

    let owner = Address::generate(&env);
    let issuer = Address::generate(&env);
    let account = env.register(AllowanceAccount, (owner.clone(), issuer.clone()));
    let client = AllowanceAccountClient::new(&env, &account);
    let token = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let payee = Address::generate(&env);
    let agent = SigningKey::from_bytes(&[7u8; 32]);

    client.set_verified(&owner, &true);
    let mandate_id = client.create_mandate(
        &BytesN::from_array(&env, agent.verifying_key().as_bytes()),
        &token,
        &(50 * USDC),
        &DAY,
        &(T0 + 30 * DAY),
        &vec![&env, payee.clone()],
    );

    Setup { env, account, client, token, payee, agent, mandate_id }
}

impl Setup {
    /// Run `__check_auth` for `token.transfer(account, to, amount)` signed by `key`.
    fn pay(&self, key: &SigningKey, to: &Address, amount: i128) -> Result<(), Error> {
        let env = &self.env;
        let payload = BytesN::<32>::random(env);
        let sig = SessionSignature {
            public_key: BytesN::from_array(env, key.verifying_key().as_bytes()),
            signature: BytesN::from_array(env, &key.sign(&payload.to_array()).to_bytes()),
        };
        let ctx = vec![
            env,
            Context::Contract(ContractContext {
                contract: self.token.clone(),
                fn_name: symbol_short!("transfer"),
                args: (self.account.clone(), to.clone(), amount).into_val(env),
            }),
        ];
        match env.try_invoke_contract_check_auth::<Error>(
            &self.account,
            &payload,
            sig.into_val(env),
            &ctx,
        ) {
            Ok(()) => Ok(()),
            Err(Ok(e)) => Err(e),
            Err(Err(e)) => panic!("host error: {e:?}"),
        }
    }
}

#[test]
fn in_cap_transfer_passes() {
    let s = setup();
    assert_eq!(s.pay(&s.agent, &s.payee, 30 * USDC), Ok(()));
    assert_eq!(s.client.remaining(&s.mandate_id), 20 * USDC);
}

#[test]
fn over_cap_transfer_rejected() {
    let s = setup();
    assert_eq!(s.pay(&s.agent, &s.payee, 30 * USDC), Ok(()));
    assert_eq!(s.pay(&s.agent, &s.payee, 30 * USDC), Err(Error::CapExceeded));
    // The rejected attempt consumed nothing.
    assert_eq!(s.client.remaining(&s.mandate_id), 20 * USDC);
}

#[test]
fn expired_mandate_rejected() {
    let s = setup();
    s.env.ledger().set_timestamp(T0 + 30 * DAY);
    assert_eq!(s.pay(&s.agent, &s.payee, USDC), Err(Error::MandateExpired));
}

#[test]
fn revoked_mandate_rejected() {
    let s = setup();
    s.client.revoke_mandate(&s.mandate_id);
    assert_eq!(s.pay(&s.agent, &s.payee, USDC), Err(Error::MandateRevoked));
    assert_eq!(s.client.remaining(&s.mandate_id), 0);
}

#[test]
fn cap_resets_next_epoch() {
    let s = setup();
    assert_eq!(s.pay(&s.agent, &s.payee, 50 * USDC), Ok(()));
    assert_eq!(s.pay(&s.agent, &s.payee, USDC), Err(Error::CapExceeded));
    s.env.ledger().set_timestamp(T0 + DAY);
    assert_eq!(s.pay(&s.agent, &s.payee, 50 * USDC), Ok(()));
}

#[test]
fn destination_not_allowlisted_rejected() {
    let s = setup();
    let stranger = Address::generate(&s.env);
    assert_eq!(s.pay(&s.agent, &stranger, USDC), Err(Error::DestinationNotAllowed));
}

#[test]
fn unknown_session_key_rejected() {
    let s = setup();
    let other = SigningKey::from_bytes(&[9u8; 32]);
    assert_eq!(s.pay(&other, &s.payee, USDC), Err(Error::UnknownSessionKey));
}

#[test]
fn unverified_owner_cannot_create_mandate() {
    let s = setup();
    let env = &s.env;
    let fresh_owner = Address::generate(env);
    let issuer = Address::generate(env);
    let acct = env.register(AllowanceAccount, (fresh_owner, issuer));
    let c = AllowanceAccountClient::new(env, &acct);
    let res = c.try_create_mandate(
        &BytesN::random(env),
        &s.token,
        &(50 * USDC),
        &DAY,
        &(T0 + DAY),
        &vec![env, s.payee.clone()],
    );
    assert_eq!(res, Err(Ok(Error::NotVerified)));
}
