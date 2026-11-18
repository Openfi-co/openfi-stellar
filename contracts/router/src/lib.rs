//! OpenFi Settlement Router (prototype).
//!
//! Releases stablecoin on Stellar when a bank debit clears off-chain. The
//! settlement authority calls `settle` with the bank payment ID; the router
//! pays out of its pre-funded float and records the payment ID so the same
//! debit can never settle twice. The `Settled` event ties the bank payment
//! to the on-chain transfer.
//!
//! The router never pays an arbitrary address. The destination must be a
//! contract running the OpenFi allowance code (its Wasm hash is checked
//! on-chain) and must hold a live mandate for the router's token. Even the
//! settlement authority cannot send the float to a wallet.
//!
//! Prototype scope (unfunded): one token, a single settlement-authority key
//! (production uses a threshold multisig). Payout lock/confirm/refund is
//! funded work.
#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    token, Address, BytesN, Env, Executable, String, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    DuplicatePayment = 1,
    InvalidAmount = 2,
    InvalidPaymentId = 3,
    /// The destination is not an OpenFi allowance account, or its mandate is
    /// for another token.
    NotAMandateAccount = 4,
    UnknownMandate = 5,
    MandateRevoked = 6,
    MandateExpired = 7,
}

/// The allowance account's mandate record, as `get_mandate` returns it. Field
/// names and types match `allowance::Mandate`; a test checks the two decode
/// alike.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mandate {
    pub session_key: BytesN<32>,
    pub token: Address,
    pub cap_per_epoch: i128,
    pub epoch_secs: u64,
    pub epoch_start: u64,
    pub spent_this_epoch: i128,
    pub expires_at: u64,
    pub allowed_destinations: Vec<Address>,
    pub revoked: bool,
}

/// The one allowance-account call the router makes. It is only made after
/// the account's code hash has been checked, so the answer can be trusted.
#[allow(dead_code)]
#[contractclient(name = "AllowanceClient")]
pub trait Allowance {
    fn get_mandate(env: Env, mandate_id: u64) -> Mandate;
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Authority,
    Token,
    /// Wasm hash of the OpenFi allowance account code.
    AllowanceWasm,
    /// payment ID -> ledger sequence it settled in.
    Settled(String),
}

#[contractevent]
pub struct Settled {
    #[topic]
    pub payment_id: String,
    pub token: Address,
    pub destination: Address,
    pub mandate_id: u64,
    pub amount: i128,
}

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 365 * DAY_LEDGERS;

#[contract]
pub struct SettlementRouter;

#[contractimpl]
impl SettlementRouter {
    /// `allowance_wasm` is the hash of the allowance account code: the only
    /// code a settlement destination may run.
    pub fn __constructor(env: Env, authority: Address, token: Address, allowance_wasm: BytesN<32>) {
        env.storage().instance().set(&DataKey::Authority, &authority);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .set(&DataKey::AllowanceWasm, &allowance_wasm);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    /// Release `amount` of the router's token into the allowance account
    /// `destination` for a cleared bank debit that funds `mandate_id`.
    /// Idempotent per `payment_id`: a repeat is rejected. The destination must
    /// run the allowance code and the mandate must be live and for this
    /// token; otherwise nothing moves.
    pub fn settle(
        env: Env,
        payment_id: String,
        destination: Address,
        mandate_id: u64,
        amount: i128,
    ) -> Result<(), Error> {
        let authority: Address = env.storage().instance().get(&DataKey::Authority).unwrap();
        authority.require_auth();
        if payment_id.len() == 0 || payment_id.len() > 64 {
            return Err(Error::InvalidPaymentId);
        }
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let key = DataKey::Settled(payment_id.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::DuplicatePayment);
        }
        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        check_mandate_account(&env, &destination, mandate_id, &token)?;
        env.storage()
            .persistent()
            .set(&key, &env.ledger().sequence());
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);

        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &destination,
            &amount,
        );
        Settled {
            payment_id,
            token,
            destination,
            mandate_id,
            amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Ledger sequence a payment settled in, if it has.
    pub fn settled_at(env: Env, payment_id: String) -> Option<u32> {
        env.storage().persistent().get(&DataKey::Settled(payment_id))
    }
}

/// `destination` runs the allowance code and holds `mandate_id`, live and
/// for `token`.
fn check_mandate_account(
    env: &Env,
    destination: &Address,
    mandate_id: u64,
    token: &Address,
) -> Result<(), Error> {
    let wasm: BytesN<32> = env
        .storage()
        .instance()
        .get(&DataKey::AllowanceWasm)
        .unwrap();
    // Wallets (Account), asset contracts and any other code are refused here,
    // before the destination is called.
    if destination.executable() != Some(Executable::Wasm(wasm)) {
        return Err(Error::NotAMandateAccount);
    }
    let m = match AllowanceClient::new(env, destination).try_get_mandate(&mandate_id) {
        Ok(Ok(m)) => m,
        _ => return Err(Error::UnknownMandate),
    };
    if m.revoked {
        return Err(Error::MandateRevoked);
    }
    if env.ledger().timestamp() >= m.expires_at {
        return Err(Error::MandateExpired);
    }
    if m.token != *token {
        return Err(Error::NotAMandateAccount);
    }
    Ok(())
}

mod test;
