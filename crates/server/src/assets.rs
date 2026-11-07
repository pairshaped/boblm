//! The browser build is a compilation input, with no runtime checkout dependency.
//!
//! Each file is served with a content hash (ETag), so a reload costs a `304 Not Modified` instead of
//! the whole file, and with gzip for browsers that accept it. The hash and the compressed copy are made
//! once, on first use. The files stay `no-cache`: the browser asks every time, and the server says
//! whether anything changed.
use std::{io::Write, sync::LazyLock};

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

struct Asset {
    content_type: &'static str,
    bytes: &'static [u8],
    etag: String,
    gzip: Vec<u8>,
    gzip_etag: String,
}

impl Asset {
    fn new(content_type: &'static str, bytes: &'static [u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let hash: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        // Writing to a Vec cannot fail; if it somehow did, the plain file is still served.
        let gzip = encoder
            .write_all(bytes)
            .ok()
            .and_then(|()| encoder.finish().ok())
            .filter(|gzip| gzip.len() < bytes.len())
            .unwrap_or_default();
        Self {
            content_type,
            bytes,
            etag: format!("\"{hash}\""),
            gzip,
            gzip_etag: format!("\"{hash}-gzip\""),
        }
    }
}

fn asset(path: &str) -> Option<&'static Asset> {
    static INDEX: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "text/html; charset=utf-8",
            include_bytes!("../../../web/dist/index.html"),
        )
    });
    static FAVICON: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "image/svg+xml",
            include_bytes!("../../../web/dist/favicon.svg"),
        )
    });
    static APP: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "text/javascript; charset=utf-8",
            include_bytes!("../../../web/dist/app.js"),
        )
    });
    static STYLE: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "text/css; charset=utf-8",
            include_bytes!("../../../web/dist/style.css"),
        )
    });
    static SW: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "text/javascript; charset=utf-8",
            include_bytes!("../../../web/dist/sw.js"),
        )
    });
    static MANIFEST: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "application/manifest+json",
            include_bytes!("../../../web/dist/manifest.webmanifest"),
        )
    });
    static ICON192: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "image/png",
            include_bytes!("../../../web/dist/icon-192.png"),
        )
    });
    static ICON512: LazyLock<Asset> = LazyLock::new(|| {
        Asset::new(
            "image/png",
            include_bytes!("../../../web/dist/icon-512.png"),
        )
    });
    match path {
        "/sw.js" => Some(&SW),
        "/manifest.webmanifest" => Some(&MANIFEST),
        "/icon-192.png" => Some(&ICON192),
        "/icon-512.png" => Some(&ICON512),
        "/" | "/index.html" => Some(&INDEX),
        "/favicon.svg" => Some(&FAVICON),
        "/app.js" => Some(&APP),
        "/style.css" => Some(&STYLE),
        _ => None,
    }
}

/// Whether the browser accepts gzip: named, and not refused with `q=0`.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|entry| {
            let mut parts = entry.split(';');
            let name = parts.next().unwrap_or("").trim();
            let refused = parts.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    == Some(0.0)
            });
            name.eq_ignore_ascii_case("gzip") && !refused
        })
}

/// Whether the browser already holds this version: its `If-None-Match` names the tag, or is `*`.
fn already_has(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}

pub async fn serve(headers: HeaderMap, uri: Uri) -> Response {
    let Some(asset) = asset(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let gzip = !asset.gzip.is_empty() && accepts_gzip(&headers);
    let (body, etag): (&'static [u8], &str) = if gzip {
        (asset.gzip.as_slice(), &asset.gzip_etag)
    } else {
        (asset.bytes, &asset.etag)
    };
    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response_headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Ok(tag) = HeaderValue::from_str(etag) {
        response_headers.insert(header::ETAG, tag);
    }
    if already_has(&headers, etag) {
        return (StatusCode::NOT_MODIFIED, response_headers).into_response();
    }
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    if gzip {
        response_headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    (response_headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use std::io::Read;
    use tower::ServiceExt;

    async fn fetch(path: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = Request::builder().uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = Router::new()
            .fallback(get(serve))
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        (parts.status, parts.headers, bytes.to_vec())
    }

    #[tokio::test]
    async fn a_file_comes_with_a_stable_tag_and_must_be_revalidated() {
        let (status, headers, body) = fetch("/app.js", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
        assert_eq!(headers[header::VARY], "Accept-Encoding");
        assert!(headers.get(header::CONTENT_ENCODING).is_none());
        assert_eq!(body, include_bytes!("../../../web/dist/app.js"));
        let (_, again, _) = fetch("/app.js", &[]).await;
        assert_eq!(headers[header::ETAG], again[header::ETAG]);
    }

    #[tokio::test]
    async fn a_browser_that_has_the_file_gets_not_modified_and_no_body() {
        let (_, headers, _) = fetch("/app.js", &[]).await;
        let tag = headers[header::ETAG].to_str().unwrap().to_owned();
        for held in [
            tag.clone(),
            format!("W/{tag}"),
            format!("\"other\", {tag}"),
            "*".into(),
        ] {
            let (status, reply, body) = fetch("/app.js", &[("if-none-match", &held)]).await;
            assert_eq!(status, StatusCode::NOT_MODIFIED, "{held}");
            assert!(body.is_empty());
            assert_eq!(reply[header::ETAG], tag.as_str());
            assert_eq!(reply[header::CACHE_CONTROL], "no-cache");
        }
        let (status, _, body) = fetch("/app.js", &[("if-none-match", "\"stale\"")]).await;
        assert_eq!(status, StatusCode::OK, "an old version is sent in full");
        assert!(!body.is_empty());
    }

    #[tokio::test]
    async fn gzip_is_sent_to_browsers_that_accept_it_and_unpacks_to_the_same_file() {
        let (status, headers, body) =
            fetch("/app.js", &[("accept-encoding", "gzip, deflate, br")]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
        let original: &[u8] = include_bytes!("../../../web/dist/app.js");
        assert!(
            body.len() < original.len() / 2,
            "{} of {}",
            body.len(),
            original.len()
        );
        let mut unpacked = Vec::new();
        flate2::read::GzDecoder::new(body.as_slice())
            .read_to_end(&mut unpacked)
            .unwrap();
        assert_eq!(unpacked, original);
        let (_, plain, _) = fetch("/app.js", &[]).await;
        assert_ne!(
            headers[header::ETAG],
            plain[header::ETAG],
            "each encoding has its own tag"
        );
    }

    #[tokio::test]
    async fn gzip_is_not_sent_when_refused_or_not_asked_for() {
        for ask in ["identity", "gzip;q=0", "br"] {
            let (_, headers, body) = fetch("/app.js", &[("accept-encoding", ask)]).await;
            assert!(headers.get(header::CONTENT_ENCODING).is_none(), "{ask}");
            assert_eq!(body, include_bytes!("../../../web/dist/app.js"), "{ask}");
        }
    }

    #[tokio::test]
    async fn the_gzip_tag_revalidates_the_gzip_copy() {
        let (_, headers, _) = fetch("/style.css", &[("accept-encoding", "gzip")]).await;
        let tag = headers[header::ETAG].to_str().unwrap().to_owned();
        let (status, _, body) = fetch(
            "/style.css",
            &[("accept-encoding", "gzip"), ("if-none-match", &tag)],
        )
        .await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn every_embedded_file_is_served_and_anything_else_is_not_found() {
        for (path, kind) in [
            ("/", "text/html"),
            ("/index.html", "text/html"),
            ("/favicon.svg", "image/svg+xml"),
            ("/app.js", "text/javascript"),
            ("/style.css", "text/css"),
            ("/sw.js", "text/javascript"),
            ("/manifest.webmanifest", "application/manifest+json"),
            ("/icon-192.png", "image/png"),
            ("/icon-512.png", "image/png"),
        ] {
            let (status, headers, body) = fetch(path, &[]).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(
                headers[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with(kind)
            );
            assert!(!body.is_empty());
        }
        assert_eq!(fetch("/nothing", &[]).await.0, StatusCode::NOT_FOUND);
    }
}
