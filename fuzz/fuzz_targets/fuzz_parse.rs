#![no_main]
//! `brush_adapter::parse_bash_script` — every script oslo ever runs enters here.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    oslo_fuzz::targets::parse_script(data);
});
