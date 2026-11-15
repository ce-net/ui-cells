//! ui.manifest (`Resolve` only) and ui.serve's check, as one library the gateway links.
//!
//! A site declares its route and slot tree once ([`Site`], built by a site crate from the cells' `VIEWS`). The build
//! writes one bundle per view, named by its sha256 ([`Bundles`], `bundles.json`, never sent to a browser). For one
//! viewer, [`resolve`] keeps only the views whose `needs` the viewer holds, with their slot place, file and digest;
//! a view the viewer may not have is not in the answer at all, so its name, hash and slot never reach the browser.
//! [`may_fetch`] is ui.serve's rule: a bundle file is given only to a viewer whose manifest would list it.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ui_part::ViewShape;

/// One place in the tree: a view of a cell, and the slots under it.
#[derive(Debug, Clone)]
pub struct Place {
    pub view: ViewShape,
    pub slots: Vec<(&'static str, Place)>,
}

#[derive(Debug, Clone)]
pub struct Site {
    pub name: &'static str,
    pub route: &'static str,
    pub root: Place,
}

/// `bundles.json`: `<api>@<major>/<View>` -> file and digest, written by the build.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bundles(pub BTreeMap<String, Bundle>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bundle {
    /// `_c/<sha256 hex>.js`, relative to the page
    pub file: String,
    /// hex sha256 of the file's bytes
    pub sha256: String,
}

pub fn key(v: &ViewShape) -> String {
    format!("{}@{}/{}", v.api, v.major, v.name)
}

/// One module the page may load.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Module {
    pub api: String,
    pub major: u32,
    pub view: String,
    /// "" for the root; else the parent's path plus "/" plus the slot name (e.g. "keys", "renewals/keys")
    pub slot: String,
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub site: String,
    pub route: String,
    pub modules: Vec<Module>,
}

/// Why a view is not in a manifest (for tests and the server's own log; never sent to the viewer).
#[derive(Debug, Clone, PartialEq)]
pub enum Left {
    NoRight { slot: String, needs: String },
    NoBundle { slot: String, key: String },
}

/// The manifest for a viewer holding the rights `has` says yes to. A place whose view is refused takes its whole
/// subtree with it (a slot inside a panel the viewer cannot open is not reachable either).
pub fn resolve(site: &Site, b: &Bundles, has: &dyn Fn(&str) -> bool) -> (Manifest, Vec<Left>) {
    let mut modules = Vec::new();
    let mut left = Vec::new();
    walk(&site.root, "", b, has, &mut modules, &mut left);
    (Manifest { site: site.name.into(), route: site.route.into(), modules }, left)
}

fn walk(p: &Place, slot: &str, b: &Bundles, has: &dyn Fn(&str) -> bool, out: &mut Vec<Module>, left: &mut Vec<Left>) {
    if !has(p.view.needs) {
        left.push(Left::NoRight { slot: slot.into(), needs: p.view.needs.into() });
        return;
    }
    let Some(f) = b.0.get(&key(&p.view)) else {
        left.push(Left::NoBundle { slot: slot.into(), key: key(&p.view) });
        return;
    };
    out.push(Module { api: p.view.api.into(), major: p.view.major, view: p.view.name.into(), slot: slot.into(), file: f.file.clone(), sha256: f.sha256.clone() });
    for (name, child) in &p.slots {
        let s = if slot.is_empty() { name.to_string() } else { format!("{slot}/{name}") };
        walk(child, &s, b, has, out, left);
    }
}

/// ui.serve: may this viewer fetch bundle `file`? Only when its own manifest lists it.
pub fn may_fetch(site: &Site, b: &Bundles, has: &dyn Fn(&str) -> bool, file: &str) -> bool {
    resolve(site, b, has).0.modules.iter().any(|m| m.file == file)
}

/// Every bundle file the site knows (the gateway answers any other `_c/` path 404, these 403 when refused).
pub fn known(b: &Bundles, file: &str) -> bool {
    b.0.values().any(|x| x.file == file)
}

/// Check a site tree against its cells' views: each slot's declared `<api>@<major>` is the view placed in it.
pub fn check(site: &Site) -> Result<(), String> {
    fn go(p: &Place) -> Result<(), String> {
        for (name, child) in &p.slots {
            let want = p.view.slots.iter().find(|(s, _)| s == name).map(|(_, c)| *c)
                .ok_or_else(|| format!("{} has no slot {name}", p.view.name))?;
            let have = format!("{}@{}", child.view.api, child.view.major);
            if want != have {
                return Err(format!("slot {name} of {} takes {want}, the site puts {have} there", p.view.name));
            }
            go(child)?;
        }
        Ok(())
    }
    go(&site.root)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod shell {
        ce_wire::api! { api S = "s.shell", version 2; types { pub struct L { pub t: String } }
            ops { 1 => Layout(String) -> L, needs "s.shell:open"; } }
        ui_part::views! { api S;
            view Room { props {} needs "s.shell:open"; events {} data { layout: Layout } streams {} slots { keys: "s.keys@2", devices: "s.devices@2" } speaks [2]; }
            parts {} }
    }
    mod keys {
        ce_wire::api! { api K = "s.keys", version 2; types { pub struct R { pub n: String } }
            ops { 1 => List(()) -> Vec<R>, needs "s.keys:read"; } }
        ui_part::views! { api K; view KeyList { props {} needs "s.keys:read"; events {} data { rows: List } streams {} slots {} speaks [2]; } parts {} }
    }
    mod devices {
        ce_wire::api! { api D = "s.devices", version 2; types { pub struct R { pub n: String } }
            ops { 1 => List(()) -> Vec<R>, needs "s.devices:read"; } }
        ui_part::views! { api D; view DeviceList { props {} needs "s.devices:read"; events {} data { rows: List } streams {} slots {} speaks [2]; } parts {} }
    }

    fn site() -> Site {
        let leaf = |v: ViewShape| Place { view: v, slots: vec![] };
        Site { name: "t", route: "/x/t/*", root: Place { view: shell::VIEWS[0], slots: vec![("keys", leaf(keys::VIEWS[0])), ("devices", leaf(devices::VIEWS[0]))] } }
    }

    fn bundles() -> Bundles {
        let mut m = BTreeMap::new();
        for (k, h) in [("s.shell@2/Room", "aa"), ("s.keys@2/KeyList", "bb"), ("s.devices@2/DeviceList", "cc")] {
            m.insert(k.to_string(), Bundle { file: format!("_c/{h}.js"), sha256: h.into() });
        }
        Bundles(m)
    }

    #[test]
    fn two_viewers_two_manifests() {
        let (root, _) = resolve(&site(), &bundles(), &|_| true);
        assert_eq!(root.modules.iter().map(|m| m.slot.as_str()).collect::<Vec<_>>(), ["", "keys", "devices"]);
        let op = |r: &str| r == "s.shell:open" || r == "s.devices:read";
        let (m, left) = resolve(&site(), &bundles(), &op);
        assert_eq!(m.modules.iter().map(|m| m.view.as_str()).collect::<Vec<_>>(), ["Room", "DeviceList"]);
        let text = serde_json::to_string(&m).unwrap();
        assert!(!text.contains("keys") && !text.contains("bb"), "{text}");
        assert_eq!(left, vec![Left::NoRight { slot: "keys".into(), needs: "s.keys:read".into() }]);
        assert!(!may_fetch(&site(), &bundles(), &op, "_c/bb.js"));
        assert!(may_fetch(&site(), &bundles(), &op, "_c/cc.js"));
        let (none, _) = resolve(&site(), &bundles(), &|r| r == "s.keys:read");
        assert!(none.modules.is_empty(), "no shell, no panels under it");
    }

    #[test]
    fn site_tree_matches_slot_types() {
        assert!(check(&site()).is_ok());
        let mut s = site();
        s.root.slots[0].1 = Place { view: devices::VIEWS[0], slots: vec![] };
        assert!(check(&s).unwrap_err().contains("takes s.keys@2"));
    }
}
