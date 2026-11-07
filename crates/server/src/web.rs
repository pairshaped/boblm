//! boblm-provided web tools for every agent (ADR 0041). `web_search` asks OpenAI's
//! hosted `web_search` tool; `web_fetch` reads a page over HTTP without a third party.
//! Both return untrusted data: page and search text is data, never instructions.
use boblm_tuning as tuning;
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

/// Load the OpenAI API key through the one-key SOPS boundary (ADR 0026). The value is
/// used only for one request and is never stored, logged or cached.
pub async fn openai_key() -> Result<String, String> {
    openai_key_for("web_search").await
}

/// Load the OpenAI API key for a boblm-owned Codex home. The value goes only into
/// the child environment named by the provider config.
pub async fn openai_api_key() -> Result<String, String> {
    openai_key_for("the Codex API account").await
}

async fn openai_key_for(what: &str) -> Result<String, String> {
    let key: Vec<String> = tuning::OPENAI_SOPS_KEY
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    crate::secrets::read_secret_labeled(
        tuning::OPENAI_SOPS_FILE,
        &key,
        Path::new("/"),
        &format!("the OpenAI API key for {what}"),
        "OPENAI_SOPS_KEY in crates/tuning",
    )
    .await
}

/// Tags whose contents are never readable text.
const SKIP_TAGS: &[&str] = &["head", "noscript", "script", "style", "svg", "template"];
/// Tags that start or end a block of readable text.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Read a response body, stopping before `max` bytes so a huge or endless reply cannot
/// exhaust memory. The error names the request, never its content.
async fn read_bounded(
    mut response: reqwest::Response,
    max: usize,
    what: &str,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("{what} could not be read: {e}."))?
    {
        if body.len() + chunk.len() > max {
            return Err(format!(
                "{what} is larger than the {max} byte limit, so reading stopped. Ask for a smaller page or raise the limit in crates/tuning."
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The `message` from an OpenAI error reply, bounded, with a next step. The API key is
/// sent as a header, so the body never carries it.
fn provider_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    let detail = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            String::from_utf8_lossy(body)
                .trim()
                .chars()
                .take(300)
                .collect()
        });
    let detail = detail.chars().take(300).collect::<String>();
    let detail = if detail.is_empty() {
        "it gave no reason".to_owned()
    } else {
        detail
    };
    format!(
        "OpenAI refused the web search (HTTP {status}): {detail}. Check the OpenAI API key and its credit, then try again."
    )
}

/// Ask OpenAI's hosted `web_search` tool and return its answer plus cited sources.
/// The endpoint and model come from tuning so tests can point them at a fake server.
pub async fn search(
    query: &str,
    key: &str,
    endpoint: &str,
    timeout: std::time::Duration,
) -> Result<Value, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("web_search needs a non-empty query; give the words to search for.".into());
    }
    let length = query.chars().count();
    if length > tuning::WEB_SEARCH_MAX_QUERY_CHARS {
        return Err(format!(
            "The web_search query is {length} characters, over the {} character limit. Shorten it and try again.",
            tuning::WEB_SEARCH_MAX_QUERY_CHARS
        ));
    }
    let request = json!({
        "model": tuning::OPENAI_WEB_SEARCH_MODEL,
        "reasoning": { "effort": tuning::OPENAI_WEB_SEARCH_REASONING_EFFORT },
        "store": false,
        "tool_choice": "required",
        "tools": [{ "type": "web_search" }],
        "input": query,
    });
    let body = serde_json::to_vec(&request).map_err(|e| {
        format!("The web search request could not be encoded ({e}). This is a bug in BobLM.")
    })?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| {
            format!(
                "Could not start the web search HTTP client: {e}. Check the server's TLS setup."
            )
        })?;
    let response = client
        .post(endpoint)
        .bearer_auth(key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "The web search did not answer within {} seconds and was stopped. Try again, or search for less.",
                    timeout.as_secs()
                )
            } else {
                format!("Could not reach OpenAI for the web search: {e}. Check the server's network connection, then try again.")
            }
        })?;
    let status = response.status();
    let bytes = read_bounded(
        response,
        tuning::WEB_SEARCH_MAX_RESPONSE_BYTES,
        "The web search reply",
    )
    .await?;
    if !status.is_success() {
        return Err(provider_error(status, &bytes));
    }
    let reply: Value = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "OpenAI's web search reply was not the expected JSON ({e}). Try again; if it keeps failing, check OPENAI_WEB_SEARCH_MODEL in crates/tuning."
        )
    })?;
    Ok(parse_search_reply(query, &reply))
}

/// Pull the answer text and the cited sources out of a Responses API reply, tolerating
/// unknown items and fields. Output text is bounded.
fn parse_search_reply(query: &str, reply: &Value) -> Value {
    let mut answer = String::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut searches = 0u64;
    for item in reply
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("web_search_call") => searches += 1,
            Some("message") => {
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if content.get("type").and_then(Value::as_str) == Some("output_text")
                        && let Some(text) = content.get("text").and_then(Value::as_str)
                    {
                        if !answer.is_empty() {
                            answer.push('\n');
                        }
                        answer.push_str(text);
                    }
                    for annotation in content
                        .get("annotations")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if annotation.get("type").and_then(Value::as_str) != Some("url_citation") {
                            continue;
                        }
                        let url = annotation.get("url").and_then(Value::as_str).unwrap_or("");
                        if url.is_empty() || sources.iter().any(|s| s["url"].as_str() == Some(url))
                        {
                            continue;
                        }
                        if sources.len() >= tuning::WEB_SEARCH_MAX_SOURCES {
                            continue;
                        }
                        sources.push(json!({
                            "url": url,
                            "title": annotation.get("title").and_then(Value::as_str).unwrap_or(""),
                        }));
                    }
                }
            }
            _ => {}
        }
    }
    let answer: String = answer
        .chars()
        .take(tuning::WEB_SEARCH_MAX_ANSWER_CHARS)
        .collect();
    json!({
        "query": query,
        "answer": answer,
        "sources": sources,
        "searchCalls": searches,
    })
}

/// Which addresses web_fetch may connect to. Production uses PUBLIC_ONLY; tests inject
/// loopback access and a fixed resolution so fake servers are reachable without DNS.
#[derive(Clone, Copy, Debug)]
pub struct AddressPolicy {
    allow_loopback: bool,
    resolution_override: Option<(&'static str, IpAddr)>,
}

impl AddressPolicy {
    /// Production rule: only globally routable addresses.
    pub const PUBLIC_ONLY: Self = Self {
        allow_loopback: false,
        resolution_override: None,
    };

    /// Test seam: permit loopback so a fake server can be reached.
    #[doc(hidden)]
    pub const fn allow_loopback_for_tests() -> Self {
        Self {
            allow_loopback: true,
            resolution_override: None,
        }
    }

    /// Test seam: resolve one host name to a fixed address without DNS.
    #[doc(hidden)]
    pub const fn resolve_for_tests(mut self, host: &'static str, ip: IpAddr) -> Self {
        self.resolution_override = Some((host, ip));
        self
    }
}

/// Why an address is not publicly routable, if it is not.
fn blocked_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => blocked_v4_reason(ip),
        IpAddr::V6(ip) => {
            if ip.is_loopback() {
                return Some("loopback");
            }
            if ip.is_unspecified() {
                return Some("unspecified");
            }
            if ip.is_multicast() {
                return Some("multicast");
            }
            if ip.is_unicast_link_local() {
                return Some("link-local");
            }
            if ip.is_unique_local() {
                return Some("unique-local");
            }
            ip.to_ipv4().and_then(blocked_v4_reason)
        }
    }
}

fn blocked_v4_reason(ip: Ipv4Addr) -> Option<&'static str> {
    let octets = ip.octets();
    if ip.is_loopback() {
        Some("loopback")
    } else if ip.is_unspecified() {
        Some("unspecified")
    } else if ip.is_private() {
        Some("private")
    } else if ip.is_link_local() {
        Some("link-local")
    } else if ip.is_broadcast() {
        Some("broadcast")
    } else if ip.is_multicast() {
        Some("multicast")
    } else if octets[0] == 100 && (octets[1] & 0xc0) == 64 {
        Some("shared (CGNAT)")
    } else {
        None
    }
}

fn blocked_address_error(ip: IpAddr, reason: &str) -> String {
    format!(
        "web_fetch does not fetch local or private network addresses; {ip} is {reason}. Fetch a public page instead."
    )
}

fn blocked_resolution_error(host: &str, ip: IpAddr, reason: &str) -> String {
    format!(
        "web_fetch does not fetch local or private network addresses; {host} resolves to {ip}, which is {reason}. Fetch a public page instead."
    )
}

/// Resolve one hop to a checked address. The caller connects to this address with a
/// reqwest override, so a second lookup cannot answer with a different address.
async fn checked_address(
    url: &reqwest::Url,
    policy: AddressPolicy,
) -> Result<(String, SocketAddr, bool), String> {
    let host = url
        .host_str()
        .ok_or_else(|| {
            format!(
                "web_fetch needs an address with a host name; {} has none. Give a full http:// or https:// address.",
                boblm_protocol::client::shown(url.as_str())
            )
        })?
        .to_owned();
    let port = url.port_or_known_default().ok_or_else(|| {
        format!(
            "web_fetch could not tell which port to use for {}; give an http:// or https:// address with a port.",
            boblm_protocol::client::shown(url.as_str())
        )
    })?;
    let literal = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&host)
        .parse::<IpAddr>()
        .ok();
    let addresses: Vec<IpAddr> = match literal {
        Some(ip) => vec![ip],
        None => {
            if let Some((name, ip)) = policy.resolution_override
                && name.eq_ignore_ascii_case(&host)
            {
                vec![ip]
            } else {
                let resolved = tokio::net::lookup_host((host.as_str(), port))
                    .await
                    .map_err(|e| {
                        format!(
                            "web_fetch could not resolve {host}: {e}. Check the address, or try again if the DNS service was busy."
                        )
                    })?;
                let addresses: Vec<IpAddr> = resolved.map(|address| address.ip()).collect();
                if addresses.is_empty() {
                    return Err(format!(
                        "web_fetch found no network address for {host}. Check the address, then try again."
                    ));
                }
                addresses
            }
        }
    };
    let mut chosen = None;
    for ip in addresses {
        if let Some(reason) = blocked_reason(ip) {
            let allowed = policy.allow_loopback && reason == "loopback";
            if !allowed {
                return Err(if literal.is_some() {
                    blocked_address_error(ip, reason)
                } else {
                    blocked_resolution_error(&host, ip, reason)
                });
            }
        }
        if chosen.is_none() {
            chosen = Some(ip);
        }
    }
    let ip = chosen.ok_or_else(|| {
        format!("web_fetch found no network address for {host}. Check the address, then try again.")
    })?;
    Ok((host, SocketAddr::new(ip, port), literal.is_some()))
}

/// Fetch a page over HTTP and return its readable text. Only http and https are allowed;
/// HTML is converted to plain text with Markdown links, other text is returned as is.
pub async fn fetch(url: &str, timeout: std::time::Duration) -> Result<Value, String> {
    fetch_with(url, timeout, AddressPolicy::PUBLIC_ONLY).await
}

/// Fetch with an explicit address policy. Production callers use `fetch`; the policy
/// parameter is the test seam for loopback fake servers and fixed resolutions.
pub async fn fetch_with(
    url: &str,
    timeout: std::time::Duration,
    policy: AddressPolicy,
) -> Result<Value, String> {
    let typed = url.trim();
    if typed.chars().count() > tuning::WEB_FETCH_MAX_URL_CHARS {
        return Err(format!(
            "The web_fetch URL is longer than the {} character limit. Use a shorter address.",
            tuning::WEB_FETCH_MAX_URL_CHARS
        ));
    }
    let mut current = reqwest::Url::parse(typed).map_err(|e| {
        format!(
            "web_fetch needs a full web address starting with http:// or https://; {} could not be parsed ({e}).",
            boblm_protocol::client::shown(typed)
        )
    })?;
    check_scheme(&current, typed)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut redirects = 0usize;
    loop {
        let (host, address, literal) = checked_address(&current, policy).await?;
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "The page at {typed} did not answer within {} seconds and was stopped. Try again, or fetch a smaller page.",
                timeout.as_secs()
            ));
        }
        let mut builder = reqwest::Client::builder()
            .timeout(remaining)
            .user_agent(tuning::WEB_FETCH_USER_AGENT)
            .redirect(reqwest::redirect::Policy::none());
        if !literal {
            builder = builder.resolve(&host, address);
        }
        let client = builder.build().map_err(|e| {
            format!("Could not start the web_fetch HTTP client: {e}. Check the server's TLS setup.")
        })?;
        let shown = current.as_str().to_owned();
        let response = client.get(current.clone()).send().await.map_err(|e| {
            if e.is_timeout() {
                format!(
                    "The page at {shown} did not answer within {} seconds and was stopped. Try again, or fetch a smaller page.",
                    timeout.as_secs()
                )
            } else {
                format!("Could not reach {shown}: {e}. Check the address and the server's network connection.")
            }
        })?;
        let status = response.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    format!(
                        "The page at {shown} answered HTTP {status} without a usable Location header, so web_fetch cannot follow the redirect. Fetch the final page directly."
                    )
                })?;
            let next = current.join(location).map_err(|e| {
                format!(
                    "The page at {shown} redirected to {}, which could not be parsed ({e}). Fetch the final page directly.",
                    boblm_protocol::client::shown(location)
                )
            })?;
            check_scheme(&next, location)?;
            if redirects >= tuning::WEB_FETCH_MAX_REDIRECTS {
                return Err(format!(
                    "The page at {typed} redirected more than {} times and was stopped. Fetch the final page directly.",
                    tuning::WEB_FETCH_MAX_REDIRECTS
                ));
            }
            redirects += 1;
            current = next;
            continue;
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !status.is_success() {
            return Err(format!(
                "The page at {shown} answered HTTP {status}, so there is no content to read. Check the address."
            ));
        }
        let bytes = read_bounded(response, tuning::WEB_FETCH_MAX_BYTES, "The page").await?;
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let media = content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        let is_html = media.is_empty() || media == "text/html" || media == "application/xhtml+xml";
        let is_text = media.starts_with("text/");
        if !is_html && !is_text {
            let media_shown = if media.is_empty() {
                "no content type"
            } else {
                media.as_str()
            };
            return Err(format!(
                "The page at {shown} is {media_shown}, which web_fetch cannot read as text. It only reads HTML and plain text pages."
            ));
        }
        let (title, content, truncated) = if is_html {
            let (title, text) = html_to_text(&raw);
            let truncated = text.chars().count() > tuning::WEB_FETCH_MAX_CHARS;
            let text: String = text.chars().take(tuning::WEB_FETCH_MAX_CHARS).collect();
            (title, text, truncated)
        } else {
            let truncated = raw.chars().count() > tuning::WEB_FETCH_MAX_CHARS;
            let text: String = raw.chars().take(tuning::WEB_FETCH_MAX_CHARS).collect();
            (None, text, truncated)
        };
        return Ok(json!({
            "url": typed,
            "finalUrl": current.as_str(),
            "contentType": if media.is_empty() { "text/html" } else { media.as_str() },
            "title": title,
            "truncated": truncated,
            "content": content,
        }));
    }
}

fn check_scheme(url: &reqwest::Url, shown: &str) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!(
            "web_fetch only fetches http and https addresses; {} uses the {other} scheme. Give an http:// or https:// address.",
            boblm_protocol::client::shown(shown)
        )),
    }
}

/// Convert HTML to readable plain text with Markdown links. Dependency-free and bounded
/// by the caller's character limit. Unknown tags are ignored rather than rejected.
fn html_to_text(html: &str) -> (Option<String>, String) {
    let title = extract_title(html);
    let mut out = String::new();
    let mut skip: Option<String> = None;
    let mut anchors: Vec<(usize, String)> = Vec::new();
    let mut index = 0;
    while index < html.len() {
        match html[index..].find('<') {
            None => {
                if skip.is_none() {
                    push_text(&mut out, &html[index..]);
                }
                break;
            }
            Some(offset) => {
                if skip.is_none() {
                    push_text(&mut out, &html[index..index + offset]);
                }
                index += offset;
                if html[index..].starts_with("<!--") {
                    match html[index..].find("-->") {
                        Some(end) => index += end + 3,
                        None => break,
                    }
                    continue;
                }
                match html[index..].find('>') {
                    Some(end) => {
                        handle_tag(
                            &html[index + 1..index + end],
                            &mut out,
                            &mut skip,
                            &mut anchors,
                        );
                        index += end + 1;
                    }
                    None => break,
                }
            }
        }
    }
    (title, normalize(&out))
}

/// Update the output for one tag. Text inside a skipped element is dropped, and an
/// anchor becomes `[text](url)` when its closing tag arrives.
fn handle_tag(
    tag: &str,
    out: &mut String,
    skip: &mut Option<String>,
    anchors: &mut Vec<(usize, String)>,
) {
    let tag = tag.trim();
    if tag.is_empty() {
        return;
    }
    let closing = tag.starts_with('/');
    let body = tag.trim_start_matches('/').trim();
    let name: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() {
        return;
    }
    if let Some(active) = skip.clone() {
        if closing && name == active {
            *skip = None;
        }
        return;
    }
    if closing {
        if SKIP_TAGS.contains(&name.as_str()) {
            return;
        }
        if name == "a" {
            if let Some((position, href)) = anchors.pop()
                && out.len() > position
                && !href.is_empty()
            {
                out.insert(position, '[');
                out.push_str("](");
                out.push_str(&href);
                out.push(')');
            }
            return;
        }
        if BLOCK_TAGS.contains(&name.as_str()) {
            out.push('\n');
        }
        return;
    }
    if SKIP_TAGS.contains(&name.as_str()) {
        *skip = Some(name);
        return;
    }
    match name.as_str() {
        "br" => out.push('\n'),
        "hr" => out.push_str("\n---\n"),
        "li" => out.push_str("\n- "),
        "img" => {
            if let Some(alt) = attr(body, "alt") {
                out.push_str("[image: ");
                out.push_str(&alt);
                out.push(']');
            }
        }
        "a" => {
            let href = attr(body, "href").unwrap_or_default();
            anchors.push((out.len(), href));
        }
        _ => {
            if BLOCK_TAGS.contains(&name.as_str()) {
                out.push('\n');
            }
        }
    }
}

/// The value of one attribute in a tag body, decoded. Quoted or bare.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(offset) = lower[search..].find(name) {
        let start = search + offset;
        let preceded = start == 0 || tag.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = &tag[start + name.len()..];
        let value = rest.trim_start();
        if preceded && let Some(value) = value.strip_prefix('=') {
            let value = value.trim_start();
            if let Some(rest) = value.strip_prefix('"')
                && let Some(end) = rest.find('"')
            {
                return Some(decode_entities(&rest[..end]));
            }
            if let Some(rest) = value.strip_prefix('\'')
                && let Some(end) = rest.find('\'')
            {
                return Some(decode_entities(&rest[..end]));
            }
            let end = value
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(value.len());
            return Some(decode_entities(&value[..end]));
        }
        search = start + name.len();
    }
    None
}

/// The page title, if it has one, bounded and decoded.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title")? + open_end;
    let title = normalize(&decode_entities(&html[open_end..close]));
    if title.is_empty() {
        None
    } else {
        Some(title.chars().take(300).collect())
    }
}

/// Append text with HTML entities decoded.
fn push_text(out: &mut String, text: &str) {
    out.push_str(&decode_entities(text));
}

/// Decode the common named entities and any numeric reference. An unknown or unterminated
/// reference is left as written rather than dropped.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let after = &rest[index..];
        let mut consumed = 1;
        if let Some(semi) = after.find(';').filter(|semi| *semi <= 12)
            && let Some(ch) = decode_entity(&after[1..semi])
        {
            out.push(ch);
            consumed = semi + 1;
        } else {
            out.push('&');
        }
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "apos" => Some('\''),
        "gt" => Some('>'),
        "hellip" => Some('\u{2026}'),
        "laquo" => Some('\u{ab}'),
        "ldquo" => Some('\u{201c}'),
        "lsquo" => Some('\u{2018}'),
        "lt" => Some('<'),
        "mdash" => Some('\u{2014}'),
        "middot" => Some('\u{b7}'),
        "nbsp" => Some(' '),
        "ndash" => Some('\u{2013}'),
        "quot" => Some('"'),
        "raquo" => Some('\u{bb}'),
        "rdquo" => Some('\u{201d}'),
        "rsquo" => Some('\u{2019}'),
        _ => {
            let number = entity.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Collapse whitespace into readable lines: one space between words, at most one blank
/// line between blocks, no leading or trailing whitespace.
fn normalize(raw: &str) -> String {
    let raw = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace(['\t', '\u{a0}'], " ");
    let mut out = String::new();
    let mut blanks = 0;
    for line in raw.split('\n') {
        let mut collapsed = String::new();
        let mut space = false;
        for ch in line.chars() {
            if ch == ' ' {
                space = true;
            } else {
                if space && !collapsed.is_empty() {
                    collapsed.push(' ');
                }
                space = false;
                collapsed.push(ch);
            }
        }
        let line = collapsed.trim_start();
        if line.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line.trim_end());
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_private_addresses_are_classified() {
        let reason = |text: &str| blocked_reason(text.parse::<IpAddr>().unwrap());
        assert_eq!(reason("127.0.0.1"), Some("loopback"));
        assert_eq!(reason("::1"), Some("loopback"));
        assert_eq!(reason("0.0.0.0"), Some("unspecified"));
        assert_eq!(reason("::"), Some("unspecified"));
        assert_eq!(reason("10.0.0.1"), Some("private"));
        assert_eq!(reason("172.16.0.1"), Some("private"));
        assert_eq!(reason("192.168.1.1"), Some("private"));
        assert_eq!(reason("169.254.1.1"), Some("link-local"));
        assert_eq!(reason("fe80::1"), Some("link-local"));
        assert_eq!(reason("100.64.0.1"), Some("shared (CGNAT)"));
        assert_eq!(reason("100.127.255.255"), Some("shared (CGNAT)"));
        assert_eq!(reason("fc00::1"), Some("unique-local"));
        assert_eq!(reason("fd12:3456::1"), Some("unique-local"));
        assert_eq!(reason("::ffff:127.0.0.1"), Some("loopback"));
        assert_eq!(reason("::ffff:10.0.0.1"), Some("private"));
        assert_eq!(reason("::ffff:100.64.0.1"), Some("shared (CGNAT)"));
        assert_eq!(reason("224.0.0.1"), Some("multicast"));
        assert_eq!(reason("::ffff:224.0.0.1"), Some("multicast"));
        assert_eq!(reason("255.255.255.255"), Some("broadcast"));
        assert_eq!(reason("8.8.8.8"), None);
        assert_eq!(reason("172.32.0.1"), None);
        assert_eq!(reason("2606:4700:4700::1111"), None);
    }

    #[test]
    fn html_becomes_readable_text_with_links_and_no_scripts() {
        let html = r#"<html><head><title>Hello &amp; Welcome</title><style>.x{}</style>
            <script>alert('no')</script></head><body><h1>Heading</h1>
            <p>First &lt;para&gt; with an entity &#39;here&#39;.</p>
            <ul><li>One</li><li>Two <a href="https://example.com/a?b=1&amp;c=2">link</a></li></ul>
            <p>Second<br>line</p></body></html>"#;
        let (title, text) = html_to_text(html);
        assert_eq!(title.as_deref(), Some("Hello & Welcome"));
        assert!(!text.contains("alert"));
        assert!(!text.contains(".x{}"));
        assert!(text.contains("Heading"));
        assert!(text.contains("First <para> with an entity 'here'."));
        assert!(text.contains("- One"));
        assert!(text.contains("[link](https://example.com/a?b=1&c=2)"));
        assert!(text.contains("Second\nline"));
    }

    #[test]
    fn entities_and_whitespace_are_bounded_and_normalized() {
        assert_eq!(
            decode_entities("a &amp; b &#x41; &#66; &unknown; &"),
            "a & b A B &unknown; &"
        );
        assert_eq!(normalize("  a   b \n\n\n\n c \t d  "), "a b\n\nc d");
        let (title, text) = html_to_text("<html><body><p>one</p><p>two</p></body></html>");
        assert_eq!(title, None);
        assert_eq!(text, "one\n\ntwo");
    }
}
