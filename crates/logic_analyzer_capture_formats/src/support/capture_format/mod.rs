//! Common parsing and packed-sample helpers for capture-file support.

mod parsing;

pub(crate) use parsing::{get_packed_bit, parse_sample_rate};
