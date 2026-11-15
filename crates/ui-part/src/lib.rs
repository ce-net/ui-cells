//! ui-part: the `ui { }` half of a full-stack cell's contract, declared in the same `-api` crate right after its
//! `ce_wire::api!` block (`views!`), because `ce_wire::api!` at the pinned rev does not parse a `ui` block yet.
//!
//! A view names its props, the one right it needs, the events it emits, the ops its data comes from, the slots other
//! cells fill and the majors it speaks. Two checks run in `cargo build` of the contract crate:
//! - every data op exists in the api (a typo fails the build), and
//! - the view's `needs` is exactly the right each data op needs ("a view that needs less than Watch needs" fails).
//!
//! `parts` declares field-level rights: a field typed `Option<T>` in the contract is a `Part<T, right>`; the
//! generated `Parts::withhold` blanks it for a caller without the right, so it is absent on the wire, and
//! `ui-gen` types it as `Part<T>` in TypeScript, which has no direct value (read with `show`/`match`).
#![forbid(unsafe_code)]

pub use ce_wire;

/// One view of a cell, as `views!` declared it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewShape {
    pub api: &'static str,
    pub major: u32,
    pub name: &'static str,
    pub needs: &'static str,
    /// (prop, Rust type)
    pub props: &'static [(&'static str, &'static str)],
    /// (event, argument type)
    pub events: &'static [(&'static str, &'static str)],
    /// (data name, op name): one answer each
    pub data: &'static [(&'static str, &'static str)],
    /// (data name, stream op name, row key field): a snapshot, then deltas keyed by that field
    pub streams: &'static [(&'static str, &'static str, &'static str)],
    /// (slot, `<api>@<major>` it takes)
    pub slots: &'static [(&'static str, &'static str)],
    pub speaks: &'static [u32],
}

/// A field-level right: `ty.field` is sent only to a caller holding `right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartShape {
    pub ty: &'static str,
    pub field: &'static str,
    pub right: &'static str,
}

/// Blank every `Part` field the caller may not have.
pub trait Parts {
    fn withhold(&mut self, has: &dyn Fn(&str) -> bool);
}

pub const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Compile-time: the right a data op needs is exactly the view's `needs` (a data op that is not an op of the api
/// does not resolve, so a typo fails the build too).
pub const fn check(needs: &str, op_right: &str) {
    if !str_eq(needs, op_right) {
        panic!("views!: the view's needs differs from the right one of its data ops needs");
    }
}

/// `views! { api CrKeys; view KeyList { props {..} needs ".."; events {..} data {..} streams {..} slots {..} speaks [..]; } parts { KeyRow { holders: "cr.keys:holders" } } }`
#[macro_export]
macro_rules! views {
    (
        api $api:ident;
        $( view $view:ident {
            props { $( $pn:ident : $pt:ty ),* $(,)? }
            needs $needs:literal;
            events { $( $en:ident ( $et:ty ) );* $(;)? }
            data { $( $dn:ident : $dop:ident ),* $(,)? }
            streams { $( $wn:ident : $wop:ident by $wk:ident ),* $(,)? }
            slots { $( $sn:ident : $scell:literal ),* $(,)? }
            speaks [ $( $sp:literal ),* ];
        } )*
        parts { $( $pty:ident { $( $pf:ident : $pr:literal ),* $(,)? } )* }
    ) => {
        pub const VIEWS: &[$crate::ViewShape] = &[ $( $crate::ViewShape {
            api: <$api as $crate::ce_wire::Api>::NAME,
            major: <$api as $crate::ce_wire::Api>::VERSION,
            name: stringify!($view),
            needs: $needs,
            props: &[ $( (stringify!($pn), stringify!($pt)) ),* ],
            events: &[ $( (stringify!($en), stringify!($et)) ),* ],
            data: &[ $( (stringify!($dn), stringify!($dop)) ),* ],
            streams: &[ $( (stringify!($wn), stringify!($wop), stringify!($wk)) ),* ],
            slots: &[ $( (stringify!($sn), $scell) ),* ],
            speaks: &[ $( $sp ),* ],
        } ),* ];
        $( const _: () = {
            $( $crate::check($needs, <$dop as $crate::ce_wire::Op>::RIGHT); )*
            $( $crate::check($needs, <$wop as $crate::ce_wire::Stream>::RIGHT); )*
        }; )*
        pub const PARTS: &[$crate::PartShape] = &[ $( $( $crate::PartShape { ty: stringify!($pty), field: stringify!($pf), right: $pr } ),* ),* ];
        $( impl $crate::Parts for $pty {
            fn withhold(&mut self, has: &dyn Fn(&str) -> bool) {
                $( if !has($pr) { self.$pf = None; } )*
            }
        } )*
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_string_compare() {
        assert!(str_eq("cr.keys:read", "cr.keys:read"));
        assert!(!str_eq("cr.keys:read", "cr.keys:reaD"));
        assert!(!str_eq("a", "ab"));
    }
}

/// The same view with the right `needs` builds:
/// ```
/// mod c {
///     ui_part::ce_wire::api! { api T = "t.t", version 1; types { pub struct R { pub a: Option<u8> } }
///         ops { 1 => List(()) -> Vec<R>, needs "t.t:read"; } }
///     ui_part::views! { api T; view V { props {} needs "t.t:read"; events {} data { rows: List } streams {} slots {} speaks [1]; } parts {} }
/// }
/// ```
/// A view whose `needs` is narrower than its data op's right does not build:
/// ```compile_fail
/// mod c {
///     ui_part::ce_wire::api! { api T = "t.t", version 1; types { pub struct R { pub a: Option<u8> } }
///         ops { 1 => List(()) -> Vec<R>, needs "t.t:read"; } }
///     ui_part::views! { api T; view V { props {} needs "t.t:health"; events {} data { rows: List } streams {} slots {} speaks [1]; } parts {} }
/// }
/// ```
/// and a data op that is not an op of the api does not resolve:
/// ```compile_fail
/// mod c {
///     ui_part::ce_wire::api! { api T = "t.t", version 1; types { pub struct R { pub a: Option<u8> } }
///         ops { 1 => List(()) -> Vec<R>, needs "t.t:read"; } }
///     ui_part::views! { api T; view V { props {} needs "t.t:read"; events {} data { rows: Lisst } streams {} slots {} speaks [1]; } parts {} }
/// }
/// ```
pub mod break_proof {}

#[cfg(test)]
mod view_tests {
    mod c {
        crate::ce_wire::api! {
            api T = "t.t", version 2;
            types { pub struct R { pub name: String, pub secret_free: Option<Vec<u8>> } }
            ops { 1 => List(()) -> Vec<R>, needs "t.t:read"; }
            streams { 2 => Watch(()) -> stream R, needs "t.t:read"; }
        }
        crate::views! {
            api T;
            view V { props { q: Option<String> } needs "t.t:read"; events { opened(String) } data { rows: List } streams { live: Watch by name } slots { child: "x.y@1" } speaks [2]; }
            parts { R { secret_free: "t.t:more" } }
        }
    }

    #[test]
    fn views_and_parts() {
        use crate::Parts;
        let v = &c::VIEWS[0];
        assert_eq!((v.api, v.major, v.name, v.needs), ("t.t", 2, "V", "t.t:read"));
        assert_eq!(v.data, &[("rows", "List")]);
        assert_eq!(v.streams, &[("live", "Watch", "name")]);
        assert_eq!(v.slots, &[("child", "x.y@1")]);
        let mut r = c::R { name: "a".into(), secret_free: Some(vec![1]) };
        r.withhold(&|x| x == "t.t:read");
        assert_eq!(r.secret_free, None);
        let mut r = c::R { name: "a".into(), secret_free: Some(vec![1]) };
        r.withhold(&|x| x == "t.t:more");
        assert!(r.secret_free.is_some());
    }
}
