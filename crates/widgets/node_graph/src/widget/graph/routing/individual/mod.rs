//! Checked single-connection routing over rectangular obstacles.
//!
//! Composes shared corridor geometry into a checked individual path.
//! Inputs are geometric records; document identity and layout adaptation stay outside.

mod router;
#[cfg(test)]
mod tests;

pub(crate) use router::route_with_budget;
