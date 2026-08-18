//! Which stream a coordinate reaches, and what a word becomes.

use super::*;

const HOSTS: &str = "web-01  10.0.0.1\nweb-02  10.0.0.2\ndb-01   10.0.0.9\n";

fn words(text: &str, streams: &Streams) -> Vec<String> {
    substitute(text, streams)
        .expect("a coordinate")
        .into_iter()
        .map(|runs| runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect()
}

/// **Zero and up walk back through this pipeline.** `{0:…}` is the stage that just finished, which
/// is the one feeding the command being built.
#[test]
fn a_positive_stream_walks_back_through_the_pipeline() {
    let mut streams = Streams::default();
    streams.push_stage(HOSTS);
    streams.push_stage("web-01  10.0.0.1\n");

    // 0 is the newest stage.
    assert_eq!(words("{0:0}", &streams), vec!["web-01"]);
    // 1 steps past it to the one before.
    assert_eq!(
        words("{1:*:0}", &streams),
        vec!["web-01", "web-02", "db-01"]
    );
    // Past the end of the pipeline is empty, not an error.
    assert_eq!(words("{9:0:0}", &streams), Vec::<String>::new());
}

/// **Below zero walks back through the session**, which is a different collection on purpose: one
/// axis would mean `{3:…}` silently crossing out of a short pipeline into the prompts behind it.
#[test]
fn a_negative_stream_walks_back_through_the_prompts() {
    let mut streams = Streams::default();
    streams.push_prompt(HOSTS);
    streams.push_prompt("only-this\n");

    assert_eq!(words("{-1:0:0}", &streams), vec!["only-this"]);
    assert_eq!(words("{-2:0:0}", &streams), vec!["web-01"]);
    assert_eq!(words("{-9:0:0}", &streams), Vec::<String>::new());
}

/// A finished command clears the pipeline it was in, because a coordinate counting forward in the
/// *next* command must not reach into a pipeline that is over.
#[test]
fn a_finished_prompt_clears_the_stages() {
    let mut streams = Streams::default();
    streams.push_stage(HOSTS);
    assert_eq!(words("{0:0}", &streams), vec!["web-01"]);

    streams.push_prompt("after\n");
    assert_eq!(words("{0:0}", &streams), Vec::<String>::new());
    assert_eq!(words("{-1:0:0}", &streams), vec!["after"]);
}

/// **A bare coordinate is one argument per value**, the way `"$@"` is — which is what makes
/// `ping {*:1}` one process with three arguments.
#[test]
fn a_bare_coordinate_becomes_one_word_per_value() {
    let mut streams = Streams::default();
    streams.push_stage(HOSTS);
    assert_eq!(
        words("{*:1}", &streams),
        vec!["10.0.0.1", "10.0.0.2", "10.0.0.9"]
    );
}

/// With text around it, the values join — `pre{*:0}post` has to stay one word to mean anything.
#[test]
fn a_coordinate_with_text_around_it_stays_one_word() {
    let mut streams = Streams::default();
    streams.push_stage(HOSTS);
    assert_eq!(words("host-{0:0}.lan", &streams), vec!["host-web-01.lan"]);
    assert_eq!(words("[{*:0}]", &streams), vec!["[web-01 web-02 db-01]"]);
}

/// **Every value is `Quoted`**, so a line with a space or a `*` in it arrives as one argument and
/// is never re-split or re-globbed. A shell that field-splits its own substitutions executes
/// filenames.
#[test]
fn a_value_is_one_argument_and_never_globs() {
    let mut streams = Streams::default();
    streams.push_stage("my file.txt  100\n*.rs  200\n");

    let runs = substitute("{0}", &streams).expect("a coordinate");
    assert_eq!(runs.len(), 1, "one line is one argument: {runs:?}");
    assert_eq!(runs[0][0].text, "my file.txt  100");
    assert_eq!(runs[0][0].origin, Origin::Quoted);

    // And the glob character survives as text rather than matching anything.
    let runs = substitute("{1:0}", &streams).expect("a coordinate");
    assert_eq!(runs[0][0].text, "*.rs");
    assert_eq!(runs[0][0].origin, Origin::Quoted);
}

/// The cheap pre-scan must not miss a coordinate, and must not claim an ordinary brace group.
#[test]
fn the_scan_claims_coordinates_and_leaves_brace_groups() {
    for yes in [
        "{0}", "{0:1}", "{-1}", "{*:0}", "x{0:1}y", "{..2:1}", "{:1}",
    ] {
        assert!(looks_like_a_coordinate(yes), "{yes:?} should be scanned");
    }
    for no in ["{a,b}", "{a..e}", "plain", "{}", "no braces here"] {
        assert!(!looks_like_a_coordinate(no), "{no:?} should be left alone");
    }
}

/// A brace group that is not a coordinate is refused outright, so brace expansion still gets it.
#[test]
fn a_brace_group_is_not_substituted() {
    let streams = Streams::default();
    assert!(substitute("{a,b}", &streams).is_none());
    assert!(substitute("{a..e}", &streams).is_none());
    assert!(substitute("plain", &streams).is_none());
}

/// Nothing captured reads empty rather than refusing to run.
#[test]
fn an_empty_stack_reads_empty() {
    let streams = Streams::default();
    assert_eq!(words("{0:0}", &streams), Vec::<String>::new());
    assert_eq!(words("{-1:0:0}", &streams), Vec::<String>::new());
}

/// A stream longer than the cap keeps its head, so `{0}` still answers and `{-1}` is honestly the
/// last line of what was kept.
#[test]
fn a_huge_stream_is_capped_at_its_head() {
    let mut streams = Streams::default();
    let huge = format!("first\n{}\n", "x".repeat(STREAM_MAX * 2));
    streams.push_stage(huge);
    assert_eq!(words("{0}", &streams), vec!["first"]);
}

/// Only ten prompts are kept; the eleventh pushes the oldest out.
///
/// Note the three dimensions throughout: `{-1:0:}` is *stream* −1, line 0, whole line. Two
/// dimensions would be line −1 and word 0 of this command's own input, which is a different
/// question and answers nothing here — the grammar is unambiguous but it does have to be counted.
#[test]
fn the_prompt_ring_is_bounded() {
    let mut streams = Streams::default();
    for n in 0..PROMPTS_KEPT + 5 {
        streams.push_prompt(format!("line-{n}\n"));
    }
    let last = PROMPTS_KEPT + 4;
    assert_eq!(words("{-1:0:}", &streams), vec![format!("line-{last}")]);
    let oldest_kept = last - (PROMPTS_KEPT - 1);
    assert_eq!(
        words(&format!("{{-{PROMPTS_KEPT}:0:}}"), &streams),
        vec![format!("line-{oldest_kept}")]
    );
    // One past the ring is gone.
    assert_eq!(
        words(&format!("{{-{}:0:}}", PROMPTS_KEPT + 1), &streams),
        Vec::<String>::new()
    );
}

/// **Two dimensions never reach a stream**, which is the counting rule stated as a test rather
/// than a comment: `{-1:0}` is line −1 word 0 of *this* input, not "the previous prompt".
#[test]
fn two_dimensions_stay_in_this_stream() {
    let mut streams = Streams::default();
    streams.push_prompt("from-the-prompt\n");
    streams.push_stage("a b\nc d\n");

    assert_eq!(words("{-1:0}", &streams), vec!["c"], "line -1, word 0");
    assert_eq!(
        words("{-1:0:}", &streams),
        vec!["from-the-prompt"],
        "stream -1, line 0"
    );
}
