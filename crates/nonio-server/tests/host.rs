//! Host profiles. The original only tells Codex from Claude Code (`hostOf` in host.ts); these tests are new.

use std::time::Duration;

use nonio_server::host::{host_of, Asks, HostId, CODEX_CLIENT};

#[test]
fn knows_claude_code_and_codex_by_the_name_they_send() {
    let cc = host_of(Some("claude-code"), None);
    assert_eq!(cc.id, HostId::ClaudeCode);
    assert!(cc.wake);
    assert_eq!(cc.page.asks, Asks::Page);
    assert_eq!(cc.max_output, None);
    let codex = host_of(Some(CODEX_CLIENT), None);
    assert_eq!(codex.id, HostId::Codex);
    assert!(!codex.wake);
    assert_eq!(codex.max_wait, Duration::from_secs(40));
    assert_eq!(codex.max_output, Some(28_000));
}

#[test]
fn puts_a_client_it_does_not_know_on_the_base_protocol() {
    for name in [None, Some(""), Some("some-new-agent"), Some("Claude Code")] {
        let h = host_of(name, None);
        assert_eq!(h.id, HostId::Base, "{name:?}");
        assert!(!h.wake);
        assert_eq!(h.page.asks, Asks::Chat);
        assert_eq!(h.max_output, Some(28_000));
        assert_eq!(h.repo_from_env, None);
    }
}

#[test]
fn takes_a_forced_profile_over_the_client_name_and_ignores_one_it_does_not_know() {
    assert_eq!(host_of(Some("claude-code"), Some("cursor")).id, HostId::Cursor);
    assert_eq!(host_of(None, Some(" codex ")).id, HostId::Codex);
    assert_eq!(host_of(Some("claude-code"), Some("vim")).id, HostId::ClaudeCode);
}

#[test]
fn keeps_cursor_on_the_base_protocol_until_it_is_measured() {
    let cursor = host_of(None, Some("cursor"));
    let base = host_of(None, None);
    assert_eq!(
        (cursor.wake, cursor.max_wait, cursor.max_output, cursor.page.asks, cursor.embedded_view),
        (base.wake, base.max_wait, base.max_output, base.page.asks, base.embedded_view)
    );
}
