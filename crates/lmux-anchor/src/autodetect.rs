//! Autodetect matcher — given a pane's command line + environment, returns
//! the first [`lmux_config::AutodetectRule`] that matches, if any.
//!
//! The matcher is pure, synchronous, and does no IO. Epic 7.

use lmux_config::AutodetectRule;

/// Result of a successful match: the rule name and whether the anchor
/// should be hidden on session close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutodetectMatch<'a> {
    pub rule_name: &'a str,
    pub hide_on_session_close: bool,
}

/// Walk `rules` in order and return the first one that matches `command`
/// and `env_keys`.
pub fn match_rule<'a>(
    rules: &'a [AutodetectRule],
    command: &str,
    env_keys: &[&str],
) -> Option<AutodetectMatch<'a>> {
    rules
        .iter()
        .find(|r| r.matches(command, env_keys))
        .map(|r| AutodetectMatch {
            rule_name: &r.name,
            hide_on_session_close: r.hide_on_session_close,
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use lmux_config::MatchSpec;

    fn rule(name: &str, needles: &[&str]) -> AutodetectRule {
        AutodetectRule {
            name: name.into(),
            match_: MatchSpec {
                command_contains: needles.iter().map(|s| s.to_string()).collect(),
                env_set: vec![],
            },
            hide_on_session_close: false,
        }
    }

    #[test]
    fn returns_first_match() {
        let rules = vec![rule("cargo", &["cargo build"]), rule("npm", &["npm run"])];
        let m = match_rule(&rules, "cargo build --release", &[]).expect("match");
        assert_eq!(m.rule_name, "cargo");
    }

    #[test]
    fn none_when_no_rule_matches() {
        let rules = vec![rule("cargo", &["cargo build"])];
        assert!(match_rule(&rules, "ls -la", &[]).is_none());
    }

    #[test]
    fn env_match_works() {
        let rules = vec![AutodetectRule {
            name: "envtag".into(),
            match_: MatchSpec {
                command_contains: vec![],
                env_set: vec!["LMUX_ANCHOR".into()],
            },
            hide_on_session_close: true,
        }];
        let m = match_rule(&rules, "node server.js", &["LMUX_ANCHOR"]).expect("match");
        assert_eq!(m.rule_name, "envtag");
        assert!(m.hide_on_session_close);
    }
}
