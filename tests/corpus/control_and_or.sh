# mode: posix
true && echo and_ran
false && echo and_skipped
false || echo or_ran
true || echo or_skipped
true && false || echo chain
