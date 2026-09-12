//! Wire protocol codecs: pure encode/decode, no IO (ARCHITECTURE.md §2).
//!
//! These translate between Rust types and the on-the-wire byte/JSON forms of the
//! external protocols. They live in `faf-domain` because they are pure and
//! exhaustively unit-testable with zero setup; `infra` does the actual IO and
//! calls into these.

pub mod changelog;
pub mod chat_input;
pub mod discord;
pub mod galactic_war;
pub mod game_outcome;
pub mod gpgnet;
pub mod irc;
pub mod log_analysis;
pub mod map_generator;
pub mod map_generator_name;
pub mod markup;
pub mod mod_info;
pub mod replay_query;
pub mod tourney;
pub mod vault_query;
