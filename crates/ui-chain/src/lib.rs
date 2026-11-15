//! ui-chain: what a view cell reads from the caller's verified chain, and `layer_view`, the row filter.
//!
//! Lifted from repo:control-room crates/cr-view (first edition). Rights come from what ce-gate verified, never from a
//! request field. Row caveats are leaf abilities of the chain, signed by the node that minted it, so a page can neither
//! add nor drop one:
//! - `<cell>:where:<key>=<v>` (equality) or `<cell>:where:<key>=<v1>|<v2>` (set): only rows whose `key` is one of them;
//! - `<cell>:scope:<key>=<v>`, the first edition's spelling, read as the same equality caveat (v1 callers keep it).
//! A wildcard right (`*`) carries no caveat and invents none. Until cap v3 is in ce-gate, this crate is the caveat check.
#![forbid(unsafe_code)]

use ce_wire::{Api, Caller};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn now_s() -> i64 {
    (now_ms() / 1000) as i64
}

/// Does the caller's chain hold `ability` (an op right the gate matched, or a verified leaf ability covering it)?
pub fn holds(c: &Caller, ability: &str) -> bool {
    c.has(ability) || c.grants(ability)
}

/// The values of every `<cell>:scope:<key>=<value>` ability in the caller's verified leaf, in order.
/// Empty: the caller is not scoped on `key`.
pub fn scopes(c: &Caller, cell: &str, key: &str) -> Vec<String> {
    let head = format!("{}{cell}:scope:{key}=", ce_wire::ABILITY_RIGHT);
    c.rights.iter().filter_map(|r| r.strip_prefix(head.as_str())).filter(|v| !v.is_empty()).map(String::from).collect()
}

/// `pattern` matches `value`: equal, `*`, or a trailing `*` prefix match.
pub fn glob(pattern: &str, value: &str) -> bool {
    pattern == "*" || pattern == value || pattern.strip_suffix('*').is_some_and(|p| value.starts_with(p))
}

/// A row with `value` for this scope key is visible: unscoped, or one scope matches.
pub fn in_scope(scopes: &[String], value: &str) -> bool {
    scopes.is_empty() || scopes.iter().any(|s| glob(s, value))
}

/// The id of the chain the caller came in with (the gate's `grant:<16 hex>`), "" when the gate gave none.
pub fn chain_id(c: &Caller) -> String {
    c.rights.iter().find_map(|r| r.strip_prefix(ce_wire::GRANT_RIGHT)).unwrap_or("").to_string()
}

/// 16 random bytes as 32 hex: a call id or span id in `ui.trace`'s shape.
pub fn new_id() -> String {
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).is_err() {
        let t = now_ms().to_le_bytes();
        b[..8].copy_from_slice(&t);
        b[8..].copy_from_slice(&(std::process::id() as u64).to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The grant this cell presents to source `A`: a chain this node signs for itself with exactly `abilities` (never
/// `<api>:*`), for `secs`, optionally confined to `path_prefix`. What a view cell may read is this list, in its code.
pub fn source_token<A: Api>(abilities: &[&str], secs: u64, path_prefix: Option<&str>) -> Result<Vec<u8>, String> {
    let me = ce_gate::node_identity().map_err(|e| format!("this machine has no ce identity: {e}"))?;
    let chain = ce_gate::self_chain_in(&me, abilities.iter().map(|a| a.to_string()).collect(), secs, path_prefix.map(String::from));
    Ok(ce_gate::proof::<A>(&me, &chain))
}

/// This machine's short host name (the node name the rows use for the local node).
pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok()).unwrap_or_else(|| "localhost".into())
}

/// The row caveats of `cell` in the caller's chain: key -> allowed values. Empty: unscoped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caveats(pub BTreeMap<String, Vec<String>>);

impl Caveats {
    pub fn of(c: &Caller, cell: &str) -> Caveats {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for spelling in ["where", "scope"] {
            let head = format!("{}{cell}:{spelling}:", ce_wire::ABILITY_RIGHT);
            for r in c.rights.iter().filter_map(|r| r.strip_prefix(head.as_str())) {
                let Some((k, v)) = r.split_once('=') else { continue };
                let vals = m.entry(k.to_string()).or_default();
                vals.extend(v.split('|').filter(|x| !x.is_empty()).map(String::from));
            }
        }
        Caveats(m)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A row whose `key` is `value` passes: the key is not caveated, or one allowed value matches (glob `*` suffix).
    pub fn allows(&self, key: &str, value: &str) -> bool {
        self.0.get(key).is_none_or(|vs| vs.iter().any(|p| glob(p, value)))
    }
}

/// layer-view: a row type names the value of each caveat key it can be filtered on.
pub mod layer_view {
    use super::Caveats;

    pub trait Row {
        /// The row's value for caveat `key`, or None when the row has no such key (then a caveat on it hides the row:
        /// fail closed).
        fn caveat(&self, key: &str) -> Option<String>;
    }

    pub fn visible<R: Row>(cv: &Caveats, r: &R) -> bool {
        visible_by(cv, |k| r.caveat(k))
    }

    /// The same check for a row type this crate cannot implement `Row` for: `value(key)` is the row's value.
    pub fn visible_by(cv: &Caveats, value: impl Fn(&str) -> Option<String>) -> bool {
        cv.0.keys().all(|k| value(k).is_some_and(|v| cv.allows(k, &v)))
    }

    /// The rows the caveats let through, in order.
    pub fn filter<R: Row + Clone>(cv: &Caveats, rows: &[R]) -> Vec<R> {
        rows.iter().filter(|r| visible(cv, *r)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(rights: &[&str]) -> Caller {
        Caller { name: "ab".repeat(32), rights: rights.iter().map(|r| r.to_string()).collect() }
    }

    #[test]
    fn scopes_are_read_from_leaf_abilities_only() {
        let c = caller(&["cr.devices:read", "ability:cr.devices:read", "ability:cr.devices:scope:node=node-1", "cr.devices:scope:node=x"]);
        assert_eq!(scopes(&c, "cr.devices", "node"), vec!["node-1".to_string()]);
        assert!(scopes(&c, "cr.devices", "app").is_empty());
    }

    #[test]
    fn a_wildcard_right_is_unscoped_but_never_a_scope() {
        let c = caller(&["ability:*"]);
        assert!(holds(&c, "cr.keys:holders"));
        assert!(scopes(&c, "cr.keys", "service").is_empty());
    }

    #[test]
    fn where_and_set_caveats_and_the_v1_spelling() {
        let c = caller(&["ability:cr.devices:where:node=ops-1|leif-desk", "ability:cr.audit:scope:app=orch", "cr.devices:where:node=x"]);
        let d = Caveats::of(&c, "cr.devices");
        assert!(d.allows("node", "ops-1") && d.allows("node", "leif-desk") && !d.allows("node", "debian"));
        assert!(d.allows("other", "anything"));
        assert!(Caveats::of(&c, "cr.audit").allows("app", "orch"));
        assert!(!Caveats::of(&c, "cr.audit").allows("app", "keys"));
        assert!(Caveats::of(&caller(&["ability:*"]), "cr.keys").is_empty());
    }

    #[derive(Clone)]
    struct R(&'static str);
    impl layer_view::Row for R {
        fn caveat(&self, k: &str) -> Option<String> {
            (k == "node").then(|| self.0.to_string())
        }
    }

    #[test]
    fn layer_view_filters_and_fails_closed() {
        let cv = Caveats::of(&caller(&["ability:cr.devices:where:node=ops-1"]), "cr.devices");
        let rows = layer_view::filter(&cv, &[R("ops-1"), R("debian")]);
        assert_eq!(rows.len(), 1);
        let cv = Caveats::of(&caller(&["ability:cr.devices:where:color=red"]), "cr.devices");
        assert!(layer_view::filter(&cv, &[R("ops-1")]).is_empty());
    }

    #[test]
    fn glob_and_scope() {
        assert!(glob("stripe*", "stripe-live"));
        assert!(!glob("stripe*", "github"));
        assert!(in_scope(&[], "anything"));
        assert!(!in_scope(&["a".into()], "b"));
    }

    #[test]
    fn chain_id_from_gate_right() {
        assert_eq!(chain_id(&caller(&["grant:0123456789abcdef"])), "0123456789abcdef");
        assert_eq!(chain_id(&caller(&[])), "");
    }

    #[test]
    fn ids_are_32_hex() {
        let a = new_id();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_id());
    }
}
