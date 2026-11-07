//! OpenFi Settlement Router (prototype).
//!
//! Releases stablecoin on Stellar when a bank debit clears off-chain. The
//! settlement authority calls `settle` with the bank payment ID; the router
//! pays out of its pre-funded float and records the payment ID so the same
//! debit can never settle twice. The `Settled` event ties the bank payment
//! to the on-chain transfer.
//!
//! Prototype scope (unfunded): one token, a single settlement-authority key
//! (production uses a threshold multisig), and no mandate or Credential
//! Registry checks inside the router. Payout lock/confirm/refund is funded
//! work.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, Address, Env,
    String,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    DuplicatePayment = 1,
    InvalidAmount = 2,
    InvalidPaymentId = 3,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Authority,
    Token,
    /// payment ID -> ledger sequence it settled in.
    Settled(String),
}

#[contractevent]
pub struct Settled {
    #[topic]
    pub payment_id: String,
    pub token: Address,
    pub destination: Address,
    pub amount: i128,
}

const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 365 * DAY_LEDGERS;

#[contract]
pub struct SettlementRouter;

#[contractimpl]
impl SettlementRouter {
    pub fn __constructor(env: Env, authority: Address, token: Address) {
        env.storage().instance().set(&DataKey::Authority, &authority);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    /// Release `amount` of the router's token to `destination` for a cleared
    /// bank debit. Idempotent per `payment_id`: a repeat is rejected.
    pub fn settle(
        env: Env,
        payment_id: String,
        destination: Address,
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
        env.storage()
            .persistent()
            .set(&key, &env.ledger().sequence());
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::TokenClient::new(&env, &token).transfer(
            &env.current_contract_address(),
            &destination,
            &amount,
        );
        Settled {
            payment_id,
            token,
            destination,
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

mod test;
