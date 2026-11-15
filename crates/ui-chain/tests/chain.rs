//! Through a real ce-gate: row caveats are inherited along the whole chain, never dropped, only narrowed (the control
//! room's drop probe, 2026-11-15: root -> A with `cr.devices:where:node=..` -> B without it saw all 5 devices and 400
//! audit rows; now B sees A's narrow rows), and a view cell's
//! sources are read with its own key's narrow grant, never the node's key.
use ce_gate::ce_cap::{Caveats as CapCaveats, Resource, SignedCapability};
use ce_gate::ce_identity::Identity;
use ce_gate::{proof, Gate};
use ce_wire::blocking;
use ui_chain::{layer_view, CellKey, Caveats};

mod api {
    ce_wire::api! {
        api Devices = "ui-chain.test.devices", version 1;
        types { pub struct Q { pub v: u8 } }
        ops { 1 => List(Q) -> Vec<String>, needs "ui-chain.test.devices:read"; }
    }
}
use api::*;

const CELL: &str = "ui-chain.test.devices";
const NODES: &[&str] = &["MacBook", "c0be11e0", "debian", "foreman-ops-1", "leif-desk"];

#[derive(Clone)]
struct Dev(&'static str);
impl layer_view::Row for Dev {
    fn caveat(&self, k: &str) -> Option<String> {
        (k == "node").then(|| self.0.to_string())
    }
}

fn ab(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn cv() -> CapCaveats {
    CapCaveats { not_after: u64::MAX, ..Default::default() }
}

/// A cell that answers with the device names the caller's caveats let through (what cr.devices List does).
fn serve(node: &Identity) -> std::net::SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    let rows: Vec<Dev> = NODES.iter().map(|n| Dev(n)).collect();
    let s = blocking::Server::<Devices>::new(Gate::<Devices>::new(node.node_id(), vec![]).into_fn())
        .on::<List, _>(move |_: Q, c| Ok(layer_view::filter(&Caveats::of(c, CELL), &rows).into_iter().map(|d| d.0.to_string()).collect()));
    std::thread::spawn(move || s.serve(l));
    a
}

fn list(a: std::net::SocketAddr, who: &Identity, chain: &[SignedCapability]) -> Result<Vec<String>, String> {
    let c = blocking::Client::<Devices>::connect(a, proof::<Devices>(who, chain)).map_err(|e| format!("{e:?}"))?;
    c.call::<List>(&Q { v: 1 }).map_err(|e| format!("{e:?}"))
}

#[test]
fn the_drop_probe_sees_only_the_narrow_rows() {
    std::env::set_var("CE_DATA_DIR", std::env::temp_dir().join("ui-chain-test-none"));
    let node = Identity::from_secret_bytes(&[31; 32]);
    let a = Identity::from_secret_bytes(&[32; 32]);
    let b = Identity::from_secret_bytes(&[33; 32]);
    let addr = serve(&node);
    // as cr-shell's mint: the operator's abilities with the node caveat
    let l0 = SignedCapability::issue(&node, a.node_id(), ab(&["ui-chain.test.devices:read", "ui-chain.test.devices:where:node=foreman-ops-1", "delegate:1"]), Resource::Any, cv(), 1, None);
    assert_eq!(list(addr, &a, &[l0.clone()]).unwrap(), vec!["foreman-ops-1"]);

    // B, A's child, with the caveat dropped: it inherits A's, so only the narrow rows, never all five
    let l1 = SignedCapability::issue(&a, b.node_id(), ab(&["ui-chain.test.devices:read"]), Resource::Any, cv(), 2, Some(l0.id()));
    assert_eq!(list(addr, &b, &[l0.clone(), l1]).unwrap(), vec!["foreman-ops-1"]);

    // a re-delegation that adds a wider caveat beside the parent's: ignored, the narrow rows
    let wide_parent = SignedCapability::issue(&node, a.node_id(), ab(&["ui-chain.test.devices:*", "ui-chain.test.devices:where:node=foreman-ops-1", "delegate:1"]), Resource::Any, cv(), 3, None);
    let wide = SignedCapability::issue(&a, b.node_id(), ab(&["ui-chain.test.devices:read", "ui-chain.test.devices:where:node=foreman-ops-1", "ui-chain.test.devices:where:node=*"]), Resource::Any, cv(), 4, Some(wide_parent.id()));
    assert_eq!(list(addr, &b, &[wide_parent.clone(), wide]).unwrap(), vec!["foreman-ops-1"]);
    // replacing it with a wider one, or with another node: never more than the parent's
    let wide = SignedCapability::issue(&a, b.node_id(), ab(&["ui-chain.test.devices:read", "ui-chain.test.devices:where:node=*"]), Resource::Any, cv(), 5, Some(wide_parent.id()));
    assert_eq!(list(addr, &b, &[wide_parent.clone(), wide]).unwrap(), vec!["foreman-ops-1"]);
    let other = SignedCapability::issue(&a, b.node_id(), ab(&["ui-chain.test.devices:read", "ui-chain.test.devices:where:node=debian"]), Resource::Any, cv(), 6, Some(wide_parent.id()));
    assert!(list(addr, &b, &[wide_parent.clone(), other]).unwrap().is_empty(), "disjoint caveats pass no row");
    // the v1 reader sees the inherited caveat too
    let c = ce_wire::Caller { name: String::new(), rights: vec!["ability:ui-chain.test.devices:where:node=".into()] };
    assert!(!ui_chain::in_scope(&ui_chain::scopes(&c, CELL, "node"), "foreman-ops-1"));
}

#[test]
fn sources_are_read_with_the_cells_own_narrow_grant() {
    std::env::set_var("CE_DATA_DIR", std::env::temp_dir().join("ui-chain-test-none"));
    let node = Identity::from_secret_bytes(&[41; 32]);
    let cell = Identity::from_secret_bytes(&[42; 32]);
    let addr = serve(&node);
    // before init: no token, and no fall back to any node key
    assert!(ui_chain::source_token::<Devices>(&["ui-chain.test.devices:read"], 60, None).unwrap_err().contains("no cell key"));

    let grant = vec![SignedCapability::issue(&node, cell.node_id(), ab(&["ui-chain.test.devices:read", "delegate:1"]), Resource::Any, CapCaveats::default(), 7, None)];
    let k = CellKey::new(CELL, Identity::from_secret_bytes(&[42; 32]), grant.clone()).unwrap();
    assert_eq!(k.root(), node.node_id());
    let chain = k.attenuate(&["ui-chain.test.devices:read"], 60, None).unwrap();
    assert_eq!(chain.len(), 2);
    assert!(chain[1].cap.caveats.not_after > 0, "the presented link is short-lived");
    assert_eq!(list(addr, &cell, &chain).unwrap().len(), 5, "an uncaveated cell grant reads every row");
    // never beyond the grant
    assert!(k.attenuate(&["ui-chain.test.devices:write"], 60, None).unwrap_err().contains("not in this cell's grant"));
    assert!(k.attenuate(&["keys:read"], 60, None).is_err());
    // a grant held by another key, or a `*` grant, is refused at load
    assert!(CellKey::new(CELL, Identity::from_secret_bytes(&[43; 32]), grant).is_err());
    let star = vec![SignedCapability::issue(&node, cell.node_id(), ab(&["*"]), Resource::Any, CapCaveats::default(), 8, None)];
    assert!(CellKey::new(CELL, Identity::from_secret_bytes(&[42; 32]), star).is_err());

    // from files, as the node lays them out
    let dir = std::env::temp_dir().join(format!("ui-chain-cell-key-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("identity")).unwrap();
    std::fs::write(dir.join("identity/node.key"), [42u8; 32]).unwrap();
    let g = vec![SignedCapability::issue(&node, cell.node_id(), ab(&["ui-chain.test.devices:read", "delegate:1"]), Resource::Any, CapCaveats::default(), 9, None)];
    std::fs::write(dir.join("grant.hex"), ce_gate::ce_cap::encode_chain(&g)).unwrap();
    std::env::set_var("CE_CELL_KEY_DIR", &dir);
    let k = ui_chain::init(CELL).unwrap();
    assert_eq!(ui_chain::node_id().unwrap(), node.node_id());
    let tok = ui_chain::source_token::<Devices>(&["ui-chain.test.devices:read"], 60, None).unwrap();
    let c = blocking::Client::<Devices>::connect(addr, tok).unwrap();
    assert_eq!(c.call::<List>(&Q { v: 1 }).unwrap().len(), 5);
    assert!(ui_chain::init("cr.other").is_err(), "one cell key per process");
    assert_eq!(k.id.node_id(), cell.node_id());
    let _ = std::fs::remove_dir_all(dir);
}
