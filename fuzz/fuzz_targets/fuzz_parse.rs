#![no_main]
//! `brush_adapter::parse_bash_script` — every script rush ever runs enters here.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    rush_fuzz::targets::parse_script(data);
});
