//! Console-configurable policy for how strictly the console treats a plugin's signature
//! (docs/PLUGIN_SIGNING.md, section 6). Like trust level (`plugin_trust`), this is a load
//! gate and a confirmation step, never something that grants or widens a permission.

/// The settings key (same generic `settings` table every other console-only setting uses --
/// see `src/api/routes/settings.rs`).
pub const SETTING_KEY: &str = "pluginSignaturePolicy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Unsigned and signed plugins are reviewed the same way (default).
    Allow,
    /// Approving an unsigned plugin needs an explicit confirmation on the review screen.
    Warn,
    /// Only a built-in or a signed plugin may load at all; an unsigned one can't even be approved.
    Require,
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Policy::Allow => "allow",
            Policy::Warn => "warn",
            Policy::Require => "require",
        }
    }

    /// Anything other than exactly "warn" or "require" is `Allow` -- the default, and what an
    /// older client or a typo falls back to, never a stricter reading.
    pub fn parse(raw: Option<&str>) -> Policy {
        match raw {
            Some("warn") => Policy::Warn,
            Some("require") => Policy::Require,
            _ => Policy::Allow,
        }
    }
}

/// Whether a plugin that is otherwise `active` (approved, files unchanged) is actually allowed
/// to load under `policy`. `require` is the only policy that can turn this false, and it never
/// touches a built-in: "only built-in and signed plugins load" means exactly that, not "and
/// built-ins have to be signed too."
pub fn permits_load(policy: Policy, builtin: bool, signed: bool) -> bool {
    builtin || signed || policy != Policy::Require
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalGate {
    /// Nothing extra needed.
    Allowed,
    /// The operator must explicitly confirm before this approval is accepted.
    NeedsConfirmation,
    /// Can't be approved at all until the policy is loosened or the plugin is signed.
    Refused,
}

/// What approving a plugin needs, given the console's policy, whether its folder is built-in,
/// whether it's signed, and whether this approval is about to grant a Critical permission.
/// Built-ins always sail through: policy and confirmation only ever apply to unsigned,
/// non-built-in plugins.
pub fn approval_gate(policy: Policy, builtin: bool, signed: bool, granting_critical: bool) -> ApprovalGate {
    if builtin || signed {
        return ApprovalGate::Allowed;
    }
    if policy == Policy::Require {
        return ApprovalGate::Refused;
    }
    if policy == Policy::Warn || granting_critical {
        return ApprovalGate::NeedsConfirmation;
    }
    ApprovalGate::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_falls_back_to_allow() {
        assert_eq!(Policy::parse(None), Policy::Allow);
        assert_eq!(Policy::parse(Some("")), Policy::Allow);
        assert_eq!(Policy::parse(Some("strict")), Policy::Allow, "a typo or an unknown future value is never read as stricter than the default");
        assert_eq!(Policy::parse(Some("warn")), Policy::Warn);
        assert_eq!(Policy::parse(Some("require")), Policy::Require);
    }

    #[test]
    fn require_blocks_only_unsigned_non_builtins_from_loading() {
        assert!(permits_load(Policy::Require, true, false), "a built-in never needs a signature");
        assert!(permits_load(Policy::Require, false, true), "signed is enough, whatever its trust level");
        assert!(!permits_load(Policy::Require, false, false));
        for p in [Policy::Allow, Policy::Warn] {
            assert!(permits_load(p, false, false), "{p:?} never blocks loading, only approving");
        }
    }

    #[test]
    fn approval_gate_never_touches_builtins_or_signed_plugins() {
        for policy in [Policy::Allow, Policy::Warn, Policy::Require] {
            for critical in [false, true] {
                assert_eq!(approval_gate(policy, true, false, critical), ApprovalGate::Allowed, "{policy:?}/{critical}");
                assert_eq!(approval_gate(policy, false, true, critical), ApprovalGate::Allowed, "{policy:?}/{critical}");
            }
        }
    }

    #[test]
    fn unsigned_non_builtin_follows_policy_and_criticality() {
        assert_eq!(approval_gate(Policy::Allow, false, false, false), ApprovalGate::Allowed);
        assert_eq!(approval_gate(Policy::Allow, false, false, true), ApprovalGate::NeedsConfirmation, "independent of policy: an unsigned plugin granted a Critical permission always needs confirmation");
        assert_eq!(approval_gate(Policy::Warn, false, false, false), ApprovalGate::NeedsConfirmation, "warn asks for confirmation on every unsigned plugin, critical or not");
        assert_eq!(approval_gate(Policy::Warn, false, false, true), ApprovalGate::NeedsConfirmation);
        assert_eq!(approval_gate(Policy::Require, false, false, false), ApprovalGate::Refused, "require never settles for a confirmation");
        assert_eq!(approval_gate(Policy::Require, false, false, true), ApprovalGate::Refused);
    }
}
