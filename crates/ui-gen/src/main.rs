//! `ui-gen <contract lib.rs> --client ./CrKeys --rt <import path of ui-rt> -o <out.ts>`
//!
//! Reads the `ce_wire::api!` block (op names, request types, unit or not) and the `ui_part::views!` block of one
//! contract file and writes the TypeScript binding of every view: props, events, data, slots and source types, and
//! the loaders that call only this cell's ops through the ts-web client ce-wire-gen wrote. Every type in it is derived
//! from the client's own method signatures, so a renamed field or op fails `tsc` in the view that uses it.
#![forbid(unsafe_code)]

use std::fmt::Write;

#[derive(Debug, Default)]
struct View {
    name: String,
    needs: String,
    props: Vec<(String, String)>,
    events: Vec<(String, String)>,
    data: Vec<(String, String)>,
    streams: Vec<(String, String, String)>,
    slots: Vec<(String, String)>,
}

#[derive(Debug)]
struct OpDecl {
    name: String,
    req: String,
}

/// The text between the brace that opens after `from` and its matching close.
fn block(s: &str, from: usize) -> Option<(usize, usize)> {
    let open = from + s[from..].find('{')?;
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + i));
                }
            }
            _ => {}
        }
    }
    None
}

fn section<'a>(body: &'a str, word: &str) -> Option<&'a str> {
    let mut at = 0;
    while let Some(i) = body[at..].find(word) {
        let i = at + i;
        let before = body[..i].chars().last().unwrap_or(' ');
        let after = body[i + word.len()..].trim_start();
        if !(before.is_alphanumeric() || before == '_') && after.starts_with('{') {
            let (a, b) = block(body, i)?;
            return Some(&body[a..b]);
        }
        at = i + word.len();
    }
    None
}

/// Split on `sep` at depth 0 of <>, (), [].
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![];
    let mut d = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '<' | '(' | '[' => d += 1,
            '>' | ')' | ']' => d -= 1,
            _ => {}
        }
        if c == sep && d == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out.into_iter().filter(|x| !x.is_empty()).collect()
}

fn strip_comments(s: &str) -> String {
    s.lines().map(|l| if let Some(i) = l.find("//") { &l[..i] } else { l }).collect::<Vec<_>>().join("\n")
}

fn ops(src: &str) -> (String, Vec<OpDecl>) {
    let at = src.find("ce_wire::api!").expect("no ce_wire::api! block");
    let (a, b) = block(src, at).expect("unclosed api!");
    let body = strip_comments(&src[a..b]);
    let ident = body.trim_start().strip_prefix("api").unwrap_or("").split('=').next().unwrap_or("").trim().to_string();
    let mut out = vec![];
    for sec in ["ops", "streams"] {
        let Some(s) = section(&body, sec) else { continue };
        for line in s.split(';') {
            let Some((_, rest)) = line.split_once("=>") else { continue };
            let rest = rest.trim();
            let Some(p) = rest.find('(') else { continue };
            let name = rest[..p].trim().to_string();
            let mut d = 0;
            let mut end = p;
            for (i, c) in rest[p..].char_indices() {
                match c {
                    '(' => d += 1,
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            end = p + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push(OpDecl { name, req: rest[p + 1..end].trim().to_string() });
        }
    }
    (ident, out)
}

fn views(src: &str) -> Vec<View> {
    let Some(at) = src.find("ui_part::views!") else { return vec![] };
    let (a, b) = block(src, at).expect("unclosed views!");
    let body = strip_comments(&src[a..b]);
    let mut out = vec![];
    let mut at = 0;
    while let Some(i) = body[at..].find("view ") {
        let i = at + i;
        let name = body[i + 5..].split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_string();
        let Some((va, vb)) = block(&body, i) else { break };
        let v = &body[va..vb];
        let mut view = View { name, ..Default::default() };
        view.needs = v.split("needs").nth(1).and_then(|x| x.split('"').nth(1)).unwrap_or("").to_string();
        for p in split_top(section(v, "props").unwrap_or(""), ',') {
            if let Some((n, t)) = p.split_once(':') {
                view.props.push((n.trim().into(), t.trim().into()));
            }
        }
        for e in split_top(section(v, "events").unwrap_or(""), ';') {
            if let Some((n, t)) = e.split_once('(') {
                view.events.push((n.trim().into(), t.trim_end_matches(')').trim().into()));
            }
        }
        for d in split_top(section(v, "data").unwrap_or(""), ',') {
            if let Some((n, o)) = d.split_once(':') {
                view.data.push((n.trim().into(), o.trim().into()));
            }
        }
        for d in split_top(section(v, "streams").unwrap_or(""), ',') {
            if let Some((n, rest)) = d.split_once(':') {
                let mut w = rest.split_whitespace();
                let op = w.next().unwrap_or("").to_string();
                let key = w.nth(1).unwrap_or("").to_string();
                view.streams.push((n.trim().into(), op, key));
            }
        }
        for s in split_top(section(v, "slots").unwrap_or(""), ',') {
            if let Some((n, c)) = s.split_once(':') {
                view.slots.push((n.trim().into(), c.trim().trim_matches('"').into()));
            }
        }
        out.push(view);
        at = vb;
    }
    out
}

fn camel(op: &str) -> String {
    let mut c = op.chars();
    c.next().map(|f| f.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn ts(t: &str) -> String {
    let t = t.trim();
    if let Some(inner) = t.strip_prefix("Option<").and_then(|x| x.strip_suffix('>')) {
        return format!("{} | null", ts(inner));
    }
    if let Some(inner) = t.strip_prefix("Vec<").and_then(|x| x.strip_suffix('>')) {
        return format!("readonly ({})[]", ts(inner));
    }
    match t {
        "String" | "str" | "&str" => "string".into(),
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" | "f32" | "f64" => "number".into(),
        "u64" | "i64" | "u128" | "i128" => "bigint".into(),
        "bool" => "boolean".into(),
        "()" => "null".into(),
        other => format!("T.{other}"),
    }
}

const PRIMS: &[&str] = &["String", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "bool", "f32", "f64"];

fn req_expr(op: &OpDecl, v: &View) -> String {
    if op.req == "()" {
        String::new()
    } else if PRIMS.contains(&op.req.as_str()) {
        v.props.first().map(|(n, _)| format!("p.{n}")).unwrap_or_else(|| "null".into())
    } else {
        format!("{{ {} }}", v.props.iter().map(|(n, _)| format!("{n}: p.{n}")).collect::<Vec<_>>().join(", "))
    }
}

fn gen(file: &str, src: &str, client: &str, rt: &str) -> Result<String, String> {
    let (ident, ops) = ops(src);
    let vs = views(src);
    if vs.is_empty() {
        return Err(format!("{file}: no views! block"));
    }
    let op = |n: &str| ops.iter().find(|o| o.name == n).ok_or_else(|| format!("{file}: views! names op {n}, which the api does not have"));
    let mut o = String::new();
    let api_name = src.split(&format!("api {ident} = \"")).nth(1).and_then(|x| x.split('"').next()).unwrap_or("");
    let major = src.split(&format!("api {ident} = \"{api_name}\", version ")).nth(1).and_then(|x| x.split(|c: char| !c.is_ascii_digit()).next()).unwrap_or("0");
    writeln!(o, "// Generated by ui-gen from the views! block of {file} ({api_name}@{major}). Do not edit; regenerate.").unwrap();
    writeln!(o, "/* eslint-disable */").unwrap();
    let mut body = String::new();
    for v in &vs {
        let n = &v.name;
        writeln!(o, "\n// ---- view {n} (needs {}) ----", v.needs).unwrap();
        writeln!(o, "export interface {n}Props {{ {} }}", v.props.iter().map(|(p, t)| format!("readonly {p}: {};", ts(t))).collect::<Vec<_>>().join(" ")).unwrap();
        writeln!(o, "export interface {n}Events {{ {} }}", v.events.iter().map(|(e, t)| format!("{e}(a: {}): void;", ts(t))).collect::<Vec<_>>().join(" ")).unwrap();
        let mut src_lines = vec![];
        let mut data_lines = vec![];
        for (d, opn) in &v.data {
            let od = op(opn)?;
            let m = camel(opn);
            let arg = if od.req == "()" { String::new() } else { format!("req: Parameters<Client['{m}']>[0]") };
            src_lines.push(format!("  {m}({arg}): ReturnType<Client['{m}']>;"));
            data_lines.push(format!("  readonly {d}: Awaited<ReturnType<Client['{m}']>>;"));
        }
        for (d, opn, _) in &v.streams {
            let od = op(opn)?;
            let m = camel(opn);
            let arg = if od.req == "()" { String::new() } else { format!("req: Parameters<Client['{m}']>[0]") };
            src_lines.push(format!("  {m}({arg}): Feed<ItemOf<ReturnType<Client['{m}']>>>;"));
            data_lines.push(format!("  readonly {d}: readonly RowOf<ItemOf<ReturnType<Client['{m}']>>>[];"));
        }
        src_lines.sort();
        src_lines.dedup();
        writeln!(o, "export interface {n}Source {{\n{}\n}}", src_lines.join("\n")).unwrap();
        writeln!(o, "export interface {n}Data {{\n{}\n}}", data_lines.join("\n")).unwrap();
        let slots = if v.slots.is_empty() { "never".to_string() } else { v.slots.iter().map(|(s, _)| format!("'{s}'")).collect::<Vec<_>>().join(" | ") };
        // slot names are types only (erased from the bundle): the view places the slots its backend names, so a viewer
        // never receives the name of a slot it may not open
        writeln!(o, "/** slots: {} */", v.slots.iter().map(|(s, c)| format!("{s} = cell:{c}")).collect::<Vec<_>>().join(", ")).unwrap();
        writeln!(o, "export type {n}Slot = {slots};").unwrap();
        let defaults = v.props.iter().map(|(p, t)| format!("{p}: {}", if t.starts_with("Option<") { "null" } else { "undefined as never" })).collect::<Vec<_>>().join(", ");
        writeln!(o, "export const {n} = defineView<{n}Props, {n}Data, {n}Events, {n}Source, {n}Slot>({{").unwrap();
        writeln!(o, "  api: API, major: MAJOR, view: '{n}', needs: '{}',", v.needs).unwrap();
        writeln!(o, "  events: [{}],", v.events.iter().map(|(e, _)| format!("'{e}'")).collect::<Vec<_>>().join(", ")).unwrap();
        writeln!(o, "  defaults: {{ {defaults} }},").unwrap();
        writeln!(o, "  connect: (g: Grant) => {ident}Client.connect(g),").unwrap();
        writeln!(o, "  load: (src, p, set, fail, gone, again) => {{").unwrap();
        writeln!(o, "    const stops = [").unwrap();
        for (d, opn, key) in &v.streams {
            let od = op(opn)?;
            writeln!(o, "      watch(() => src.{}({}), (r) => String(r.{key}), (v) => set('{d}', v), fail, gone, again),", camel(opn), req_expr(od, v)).unwrap();
        }
        for (d, opn) in &v.data {
            let od = op(opn)?;
            writeln!(o, "      once(() => src.{}({}), (v) => set('{d}', v), fail, gone, again),", camel(opn), req_expr(od, v)).unwrap();
        }
        writeln!(o, "    ];\n    return () => {{ for (const s of stops) s(); }};\n  }},\n}});").unwrap();
    }
    std::mem::swap(&mut o, &mut body);
    let split = body.match_indices('\n').nth(1).map(|(i, _)| i + 1).unwrap_or(0);
    o.push_str(&body[..split]);
    let body = body[split..].to_string();
    // import only what the bindings use (the views are checked with noUnusedLocals)
    let mut rt_names = vec!["defineView", "type Grant"];
    let uses = |w: &str| body.contains(w);
    for (w, n) in [("once(", "once"), ("watch(", "watch"), ("Feed<", "type Feed"), ("ItemOf<", "type ItemOf"), ("RowOf<", "type RowOf")] {
        if uses(w) {
            rt_names.push(n);
        }
    }
    writeln!(o, "import {{ {ident}Client }} from '{client}';").unwrap();
    if uses("T.") {
        writeln!(o, "import type * as T from '{client}';").unwrap();
    }
    writeln!(o, "import {{ {} }} from '{rt}';", rt_names.join(", ")).unwrap();
    writeln!(o, "type Client = {ident}Client;").unwrap();
    writeln!(o, "export const API = '{api_name}';\nexport const MAJOR = {major};").unwrap();
    o.push_str(&body);
    Ok(o)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let (Some(file), Some(out)) = (a.first().cloned(), get("-o")) else {
        eprintln!("usage: ui-gen <contract lib.rs> --client ./CrKeys --rt <ui-rt import> -o <out.ts>");
        std::process::exit(2);
    };
    let client = get("--client").unwrap_or_else(|| "./Client".into());
    let rt = get("--rt").unwrap_or_else(|| "@ui/rt".into());
    let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file}: {e}"));
    match gen(&file, &src, &client, &rt) {
        Ok(t) => {
            std::fs::write(&out, t).unwrap_or_else(|e| panic!("{out}: {e}"));
            eprintln!("ui-gen: {out}");
        }
        Err(e) => {
            eprintln!("ui-gen: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
ce_wire::api! {
    api CrKeys = "cr.keys", version 2;
    types { pub struct KeysQuery { pub q: Option<String> } }
    ops { 1 => HealthOp(()) -> Health, needs "cr.keys:health"; 6 => Sources(()) -> Vec<Source>, needs "cr.keys:read"; }
    streams { 3 => Watch(KeysQuery) -> stream KeyDelta, needs "cr.keys:read"; }
}
ui_part::views! {
    api CrKeys;
    view KeyList {
        props { q: Option<String> }
        needs "cr.keys:read";
        events { opened(String); renew(String) }
        data { sources: Sources }
        streams { rows: Watch by name }
        slots {}
        speaks [2];
    }
    parts { KeyRow { holders: "cr.keys:holders" } }
}
"#;

    #[test]
    fn binds_a_view() {
        let t = gen("lib.rs", SRC, "./CrKeys", "@ui/rt").unwrap();
        assert!(t.contains("cr.keys@2"));
        assert!(t.contains("export interface KeyListProps { readonly q: string | null; }"));
        assert!(t.contains("opened(a: string): void; renew(a: string): void;"));
        assert!(t.contains("watch(() => src.watch({ q: p.q }), (r) => String(r.name), (v) => set('rows', v), fail, gone, again)"));
        assert!(t.contains("once(() => src.sources(), (v) => set('sources', v), fail, gone, again)"));
        assert!(t.contains("export type KeyListSlot = never;"));
    }

    #[test]
    fn an_unknown_op_is_an_error() {
        let bad = SRC.replace("sources: Sources", "sources: Sourcez");
        assert!(gen("lib.rs", &bad, "./CrKeys", "@ui/rt").unwrap_err().contains("Sourcez"));
    }
}
