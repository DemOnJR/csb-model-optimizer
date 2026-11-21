//! GoldSrc (Half-Life / CS 1.6) studio model optimizer.
//!
//! - `repack`: packs a model's animation streams (identical channels stored once). Same file
//!   name, same behaviour, works for any model and any client.
//! - share: player models whose animation overlaps get one `models/player/_anims/<hash>.mdl`
//!   sequence-group file (GoldSrc's own `$sequencegroup` mechanism, loaded by stock engines).
//! - `gz`: `.gz` copies for fast-download hosts.
//!
//! Every rewritten model is checked with `verify` against its original before it is written.

pub mod gz;
pub mod layout;
pub mod lint;
pub mod process;
pub mod rewrite;
pub mod studio;
pub mod verify;
