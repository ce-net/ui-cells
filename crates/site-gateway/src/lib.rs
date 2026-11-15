//! site.gateway (minimum) and ui.serve, loopback only. Lifted from repo:control-room crates/cr-shell/src/web.rs.
//!
//! The apps.ce-net.com sign-in gate sends `/<prefix>/*` here with the prefix cut and `X-Ce-Person` set from the
//! session, signed with the gate's per-host key (`X-Ce-Person-Sig`, hex HMAC-SHA256 over `ce-person\n<person>\n<at>`),
//! so no other local process can speak for a person. The same check as repo:shell-grant/crates/shell-grant/src/web.rs.
//!
//! Routes:
//! - `GET /session`: mints the person's grant (their role's abilities, from the site's policy) and answers `302` to
//!   `./#g=<grant>&w=<door>`; the grant rides in the fragment, which a browser never sends to a server.
//! - `GET /manifest.json`: ui.manifest `Resolve` for this person: only the views their rights may receive.
//! - `GET /_c/<sha256>.js`: ui.serve: a bundle, only to a person whose manifest lists it (`403 Refused` otherwise,
//!   `404` for a name no bundle has). `bundles.json`, the full index, is never served.
//! - `/wire`: piped byte for byte to wire.ws-door, where each cell's own gate checks every Hello.
//! - anything else: the site's static files (the page, `_k/` kit assets), the same for every signed-in person.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use ui_manifest::{Bundles, Site};

/// Who a person is to this site: their role and the abilities their grant carries (from the site's policy).
#[derive(Debug, Clone, PartialEq)]
pub struct Viewer {
    pub role: String,
    pub abilities: Vec<String>,
}

pub trait Mint {
    /// A link `<page>#g=...` for a fresh browser key holding `abilities` for `ttl_s`, and its end (unix seconds).
    fn mint(&self, page: &str, abilities: &[String], ttl_s: u32) -> Result<(String, u64), String>;
}

pub struct Web {
    pub dir: PathBuf,
    pub wire: String,
    pub mint: Arc<dyn Mint + Send + Sync>,
    pub person_key: PathBuf,
    /// person -> Viewer, or why they get nothing (no role: `Err` starting with "refused")
    pub viewer: Arc<dyn Fn(&str) -> Result<Viewer, String> + Send + Sync>,
    pub site: Arc<Site>,
    pub ttl_s: u32,
    /// The public page and its door, used when the request came through the gate.
    pub page: String,
    pub door: String,
}

/// An ability covers a right: equal, `*`, or `<domain>:*` (ce-cap's rule), never a caveat ability.
pub fn covers(abilities: &[String], right: &str) -> bool {
    abilities.iter().any(|a| a == right || a == "*" || a.strip_suffix(":*").is_some_and(|d| right.split(':').next() == Some(d)))
}

/// Read `bundles.json` from the built site.
pub fn bundles(dir: &Path) -> Bundles {
    std::fs::read(dir.join("bundles.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}


pub const SIG_MAX_AGE: u64 = 60;
const HEAD_MAX: usize = 16 * 1024;

pub fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

/// The person the gate signed into this request head, or None (unsigned, bad signature, older than SIG_MAX_AGE).
pub fn signed_person(head: &str, key: &[u8], now: u64) -> Option<String> {
    use hmac::{Hmac, Mac};
    let person = header(head, "x-ce-person").filter(|p| !p.is_empty())?;
    let at = header(head, "x-ce-person-at")?;
    let sig = header(head, "x-ce-person-sig")?;
    let t: u64 = at.parse().ok()?;
    if t.abs_diff(now) > SIG_MAX_AGE || sig.len() != 64 {
        return None;
    }
    let want: Vec<u8> = (0..32).map(|i| u8::from_str_radix(sig.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<_>>()?;
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(key).ok()?;
    m.update(format!("ce-person\n{person}\n{at}").as_bytes());
    m.verify_slice(&want).ok()?;
    Some(person.to_string())
}

pub fn read_key(p: &Path) -> Option<Vec<u8>> {
    let s = std::fs::read_to_string(p).ok()?;
    let s = s.trim();
    (s.len() == 64).then(|| (0..32).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect::<Option<Vec<u8>>>())?
}

fn kind(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// The file a GET path names inside dir: plain names only, no "..", no hidden files.
pub fn file_of(dir: &Path, path: &str) -> Option<PathBuf> {
    let p = path.split(['?', '#']).next().unwrap_or("/");
    let p = if p.ends_with('/') { format!("{p}index.html") } else { p.to_string() };
    let rel = p.strip_prefix('/').unwrap_or("");
    if rel.is_empty() || rel.split('/').any(|s| s.is_empty() || s.starts_with('.')) || !rel.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
        return None;
    }
    Some(dir.join(rel))
}

/// One strict policy for every page: scripts, styles and fonts from this origin only, sockets only to the door.
pub fn csp(door: &str) -> String {
    let origin = door.rsplit_once('/').map(|(a, _)| a).unwrap_or(door);
    let origin = origin.rsplit_once('/').filter(|(a, _)| a.contains("//")).map(|(a, _)| a).unwrap_or(origin);
    format!("default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self' {origin} {door}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
}

async fn reply(s: &mut TcpStream, status: &str, ty: &str, extra: &str, body: &[u8]) {
    let h = format!("HTTP/1.1 {status}\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{extra}Connection: close\r\n\r\n", body.len());
    let _ = s.write_all(h.as_bytes()).await;
    let _ = s.write_all(body).await;
}

/// Where this request came from decides the door: through the gate (`Host: apps.ce-net.com`) the public door, on
/// loopback the same port's `/wire`.
fn door_for(w: &Web, head: &str) -> (String, String) {
    let host = header(head, "host").unwrap_or("");
    if host.starts_with("127.0.0.1") || host.starts_with("localhost") {
        (format!("http://{host}/"), format!("ws://{host}/wire"))
    } else {
        (w.page.clone(), w.door.clone())
    }
}

async fn one(w: Arc<Web>, mut s: TcpStream) {
    let mut buf = Vec::with_capacity(2048);
    let end = loop {
        let mut chunk = [0u8; 2048];
        let n = match tokio::time::timeout(std::time::Duration::from_secs(20), s.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => return,
        };
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|x| x == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > HEAD_MAX {
            return reply(&mut s, "431 Request Header Fields Too Large", "text/plain", "", b"").await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let mut first = head.lines().next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or("/"));
    if path == "/wire" || path.starts_with("/wire?") {
        let Ok(mut up) = TcpStream::connect(&w.wire).await else {
            return reply(&mut s, "502 Bad Gateway", "text/plain", "", b"wire.ws-door is down").await;
        };
        if up.write_all(&buf).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut s, &mut up).await;
        return;
    }
    if method != "GET" && method != "HEAD" {
        return reply(&mut s, "405 Method Not Allowed", "text/plain", "", b"").await;
    }
    let (page, door) = door_for(&w, &head);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let person = read_key(&w.person_key).and_then(|k| signed_person(&head, &k, now)).unwrap_or_default();
    let route = path.split(['?', '#']).next().unwrap_or("/");
    if route == "/session" || route == "/manifest.json" || route.starts_with("/_c/") {
        if person.is_empty() {
            return reply(&mut s, "401 Unauthorized", "text/plain", "", b"sign in first").await;
        }
        let v = (w.viewer)(&person);
        let viewer = match v {
            Ok(v) => v,
            Err(why) if why.starts_with("refused") => return reply(&mut s, "403 Forbidden", "text/plain", "", b"Refused").await,
            Err(why) => return reply(&mut s, "503 Service Unavailable", "text/plain", "", why.as_bytes()).await,
        };
        if route == "/manifest.json" {
            let b = bundles(&w.dir);
            let (m, left) = ui_manifest::resolve(&w.site, &b, &|r| covers(&viewer.abilities, r));
            eprintln!("site-gateway: manifest for {person} ({}): {} modules, {} left out", viewer.role, m.modules.len(), left.len());
            let body = serde_json::to_vec(&m).unwrap_or_default();
            return reply(&mut s, "200 OK", "application/json", "", &body).await;
        }
        if let Some(file) = route.strip_prefix('/') .filter(|f| f.starts_with("_c/")) {
            let b = bundles(&w.dir);
            if !ui_manifest::known(&b, file) {
                return reply(&mut s, "404 Not Found", "text/plain", "", b"no such file").await;
            }
            if !ui_manifest::may_fetch(&w.site, &b, &|r| covers(&viewer.abilities, r), file) {
                eprintln!("site-gateway: refused {file} to {person} ({})", viewer.role);
                return reply(&mut s, "403 Forbidden", "text/plain", "", b"Refused").await;
            }
            let Ok(body) = tokio::fs::read(w.dir.join(file)).await else { return reply(&mut s, "404 Not Found", "text/plain", "", b"no such file").await };
            let h = format!("HTTP/1.1 200 OK\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: private, max-age=31536000, immutable\r\nX-Content-Type-Options: nosniff\r\nVary: X-Ce-Person\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(h.as_bytes()).await;
            if method != "HEAD" {
                let _ = s.write_all(&body).await;
            }
            return;
        }
        let (m, abilities, ttl) = (w.mint.clone(), viewer.abilities.clone(), w.ttl_s);
        let (pg, dr) = (page.clone(), door.clone());
        let r = tokio::task::spawn_blocking(move || -> Result<String, String> {
            eprintln!("site-gateway: grant for {person} ({} abilities)", abilities.len());
            let (link, _) = m.mint(&pg, &abilities, ttl)?;
            fragment(&link, &dr)
        })
        .await;
        return match r {
            Ok(Ok(f)) => {
                let loc = format!("Location: ./#{f}\r\nReferrer-Policy: no-referrer\r\n");
                reply(&mut s, "302 Found", "text/plain", &loc, b"").await
            }
            Ok(Err(e)) => reply(&mut s, "503 Service Unavailable", "text/plain", "", e.as_bytes()).await,
            Err(_) => reply(&mut s, "500 Internal Server Error", "text/plain", "", b"").await,
        };
    }
    if route == "/bundles.json" {
        return reply(&mut s, "404 Not Found", "text/plain", "", b"no such file").await;
    }
    let Some(f) = file_of(&w.dir, path) else { return reply(&mut s, "404 Not Found", "text/plain", "", b"no such file").await };
    let Ok(body) = tokio::fs::read(&f).await else { return reply(&mut s, "404 Not Found", "text/plain", "", b"no such file").await };
    let ty = kind(&f);
    let n = f.file_name().and_then(|x| x.to_str()).unwrap_or("");
    // hashed modules never change; the page and the manifest do
    let cache = if n == "index.html" || n.ends_with(".json") { "no-cache" } else { "private, max-age=31536000, immutable" };
    let sec = if ty.starts_with("text/html") { format!("Content-Security-Policy: {}\r\nReferrer-Policy: no-referrer\r\n", csp(&door)) } else { String::new() };
    let h = format!("HTTP/1.1 200 OK\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nX-Content-Type-Options: nosniff\r\n{sec}Connection: close\r\n\r\n", body.len());
    let _ = s.write_all(h.as_bytes()).await;
    if method != "HEAD" {
        let _ = s.write_all(&body).await;
    }
}

pub async fn serve(addr: &str, w: Web) -> Result<(), String> {
    let sa: std::net::SocketAddr = addr.parse().map_err(|e| format!("{addr}: {e}"))?;
    if !sa.ip().is_loopback() {
        return Err(format!("{addr}: the HTTP side binds loopback only (the sign-in gate is in front)"));
    }
    let sock = tokio::net::TcpSocket::new_v4().map_err(|e| format!("{addr}: {e}"))?;
    sock.set_reuseaddr(true).map_err(|e| format!("{addr}: {e}"))?;
    sock.set_reuseport(true).map_err(|e| format!("{addr}: {e}"))?;
    sock.bind(sa).map_err(|e| format!("{addr}: {e}"))?;
    let l: TcpListener = sock.listen(1024).map_err(|e| format!("{addr}: {e}"))?;
    let w = Arc::new(w);
    loop {
        let (s, _) = l.accept().await.map_err(|e| e.to_string())?;
        tokio::spawn(one(w.clone(), s));
    }
}

/// The fragment of a minted link with the door the page must use.
pub fn fragment(link: &str, door: &str) -> Result<String, String> {
    let g = link.split_once('#').map(|(_, f)| f).filter(|f| f.starts_with("g=")).ok_or("wire.ws-door: mint gave no grant")?;
    Ok(format!("{g}&w={door}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forged_or_stale_person_is_nobody() {
        use hmac::{Hmac, Mac};
        let key = [7u8; 32];
        let sign = |p: &str, at: u64| {
            let mut m = Hmac::<sha2::Sha256>::new_from_slice(&key).unwrap();
            m.update(format!("ce-person\n{p}\n{at}").as_bytes());
            m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()
        };
        let head = |p: &str, at: u64, sig: &str| format!("GET /session HTTP/1.1\r\nX-Ce-Person: {p}\r\nX-Ce-Person-At: {at}\r\nX-Ce-Person-Sig: {sig}\r\n\r\n");
        assert_eq!(signed_person(&head("leif", 1000, &sign("leif", 1000)), &key, 1010).as_deref(), Some("leif"));
        assert_eq!(signed_person(&head("leif", 1000, &sign("ops", 1000)), &key, 1010), None);
        assert_eq!(signed_person(&head("leif", 1000, &sign("leif", 1000)), &key, 2000), None);
        assert_eq!(signed_person("GET / HTTP/1.1\r\nX-Ce-Person: leif\r\n\r\n", &key, 0), None);
    }

    #[test]
    fn files_stay_inside_the_dir() {
        let d = Path::new("/srv");
        assert_eq!(file_of(d, "/"), Some(d.join("index.html")));
        assert_eq!(file_of(d, "/panels/keys.ab12.js"), Some(d.join("panels/keys.ab12.js")));
        assert_eq!(file_of(d, "/../etc/passwd"), None);
        assert_eq!(file_of(d, "/.git/config"), None);
    }

    #[test]
    fn covers_is_ce_caps_rule() {
        let a = vec!["cr.keys:read".to_string(), "cr.audit:*".to_string(), "cr.devices:where:node=x".to_string()];
        assert!(covers(&a, "cr.keys:read") && covers(&a, "cr.audit:verify"));
        assert!(!covers(&a, "cr.keys:holders") && !covers(&a, "cr.devices:read"));
    }

    #[test]
    fn csp_allows_only_self_and_the_door() {
        let c = csp("wss://apps.ce-net.com/control-room/wire");
        assert!(c.contains("script-src 'self'"));
        assert!(c.contains("connect-src 'self' wss://apps.ce-net.com wss://apps.ce-net.com/control-room/wire"));
        assert!(!c.contains("unsafe"));
    }
}
