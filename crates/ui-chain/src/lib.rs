//! ui-chain: what a view cell reads from the caller's verified chain, and `layer_view`, the row filter.
//!
//! Lifted from repo:control-room crates/cr-view (first edition). Rights come from what ce-gate verified, never from a
//! request field. Row caveats are abilities of the chain, signed along it, so a page can neither add nor drop one:
//! - `<cell>:where:<key>=<v>` (equality) or `<cell>:where:<key>=<v1>|<v2>` (set): only rows whose `key` is one of them;
//! - `<cell>:scope:<key>=<v>`, the first edition's spelling, read as the same equality caveat (v1 callers keep it).
//! A wildcard right (`*`) carries no caveat and invents none.
//!
//! Never droppable: ce-gate ([`ce_gate::row_caveats_inherited`], rust-stack d283bd5) hands the cell the leaf's
//! abilities with the chain's effective row caveats, per cell and key the intersection of every link's (spelled
//! `where`), and every row caveat reaches the cell (none is cut off by the 64-ability limit). A delegate that drops
//! `cr.devices:where:node=..` still carries it; one that names a wider value is held to its parent's. So a cell that
//! filters with [`Caveats::of`] never runs unfiltered on a dimension any link of the chain caveats.
//!
//! Sources: a view cell calls its sources with its own key ([`cell_key`]), never the node's: the node mints each cell
//! one grant for exactly the abilities it reads with (`ce-iam grant --to <cell key> --action .. --action delegate:1`),
//! kept beside the cell's key in `<ce data dir>/cell-keys/<api>/` (`identity/node.key`, `grant.hex`).
//! [`source_token`] attenuates that grant to one source's abilities for a short time. A taken backend holds its own
//! narrow grant, not the node's root.
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

/// The allowed values of `key` in the caller's row caveats for `cell` (both spellings, `where` and `scope`, as
/// [`Caveats::of`] reads them; the gate hands effective caveats over as `where`), in order. Empty: not scoped on
/// `key`. A caveat whose links share no value reads as a value no row has, so it still hides every row.
pub fn scopes(c: &Caller, cell: &str, key: &str) -> Vec<String> {
    match Caveats::of(c, cell).0.remove(key) {
        None => vec![],
        Some(v) if v.is_empty() => vec![String::from("\u{0}none")],
        Some(v) => v,
    }
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

/// The proof this cell presents to source `A`: its own grant ([`cell_key`]) attenuated by its own key to exactly
/// `abilities` (never `<api>:*`), for `secs`, optionally confined to `path_prefix`. What a view cell may read is this
/// list, in its code, and never more than the node granted it. Never the node's key: before [`init`] it is an error.
pub fn source_token<A: Api>(abilities: &[&str], secs: u64, path_prefix: Option<&str>) -> Result<Vec<u8>, String> {
    let k = cell_key()?;
    Ok(ce_gate::proof::<A>(&k.id, &k.attenuate(abilities, secs, path_prefix)?))
}

pub use cell_key::{cell_key, init, node_id, CellKey};

/// A view cell's own key and the grant its node minted for it.
pub mod cell_key {
    use ce_gate::ce_cap::{ability_covers, Caveats, SignedCapability};
    use ce_gate::ce_identity::{Identity, NodeId};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    pub struct CellKey {
        pub api: String,
        pub id: Identity,
        /// root (this node) -> this cell's key: the abilities the cell reads its sources with, and `delegate:1`
        pub grant: Vec<SignedCapability>,
    }

    static KEY: OnceLock<CellKey> = OnceLock::new();

    /// Where cell `api` keeps its key and grant: `$CE_CELL_KEY_DIR`, else `<ce data dir>/cell-keys/<api>`.
    pub fn dir(api: &str) -> PathBuf {
        std::env::var("CE_CELL_KEY_DIR").ok().filter(|d| !d.is_empty()).map(PathBuf::from)
            .unwrap_or_else(|| ce_wire::ce_data_dir().join("cell-keys").join(api))
    }

    /// Load this process's cell key once (at the top of `serve`): `api`'s key and grant from [`dir`].
    pub fn init(api: &str) -> Result<&'static CellKey, String> {
        if let Some(k) = KEY.get() {
            return if k.api == api { Ok(k) } else { Err(format!("cell key already loaded for {}, not {api}", k.api)) };
        }
        let k = CellKey::load(api, &dir(api))?;
        Ok(KEY.get_or_init(|| k))
    }

    /// The key [`init`] loaded. None loaded: an error, never a fall back to the node's key.
    pub fn cell_key() -> Result<&'static CellKey, String> {
        KEY.get().ok_or_else(|| "no cell key loaded (ui_chain::init(<api>) at start; the node's own key is never used)".into())
    }

    /// This node's id (the root of the cell's grant): what the cell's own gate roots chains at.
    pub fn node_id() -> Result<NodeId, String> {
        Ok(cell_key()?.root())
    }

    impl CellKey {
        /// `<dir>/identity/node.key` (32 bytes, the layout `ce-iam --data-dir <dir>` keeps) and `<dir>/grant.hex` (the
        /// chain `ce-iam grant --to <its id>` printed). Refused unless the grant's leaf is held by that key.
        pub fn load(api: &str, dir: &Path) -> Result<CellKey, String> {
            let kf = dir.join("identity/node.key");
            let b = std::fs::read(&kf).map_err(|e| format!("{api}: no cell key at {}: {e}", kf.display()))?;
            let b: [u8; 32] = b.try_into().map_err(|_| format!("{}: not a 32-byte key", kf.display()))?;
            let id = Identity::from_secret_bytes(&b);
            let gf = dir.join("grant.hex");
            let hex = std::fs::read_to_string(&gf).map_err(|e| format!("{api}: no grant at {}: {e}", gf.display()))?;
            let grant = ce_gate::chain_from_hex(hex.trim()).map_err(|e| format!("{}: {e}", gf.display()))?;
            CellKey::new(api, id, grant)
        }

        pub fn new(api: &str, id: Identity, grant: Vec<SignedCapability>) -> Result<CellKey, String> {
            let leaf = grant.last().ok_or_else(|| format!("{api}: an empty grant"))?;
            if leaf.cap.audience != id.node_id() {
                return Err(format!("{api}: the grant is not held by this cell's key"));
            }
            if grant.iter().flat_map(|l| &l.cap.abilities).any(|a| a == "*") {
                return Err(format!("{api}: a cell grant names its abilities, never *"));
            }
            Ok(CellKey { api: api.into(), id, grant })
        }

        /// The issuer of the grant's first link: the node.
        pub fn root(&self) -> NodeId {
            self.grant[0].cap.issuer
        }

        /// The grant plus one link, signed by this cell's key to itself: exactly `abilities` (each covered by the
        /// grant's leaf, else refused here), `secs` long (never past the grant), optionally under `path_prefix`.
        pub fn attenuate(&self, abilities: &[&str], secs: u64, path_prefix: Option<&str>) -> Result<Vec<SignedCapability>, String> {
            let leaf = self.grant.last().expect("checked in new");
            if let Some(a) = abilities.iter().find(|a| !leaf.cap.abilities.iter().any(|g| ability_covers(g, a))) {
                return Err(format!("{}: {a} is not in this cell's grant", self.api));
            }
            let now = crate::now_s() as u64;
            let end = match leaf.cap.caveats.not_after {
                0 => now + secs,
                n => (now + secs).min(n),
            };
            let cv = Caveats { not_after: end, path_prefix: path_prefix.map(String::from).or_else(|| leaf.cap.caveats.path_prefix.clone()), ..leaf.cap.caveats.clone() };
            let mut n = [0u8; 8];
            getrandom::getrandom(&mut n).map_err(|e| format!("os random: {e}"))?;
            let mut out = self.grant.clone();
            out.push(SignedCapability::issue(&self.id, self.id.node_id(), abilities.iter().map(|a| a.to_string()).collect(), leaf.cap.resource.clone(), cv, u64::from_le_bytes(n), Some(leaf.id())));
            Ok(out)
        }
    }
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
