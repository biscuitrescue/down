//! OilsChain backend: builds supply-chain records, stores them on IPFS, anchors their commitments on the
//! `OilseedValueChain` contract and verifies them back. See `docs/ARCHITECTURE.md` and `docs/INTEGRITY.md`.

pub mod advisory;
pub mod auth;
pub mod canonical;
pub mod chain;
pub mod cid;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod ipfs;
pub mod merkle;
pub mod nft;
pub mod ops;
pub mod participants;
pub mod pinning;
pub mod ranking;
pub mod reconciler;
pub mod records;
pub mod routes;
pub mod state;
pub mod supply_chain_handlers;
pub mod verify;
pub mod workflows;
