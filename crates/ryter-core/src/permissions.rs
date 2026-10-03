//! `[permissions]`: what the build and test hats ask about, as rules the
//! user writes. The gate's fixed layers (what a hat may do at all, secrets,
//! privilege) are not rules; these decide the rest, where the shipped
//! answer is "run it" and the few asks can be turned off one by one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a rule says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// Runs.
    Allow,
    /// A person is asked.
    Ask,
    /// Refused.
    Deny,
}

/// The user's rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// What the build hat does with the project's files: `allow` (the
    /// default) or `ask`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<Answer>,
    /// Patterns over a command, `*` for any run of characters and `?` for
    /// one: `"git push*" = "ask"`. The most specific pattern that matches
    /// wins; at a tie, the stricter answer.
    pub bash: BTreeMap<String, Answer>,
}

impl Permissions {
    /// The rule for `command`, if one matches. `command` is one command
    /// as it runs: a segment of a longer line, or the whole line.
    pub fn for_command(&self, command: &str) -> Option<Answer> {
        let command = command.trim();
        self.bash
            .iter()
            .filter(|(pattern, _)| glob_matches(pattern, command))
            .max_by(|(pa, aa), (pb, ab)| {
                specificity(pa)
                    .cmp(&specificity(pb))
                    .then(strictness(**aa).cmp(&strictness(**ab)))
            })
            .map(|(_, a)| *a)
    }

    /// Whether any rule is set.
    pub fn is_empty(&self) -> bool {
        self.edit.is_none() && self.bash.is_empty()
    }
}

/// How much of a pattern is spelled out: the characters that aren't
/// wildcards. `git push*` beats `git *`, which beats `*`.
fn specificity(pattern: &str) -> usize {
    pattern.chars().filter(|c| !matches!(c, '*' | '?')).count()
}

fn strictness(a: Answer) -> u8 {
    match a {
        Answer::Allow => 0,
        Answer::Ask => 1,
        Answer::Deny => 2,
    }
}

/// `pattern` against `text`, `*` any run (including none), `?` one
/// character, everything else itself.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    // Classic two-pointer glob with backtracking to the last `*`.
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_as_a_shell_would() {
        for (p, t, want) in [
            ("git push*", "git push origin main", true),
            ("git push*", "git pushing", true),
            ("git push*", "git pull", false),
            ("*", "", true),
            ("*", "anything at all", true),
            ("rm -rf ?", "rm -rf x", true),
            ("rm -rf ?", "rm -rf xy", false),
            ("*prune*", "docker system prune -af", true),
            ("cargo *", "cargo", false),
            ("npm run *", "npm run build && echo", true),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
        ] {
            assert_eq!(glob_matches(p, t), want, "{p:?} vs {t:?}");
        }
    }

    #[test]
    fn the_most_specific_rule_wins_and_the_stricter_at_a_tie() {
        let text = r#"
            [bash]
            "*" = "allow"
            "git *" = "ask"
            "git push*" = "allow"
            "git push --force*" = "deny"
            "rm *" = "ask"
            "rm -rf target" = "allow"
        "#;
        let p: Permissions = toml::from_str(text).unwrap();
        assert_eq!(p.for_command("git status"), Some(Answer::Ask));
        assert_eq!(p.for_command("git push origin"), Some(Answer::Allow));
        assert_eq!(p.for_command("git push --force origin"), Some(Answer::Deny));
        assert_eq!(p.for_command("rm -rf target"), Some(Answer::Allow));
        assert_eq!(p.for_command("rm -rf src"), Some(Answer::Ask));
        assert_eq!(p.for_command("ls"), Some(Answer::Allow));
        assert_eq!(p.for_command("  ls  "), Some(Answer::Allow));
        // Two patterns as specific as each other: the stricter answer.
        let text = r#"
            [bash]
            "cargo publish*" = "allow"
            "*cargo publish" = "deny"
        "#;
        let p: Permissions = toml::from_str(text).unwrap();
        assert_eq!(p.for_command("cargo publish"), Some(Answer::Deny));
        // No rule, no answer.
        assert_eq!(Permissions::default().for_command("ls"), None);
        assert!(Permissions::default().is_empty());
        // `edit` on its own.
        let p: Permissions = toml::from_str("edit = \"ask\"\n").unwrap();
        assert_eq!(p.edit, Some(Answer::Ask));
    }
}
