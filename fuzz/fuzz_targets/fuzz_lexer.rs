#![no_main]
//! The word lexer and its token scanner — quoting, `$` forms and ANSI-C escapes.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    rush_fuzz::targets::lex_word(data);
});
