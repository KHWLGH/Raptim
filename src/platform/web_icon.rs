//! A web address has no file on disk to take an icon from, so the page itself is asked for one.
//! Everything here blocks, which is fine because every icon call site already runs off the event
//! loop. Any failure — offline, dead host, undecodable image — means "no icon", the same silent
//! outcome a missing executable has.

use core::ffi::c_void;
use url::Url;
use windows::{Win32::Networking::WinHttp::*, core::PCWSTR};

/// HTML is only read to find the icon links inside it.
const PAGE_LIMIT: usize = 256 * 1024;
/// Generous for a 180px PNG, still small enough to stop at a mis-typed image address.
const IMAGE_LIMIT: usize = 1024 * 1024;
/// `Entry::validate` caps an embedded icon at 256KB.
const STORED_LIMIT: usize = 200 * 1024;
/// A site with several declared icons still gets one chance each, not one per `<link>` tag.
const SIDE_ATTEMPTS: usize = 4;
/// Under this the icon is unreadable in the grid; over this it wastes database space.
const SHORTEST_EDGE: u32 = 16;
const LONGEST_EDGE: u32 = 128;
/// The edge a touch icon carries by convention when it declares no `sizes` of its own.
const TOUCH_ICON_EDGE: u32 = 180;
/// How long one address may keep a background slot before the lookup is abandoned.
const ATTEMPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
/// A server that refuses unknown clients just gets the same answer as one that is not there.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Diascia/0.0 favicon";

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub fn fetch(url: &str) -> Vec<u8> {
    let Ok(page) = Url::parse(url) else {
        return Vec::new();
    };
    if !matches!(page.scheme(), "http" | "https") {
        return Vec::new();
    }
    let started = std::time::Instant::now();
    let mut candidates = match get(&page, PAGE_LIMIT) {
        Some((kind, body)) if kind.to_ascii_lowercase().contains("html") => {
            icon_links(&String::from_utf8_lossy(&body), &page)
        }
        _ => Vec::new(),
    };
    candidates.truncate(SIDE_ATTEMPTS);
    if let Ok(fallback) = page.join("/favicon.ico") {
        candidates.push(fallback);
    }
    for candidate in candidates {
        // A host that stalls costs its full timeout per request; give up on the whole entry rather
        // than holding a background slot for half a minute.
        if started.elapsed() > ATTEMPT_BUDGET {
            break;
        }
        if let Some((_, body)) = get(&candidate, IMAGE_LIMIT)
            && let Some(png) = to_png(&body)
        {
            return png;
        }
    }
    Vec::new()
}

/// The `<link rel="…icon…">` addresses of a document, biggest expected edge first.
fn icon_links(html: &str, page: &Url) -> Vec<Url> {
    let mut found = Vec::new();
    for (names, values) in link_tags(html) {
        let rel = attribute(&names, "rel").unwrap_or_default();
        if !rel.contains("icon") {
            continue;
        }
        // `href` is read from the original casing: attribute names and `rel` are safe to
        // lowercase, a resource path is not.
        let Some(href) = attribute(&values, "href") else {
            continue;
        };
        if let Ok(resolved) = page.join(href.trim())
            && matches!(resolved.scheme(), "http" | "https")
        {
            let declared = declared_edge(&names);
            let touch = rel.contains("apple-touch") && declared == 0;
            found.push((if touch { TOUCH_ICON_EDGE } else { declared }, resolved));
        }
    }
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().map(|(_, url)| url).collect()
}

/// Every `<link …>` body in the document as a lowercased copy and the original. ASCII
/// lowercasing keeps byte lengths, so the two always line up.
fn link_tags(html: &str) -> Vec<(String, String)> {
    let lowered = html.to_ascii_lowercase();
    let mut tags = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = lowered[cursor..].find("<link") {
        let start = cursor + offset + "<link".len();
        cursor = start;
        let Some(width) = lowered[start..].find('>') else {
            continue;
        };
        tags.push((
            lowered[start..start + width].to_string(),
            html[start..start + width].to_string(),
        ));
    }
    tags
}

/// Value of `name="…"`, `name='…'` or `name=…` inside one tag body.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut cursor = 0usize;
    while let Some(offset) = tag[cursor..].find(name) {
        let start = cursor + offset;
        cursor = start + name.len();
        let boundary = start == 0
            || matches!(
                tag.as_bytes()[start - 1],
                b' ' | b'\t' | b'\n' | b'\r' | b'/'
            );
        let rest = tag[cursor..].trim_start_matches([' ', '\t', '\n', '\r']);
        if !boundary || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start_matches([' ', '\t', '\n', '\r']);
        let value = if let Some(value) = value.strip_prefix('"') {
            value.split('"').next()?
        } else if let Some(value) = value.strip_prefix('\'') {
            value.split('\'').next()?
        } else {
            value.split([' ', '\t', '\n', '\r', '>']).next()?
        };
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

/// The longest edge a `sizes` list promises, so a 180px touch icon beats a 16px one. `any` and
/// unparsable lists promise nothing and are tried last.
fn declared_edge(tag: &str) -> u32 {
    let Some(sizes) = attribute(tag, "sizes") else {
        return 0;
    };
    sizes
        .split_whitespace()
        .filter_map(|token| {
            let (width, height) = token.split_once('x')?;
            Some(width.parse::<u32>().ok()?.max(height.parse::<u32>().ok()?))
        })
        .max()
        .unwrap_or(0)
}

/// Re-encode whatever the server called an icon into the PNG format every other icon source uses.
fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width.min(height) < SHORTEST_EDGE || width.max(height) > 4096 {
        return None;
    }
    let image = decoded.into_rgba8();
    let longest = width.max(height);
    let image = if longest > LONGEST_EDGE {
        let scale = LONGEST_EDGE as f64 / longest as f64;
        image::imageops::resize(
            &image,
            ((width as f64 * scale) as u32).max(1),
            ((height as f64 * scale) as u32).max(1),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    let png = crate::icons::encode(image).ok()?;
    (png.len() <= STORED_LIMIT).then_some(png)
}

/// One plain GET with hard timeouts. Loopback never goes through a proxy, which is what a browser
/// does as well and what keeps the local test server reachable.
fn get(url: &Url, limit: usize) -> Option<(String, Vec<u8>)> {
    let host = url.host_str()?;
    let secure = url.scheme() == "https";
    let port = url.port_or_known_default()?;
    let mut object = url.path().to_string();
    if let Some(query) = url.query() {
        object.push('?');
        object.push_str(query);
    }
    let agent = super::wide(USER_AGENT);
    let session = unsafe {
        WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            if is_loopback(host) {
                WINHTTP_ACCESS_TYPE_NO_PROXY
            } else {
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY
            },
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    if session.is_null() {
        return None;
    }
    let _session = Handle(session);
    let server = super::wide(host);
    let verb = super::wide("GET");
    let path = super::wide(&object);
    let accepts = super::wide("Accept: image/*, */*;q=0.5\r\n");
    unsafe {
        let _ = WinHttpSetTimeouts(session, 2000, 3000, 3000, 3000);
        let connect = WinHttpConnect(session, PCWSTR(server.as_ptr()), port, 0);
        if connect.is_null() {
            return None;
        }
        let _connect = Handle(connect);
        let request = WinHttpOpenRequest(
            connect,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            if secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            },
        );
        if request.is_null() {
            return None;
        }
        let _request = Handle(request);
        let _ = WinHttpAddRequestHeaders(request, &accepts, WINHTTP_ADDREQ_FLAG_ADD);
        WinHttpSendRequest(request, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request, std::ptr::null_mut()).ok()?;
        let kind = header(request, WINHTTP_QUERY_CONTENT_TYPE);
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request, &mut available).ok()?;
            if available == 0 {
                break;
            }
            let mut buffer = vec![0u8; (available as usize).min(64 * 1024)];
            let mut read = 0u32;
            WinHttpReadData(
                request,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
                &mut read,
            )
            .ok()?;
            body.extend_from_slice(&buffer[..read as usize]);
            if body.len() > limit {
                return None;
            }
        }
        Some((kind, body))
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host == "[::1]"
        || host
            .strip_prefix("127.")
            .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}

fn header(request: *mut c_void, which: u32) -> String {
    unsafe {
        let mut length = 0u32;
        let _ = WinHttpQueryHeaders(
            request,
            which,
            PCWSTR::null(),
            None,
            &mut length,
            std::ptr::null_mut(),
        );
        if length == 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize / 2 + 1];
        let mut size = (buffer.len() * 2) as u32;
        if WinHttpQueryHeaders(
            request,
            which,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .is_err()
        {
            return String::new();
        }
        let units = (size as usize / 2).min(buffer.len());
        String::from_utf16_lossy(&buffer[..units])
            .trim_matches('\0')
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        sync::{Arc, Mutex},
    };

    /// A one-shot HTTP server on the loopback interface: the fetcher only makes sense end to end,
    /// and a real round trip is the only way to drive WinHTTP without inventing a fake stack.
    type Routes = Vec<(String, String, Vec<u8>)>;

    struct LocalServer {
        address: std::net::SocketAddr,
        routes: Arc<Mutex<Routes>>,
    }

    impl LocalServer {
        fn start() -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let routes = Arc::new(Mutex::new(Routes::new()));
            let served = routes.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let Some(path) = read_request(&mut stream) else {
                        continue;
                    };
                    let answer = served
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(route, _, _)| *route == path)
                        .cloned();
                    let reply = match answer {
                        Some((_, kind, body)) => {
                            let mut reply = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                            .into_bytes();
                            reply.extend_from_slice(&body);
                            reply
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = stream.write_all(&reply);
                    let _ = stream.flush();
                }
            });
            Self { address, routes }
        }

        fn serve(&self, routes: &[(&str, &str, Vec<u8>)]) {
            *self.routes.lock().unwrap() = routes
                .iter()
                .map(|(path, kind, body)| ((*path).to_string(), (*kind).to_string(), body.clone()))
                .collect();
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.address)
        }
    }

    /// Read exactly one request header block: waiting for the client to half-close would hang,
    /// because WinHTTP keeps the connection open until it has the answer.
    fn read_request(stream: &mut std::net::TcpStream) -> Option<String> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 256];
        loop {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") || bytes.len() > 8 * 1024 {
                break;
            }
        }
        String::from_utf8_lossy(&bytes)
            .split_whitespace()
            .nth(1)
            .map(str::to_string)
    }

    fn png(edge: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(edge, edge, image::Rgba([0, 120, 212, 255]));
        crate::icons::encode(image).unwrap()
    }

    fn edge(png: &[u8]) -> u32 {
        image::load_from_memory(png).unwrap().width()
    }

    fn fetched(url: &str) -> Vec<u8> {
        let png = fetch(url);
        assert!(!png.is_empty(), "{url} should have produced an icon");
        png
    }

    fn links(html: &str) -> Vec<String> {
        icon_links(
            html,
            &Url::parse("https://example.com/docs/index.html").unwrap(),
        )
        .into_iter()
        .map(|url| url.to_string())
        .collect()
    }

    #[test]
    fn icon_links_are_ordered_by_expected_size_and_survive_every_quoting_style() {
        assert_eq!(
            links(
                r#"<html><head>
                    <link rel="stylesheet" href="/site.css">
                    <link rel='shortcut icon' href='/small.ico' sizes="16x16">
                    <link rel=icon href=big.png sizes=180x180>
                    <link rel="apple-touch-icon" href="/touch.png">
                    <link rel="icon" href="/mid.png" sizes="32x32">
                    <link rel="icon" href="">
                    <link rel="icon" href="data:image/png;base64,AAAA">
                </head><body>rel="icon" outside a tag</body>"#
            ),
            vec![
                "https://example.com/docs/big.png",
                "https://example.com/touch.png",
                "https://example.com/mid.png",
                "https://example.com/small.ico",
            ],
            "the biggest expected icon is tried first; empty and data: hrefs are dropped"
        );
        assert!(links(r#"<link rel="stylesheet" href="/a.css">"#).is_empty());
    }

    #[test]
    fn declared_sizes_only_ever_reorder_the_candidates() {
        assert_eq!(declared_edge(r#" sizes="32x32 180x180 any" "#), 180);
        assert_eq!(declared_edge(r#"sizes="any""#), 0);
        assert_eq!(declared_edge(""), 0);
    }

    #[test]
    fn a_decoded_icon_is_normalised_to_a_storable_png() {
        assert_eq!(edge(&to_png(&png(16)).expect("16px is usable")), 16);
        assert_eq!(edge(&to_png(&png(600)).expect("oversized is scaled")), 128);
        assert!(to_png(b"not an image at all").is_none());
        assert!(to_png(&png(8)).is_none(), "too small to read in the grid");
    }

    #[test]
    fn an_unreachable_address_gives_no_icon_and_never_panics() {
        for dead in [
            "",
            "javascript:alert(1)",
            "file:///c:/test",
            "https://example.invalid/",
        ] {
            assert!(fetch(dead).is_empty(), "{dead} has no icon to give");
        }
    }

    #[test]
    fn a_page_is_scanned_for_its_icon_and_the_site_default_is_the_fallback() {
        let server = LocalServer::start();
        server.serve(&[
            (
                "/docs/",
                "text/html; charset=utf-8",
                br#"<html><head><link rel="icon" href="/static/site.png" sizes="64x64"></head>"#
                    .to_vec(),
            ),
            ("/static/site.png", "image/png", png(64)),
            ("/favicon.ico", "image/x-icon", png(32)),
        ]);
        // The dispatch every icon call site goes through, not only this module's entry point.
        let address = crate::model::Target::Url {
            url: server.url("/docs/"),
        };
        assert_eq!(
            edge(&super::super::icon_for(&address, std::path::Path::new("."))),
            64,
            "icon_for sends an address to the favicon lookup"
        );
        assert_eq!(
            edge(&fetched(&server.url("/docs/"))),
            64,
            "the declared icon wins"
        );

        server.serve(&[
            ("/docs/", "text/html", b"<html><head></head>".to_vec()),
            ("/favicon.ico", "image/x-icon", png(32)),
        ]);
        assert_eq!(
            edge(&fetched(&server.url("/docs/"))),
            32,
            "without a declared icon, /favicon.ico is tried"
        );

        server.serve(&[("/docs/", "text/plain", b"not a page".to_vec())]);
        assert!(
            fetch(&server.url("/docs/")).is_empty(),
            "a non-HTML answer is not a page"
        );
    }
}
