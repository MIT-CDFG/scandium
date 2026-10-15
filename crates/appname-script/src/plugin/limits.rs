// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Sandbox resource limits — how a runaway plugin traps typed instead
//! of hanging the app (ES §25.3 sandbox posture; W08 "runaway loops"
//! corpus row).
//!
//! The limits are HOST-INDEPENDENT here; each binding maps them onto
//! its engine's native mechanisms (documented per field):
//!
//! * wasmtime — `Config::consume_fuel` + `Store::set_fuel` (fuel),
//!   `Config::epoch_interruption` + a ticker thread
//!   (`epoch_deadline_ms`), `Store::limiter` / `StoreLimits`
//!   (`memory_max_bytes`, `instances`, `tables`).
//! * mlua — instruction-count hook every `STRIDE` VM instructions
//!   decrementing a fuel account (fuel + deadline check in the same
//!   hook), `Lua::set_memory_limit` (`memory_max_bytes`).
//!
//! Both bindings are LIVE (`hosts/wasm_host.rs`, `hosts/lua_host.rs`);
//! the stub host consumes fuel explicitly so the trap paths are tested
//! even in a `--no-default-features` build.

/// Execution budgets for one plugin invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Abstract execution units per invocation; `None` = unmetered
    /// (never the default). Exhaustion = [`super::host::PluginTrap::FuelExhausted`].
    pub fuel: Option<u64>,
    /// Wall-clock deadline per invocation, milliseconds. Guards host
    /// calls and instruction loops alike (fuel alone cannot bound a
    /// slow host call). Exceeded = [`super::host::PluginTrap::DeadlineExceeded`].
    pub deadline_ms: u64,
    /// Linear/heap memory cap, bytes.
    pub memory_max_bytes: usize,
}

impl ResourceLimits {
    /// The v1 defaults (lane pins, tunable by the embedder; recorded in
    /// the lane log for the fold-in): 500M fuel units ≈ generous for
    /// param-marshalling work — pixel loops belong in core ops (§25.4
    /// "plugins don't pixel-loop in script"); 10 s deadline (a filter
    /// invocation is interactive); 256 MiB memory.
    pub const V1_DEFAULT: Self = Self {
        fuel: Some(500_000_000),
        deadline_ms: 10_000,
        memory_max_bytes: 256 * 1024 * 1024,
    };
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::V1_DEFAULT
    }
}

/// A fuel account, decremented by the executing host; shared shape for
/// the stub host now and the real bindings' hook callbacks later.
#[derive(Debug, Clone)]
pub struct FuelAccount {
    remaining: Option<u64>,
}

impl FuelAccount {
    pub fn new(limits: &ResourceLimits) -> Self {
        Self {
            remaining: limits.fuel,
        }
    }

    /// Consume `n` units; `false` = exhausted (the caller traps typed).
    #[must_use]
    pub fn consume(&mut self, n: u64) -> bool {
        match &mut self.remaining {
            None => true,
            Some(rem) => match rem.checked_sub(n) {
                Some(next) => {
                    *rem = next;
                    true
                }
                None => {
                    *rem = 0;
                    false
                }
            },
        }
    }

    pub fn remaining(&self) -> Option<u64> {
        self.remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuel_exhausts_exactly() {
        let mut acct = FuelAccount::new(&ResourceLimits {
            fuel: Some(10),
            ..ResourceLimits::V1_DEFAULT
        });
        assert!(acct.consume(10));
        assert!(!acct.consume(1));
        assert_eq!(acct.remaining(), Some(0));
    }

    #[test]
    fn unmetered_never_exhausts() {
        let mut acct = FuelAccount::new(&ResourceLimits {
            fuel: None,
            ..ResourceLimits::V1_DEFAULT
        });
        assert!(acct.consume(u64::MAX));
        assert!(acct.consume(u64::MAX));
    }
}
