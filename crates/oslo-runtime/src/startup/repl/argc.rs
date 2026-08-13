//! The completion provider that reads a script's `# @option` comments.

/// Register it, once, as the shell starts.
///
/// Native rather than Lua, and registered like any other provider: it competes on score, it carries
/// a kind so `oslo.completion.sources` can filter it, and a config that wants to replace it declares
/// one of the same name.
pub(super) fn register() {
    use oslo_ui::completion::provider::{DEFAULT_MAX_ITEMS, Provider, register};
    register(Provider {
        name: "argc".to_string(),
        kind: "argc".to_string(),
        // Every command, because which ones declare their arguments is a property of the script
        // rather than a list anybody could write down.
        when: None,
        score_offset: 0.0,
        max_items: DEFAULT_MAX_ITEMS,
        min_chars: 0,
        enabled: Some(std::rc::Rc::new(|ctx| {
            oslo_shell::argc::complete::worth_asking(&ctx.command)
        })),
        answer: std::rc::Rc::new(oslo_shell::argc::complete::offers),
    });
}
