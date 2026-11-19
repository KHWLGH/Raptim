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
/// The largest edge a server is allowed to hand back before it is refused. Kept well above
/// `LONGEST_EDGE` because the answer is downscaled, not rejected, between the two.
const MAX_EDGE: u32 = 4096;
const LONGEST_EDGE: u32 = 128;
/// The edge a touch icon carries by convention when it declares no `sizes` of its own.
const TOUCH_ICON_EDGE: u32 = 180;
/// How long one address may keep a background slot before the lookup is abandoned.
const ATTEMPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
/// A server that refuses unknown clients just gets the same answer as one that is not there.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Raptim/0.0 favicon";

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
        Some(page_response) if page_response.ok() && page_response.is_html() => {
            icon_links(&String::from_utf8_lossy(&page_response.body), &page)
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
        if let Some(answer) = get(&candidate, IMAGE_LIMIT)
            && answer.ok()
            && let Some(png) = to_png(&answer.body)
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
        let rel = attribute(&names, &names, "rel").unwrap_or_default();
        if !rel.contains("icon") {
            continue;
        }
        // `href` is read out of the original casing: the name is matched case-insensitively and
        // `rel` is safe to lowercase, a resource path is not.
        let Some(href) = attribute(&names, &values, "href") else {
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

/// Value of `name="…"`, `name='…'` or `name=…` inside one tag body. The attribute *name* is looked up
/// in the case-folded copy — HTML spells names any way it likes — while the value is cut out of the
/// original, because a resource path is case-sensitive. ASCII folding keeps byte lengths, which is
/// what lets one set of offsets address both copies; that is also why `link_tags` keeps the pair.
fn attribute<'a>(lower: &str, raw: &'a str, name: &str) -> Option<&'a str> {
    let mut cursor = 0usize;
    while let Some(offset) = lower[cursor..].find(name) {
        let start = cursor + offset;
        cursor = start + name.len();
        let boundary = start == 0
            || matches!(
                lower.as_bytes()[start - 1],
                b' ' | b'\t' | b'\n' | b'\r' | b'/'
            );
        let equals = skip_space(lower, cursor);
        if !boundary || !lower[equals..].starts_with('=') {
            continue;
        }
        let value = &raw[skip_space(lower, equals + 1)..];
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
/// The byte offset of the first non-whitespace character at or after `at`.
fn skip_space(text: &str, at: usize) -> usize {
    let rest = &text[at..];
    at + rest.len() - rest.trim_start_matches([' ', '\t', '\n', '\r']).len()
}

/// The longest edge a `sizes` list promises, so a 180px touch icon beats a 16px one. `any` and
/// unparsable lists promise nothing and are tried last. Fed the case-folded copy: a sizes list is
/// dimensions, not a path, so folding it is free.
fn declared_edge(lower: &str) -> u32 {
    let Some(sizes) = attribute(lower, lower, "sizes") else {
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
/// The edge limits go to the decoder rather than to its output: a compressed image is a fraction of
/// its own size, so checking the dimensions after decoding means the pixels were already allocated
/// by the time the answer was "too big".
fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width.min(height) < SHORTEST_EDGE {
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

/// One answered request. The status line travels with the body because a body is not an icon just
/// because the server named an image type on it: a 404 stub, a 500 trace page and a captive portal
/// that answers 200 with HTML all pass a content-type test, and none of them is what was asked for.
struct Response {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}
impl Response {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    fn is_html(&self) -> bool {
        self.content_type.to_ascii_lowercase().contains("html")
    }
}

/// One plain GET with hard timeouts. Loopback never goes through a proxy, which is what a browser
/// does as well and what keeps the local test server reachable.
fn get(url: &Url, limit: usize) -> Option<Response> {
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
        // Read as text, not `WINHTTP_FLAG_NUMBER`: `header` decodes a UTF-16 buffer, which is what
        // the string form of the status code is. An unreadable status counts as "not 2xx" (0), so a
        // response this code cannot interpret never becomes an icon.
        let status = header(request, WINHTTP_QUERY_STATUS_CODE)
            .trim()
            .parse()
            .unwrap_or(0);
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
        Some(Response {
            status,
            content_type: kind,
            body,
        })
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

    fn png(edge: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(edge, edge, image::Rgba([0, 120, 212, 255]));
        crate::icons::encode(image).unwrap()
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
    fn parses_icon_links_and_normalizes_pngs() {
        assert_eq!(
            links(
                r#"<html><head>
                    <link rel="stylesheet" href="/site.css">
                    <link rel='shortcut icon' href='/small.ico' sizes="16x16">
                    <link rel=icon href=big.png sizes=180x180>
                    <LINK REL="ICON" HREF="/UPPER.PNG">
                </head></html>"#
            ),
            vec![
                "https://example.com/docs/big.png",
                "https://example.com/small.ico",
                "https://example.com/UPPER.PNG",
            ]
        );
        assert_eq!(
            image::load_from_memory(&to_png(&png(600)).unwrap())
                .unwrap()
                .width(),
            128
        );
        assert!(to_png(&png(8)).is_none());
        assert!(to_png(b"not an image").is_none());
    }
}
