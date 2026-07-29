#![no_main]
//! `eval_arithmetic` — PLAN.md R3.5's target for the overflow guards and the Round 3 rewrite.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    rush_fuzz::targets::eval_arith(data);
});
