# Vendored brush-parser 0.4.0 — why, and how to stop

This directory is the published `brush-parser` 0.4.0 crate
(<https://github.com/reubeno/brush>, MIT) with **one patch, to one grammar rule pair**, so that
`for ((;;))` parses. Everything under `src/` is upstream's byte for byte apart from the two hunks
below. `Cargo.toml` is upstream's crates.io-normalised manifest minus build plumbing rush does not
use; the deletions are listed in a comment at the top of that file.

The fork is meant to be temporary. **The exit condition is the upstream PR below landing in a
release**; when it does, delete this directory and put `brush-parser = "0.x"` back in the root
`Cargo.toml`.

To confirm nothing else has drifted:

```sh
diff -ru "$(cargo pkgid brush-parser | sed 's/.*#//' >/dev/null; \
            ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/brush-parser-0.4.0)/src" \
         vendor/brush-parser/src
```

The only output should be the two hunks in `src/parser/peg.rs` reproduced below.

## The bug

```
$ rush -c 'for ((;;)); do break; done'
syntax error at line 1 col 9
$ rush -c 'for (( ; ; )); do break; done'      # works
$ rush -c 'for ((i=0;;i++)); do break; done'
syntax error at line 1 col 20
```

`;;` is in the tokenizer's operator table, and the tokenizer takes the longest match. When an
arithmetic `for` loop has an **empty condition**, its two section separators end up adjacent in the
source, so they come out as a single `;;` operator token rather than two `;` tokens:

```
for ((;;))       ->  for  (  (  ;;  )  )
for ((i=0;;i++)) ->  for  (  (  i=0  ;;  i++  )  )
```

`arithmetic_for_clause` asks for `specific_operator(";")` twice, and neither request matches a `;;`
token, so the clause fails. Any spelling with a space between the separators tokenizes as two `;`
and has always worked, which is why this reads as an arbitrary whitespace sensitivity from the
outside. It is not restricted to the infinite loop: `for ((i=0;;i++))` — increment with no bound,
a normal thing to write — fails the same way.

## Why the grammar and not the tokenizer

`;;` terminates a `case` item. It is that far more often than it is a fused pair of loop
separators. Teaching the tokenizer to split `;;` into two `;` — or to suppress the longest match
inside `((`/`))` — would put every `case` in every script downstream of the change, and the
tokenizer has no reliable notion of "inside an arithmetic for header" to condition on: it does not
know whether the `((` it saw belongs to `for ((`, to an arithmetic command, or to nested subshells
that the parser will only later backtrack into.

Recognising the fused token in the grammar keeps the change inside the two rules that want it, and
the `case` path never reaches either of them.

## The patch

Two hunks in `src/parser/peg.rs`.

**1. `arithmetic_end` must stop an expression at `;;`.** Without this, the alternative in hunk 2
cannot help: `arithmetic_expression` is `$(arithmetic_expression_piece()*)`, and a piece consumes
any token that is not an arithmetic end or a stray `)`. A `;;` is neither, so the initializer of
`for ((i=0;;i++))` swallows `;; i++` whole and then finds `))` — and a PEG repetition does not give
tokens back, so no later alternative can recover them.

```diff
         // TODO(arithmetic): evaluate arithmetic end; the semicolon is used in arithmetic for loops.
         rule arithmetic_end() -> () =
             specific_operator(")") specific_operator(")") {} /
-            specific_operator(";") {}
+            specific_operator(";") {} /
+            // An arithmetic for loop with an empty condition puts its two section separators next
+            // to each other, and the tokenizer's longest match fuses `;` `;` into one `;;`
+            // operator. Ending the expression here is what lets `for ((i=0;;i++))` split into
+            // sections at all; without it the initializer would swallow the `;;` and everything
+            // after it, because a repetition in a PEG does not backtrack.
+            specific_operator(";;") {}
```

**2. An alternative to `arithmetic_for_clause` for the fused separator.** Two adjacent separators
can only mean the condition is empty, so this alternative yields `condition: None` and is otherwise
the same clause.

```diff
                 let end = &body.loc;
                 let loc = SourceSpan::within(start, end);
                 ast::ArithmeticForClauseCommand { initializer, condition, updater, body, loc }
+            } /
+            // Same clause, but with an empty condition written without a space between the two
+            // section separators: `for ((;;))`, `for ((i=0;;i++))`. The tokenizer takes the
+            // longest match and emits one `;;` operator for them, so the alternative above never
+            // sees the two `;` tokens it asks for. Recognising `;;` here — rather than teaching
+            // the tokenizer to split it — keeps the change inside the one rule that wants it, and
+            // so cannot disturb the `;;` that terminates a `case` item.
+            //
+            // Two adjacent separators can only mean an empty condition; `for ((a;b;;))` is a
+            // syntax error in bash too, and stays one here because the updater section would then
+            // have to start with `;`.
+            s:specific_word("for")
+            specific_operator("(") specific_operator("(")
+                initializer:arithmetic_expression()? specific_operator(";;")
+                updater:arithmetic_expression()?
+            specific_operator(")") specific_operator(")")
+            body:arithmetic_for_body() {
+                let start = s.location();
+                let end = &body.loc;
+                let loc = SourceSpan::within(start, end);
+                ast::ArithmeticForClauseCommand { initializer, condition: None, updater, body, loc }
             }
 
         rule arithmetic_for_body() -> ast::DoGroupCommand =
```

The original alternative is tried first and is unchanged, so every spaced form takes exactly the
path it took before.

## What it does not change

- `case x in a) echo A;; b) echo B;; esac` — `case_item_post_action` still matches the same `;;`
  token; neither patched rule is on the `case` path.
- `for ((a;b;;))` stays a syntax error, as it is in bash: the first alternative stops at the `;;`
  where it wants a `;`, and the second wants the `;;` one section earlier.
- `((;;))` as a bare arithmetic command becomes a parse error instead of a runtime "operand
  expected". Both are diagnosed failures; bash reports it at runtime too.

## Suggested upstream tests

To go in `src/parser/tests/compound_commands.rs` beside `parse_arithmetic_for_empty_parts`
(snapshots generated with `cargo insta accept`). They are not in this vendored copy, because the
copy ships no snapshots for them and rush never runs brush's own test suite.

```rust
#[test]
fn parse_arithmetic_for_unspaced_empty_parts() -> Result<()> {
    let input = "for ((;;)); do echo loop; done";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult { input, result: &result });
    Ok(())
}

#[test]
fn parse_arithmetic_for_unspaced_empty_condition() -> Result<()> {
    let input = "for ((i=0;;i++)); do echo $i; done";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult { input, result: &result });
    Ok(())
}

#[test]
fn parse_case_with_arithmetic_body() -> Result<()> {
    // Regression guard for the fix above: `;;` must still terminate a case item.
    let input = "case $x in a) ((i++));; b) echo B;; esac";
    let result = test_with_snapshot(input)?;
    assert_snapshot_redacted!(ParseResult { input, result: &result });
    Ok(())
}
```

## Suggested PR description

> **Title:** parser: accept `for ((;;))` when the fused `;;` is two loop separators
>
> **Problem**
>
> `for ((;;))` and `for ((i=0;;i++))` are syntax errors, while `for (( ; ; ))` and
> `for (( i=0 ; ; i++ ))` parse. `;;` is in the operator table and the tokenizer takes the longest
> match, so whenever an arithmetic `for` loop has an empty condition its two section separators
> come out as one `;;` token. `arithmetic_for_clause` asks for `specific_operator(";")` twice and
> neither request matches, so the clause fails.
>
> The unspaced spelling is the common one — `for ((;;))` is how an infinite loop is normally
> written — and the failure is not limited to it: `for ((i=0;;i++))` is an ordinary unbounded
> counting loop and fails identically.
>
> **Fix**
>
> Two hunks in `src/parser/peg.rs`, both in the grammar:
>
> 1. `arithmetic_end` gains a `;;` alternative, so an arithmetic section stops at a fused
>    separator. This is load-bearing rather than cosmetic: `arithmetic_expression` is a PEG
>    repetition and does not backtrack, so without it the initializer consumes the `;;` and
>    everything after it before any later alternative gets a chance.
> 2. `arithmetic_for_clause` gains an alternative matching `init? ;; updater?`, yielding
>    `condition: None`. Two adjacent separators can only mean an empty condition.
>
> The pre-existing alternative is tried first and is untouched, so spaced forms parse exactly as
> before.
>
> **Why not the tokenizer**
>
> Splitting `;;` in the tokenizer, or suppressing the longest match inside `((`/`))`, would put
> every `case` item terminator downstream of the change, and the tokenizer cannot tell a `for ((`
> header from an arithmetic command or from nested subshells the parser has yet to backtrack into.
> Doing it in the grammar confines the change to the two rules that need it; the `case` path
> reaches neither.
>
> **Tests**
>
> Snapshot tests for `for ((;;))` and `for ((i=0;;i++))`, plus a `case` regression test whose item
> body ends in `))` so the `;;` sits directly against a closing paren.
>
> `for ((a;b;;))` remains a syntax error, matching bash.
