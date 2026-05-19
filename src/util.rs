use scraper::{ElementRef, Html};
use std::time::Duration;
use url::Url;

use crate::cli::Args;
use crate::model::ContentBlock;
use crate::selectors::{DEFAULT_EXCLUDE_PATTERNS, SEL_BODY, SEL_SKIP};

/// Pull the human-readable text out of an element, inserting whitespace
/// between text nodes from different child elements so that
/// `<span>CRM</span><p>desc</p>` becomes `"CRM desc"` instead of `"CRMdesc"`.
pub(crate) fn element_text(el: &ElementRef) -> String {
    el.text()
        .flat_map(|s| s.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
}

pub(crate) fn url_matches_excludes(url: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| url.contains(p.as_str()))
}

/// Whitelist counterpart to `url_matches_excludes`. When the include list is
/// non-empty, a URL must contain at least one of the patterns. Empty list →
/// allow everything (no filtering).
pub(crate) fn url_matches_includes(url: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| url.contains(p.as_str()))
}

/// Canonicalise a URL for deduplication. Strips fragment, collapses
/// trailing slash on non-root paths, lowercases the host, drops common
/// tracking query params (utm_*, fbclid, gclid, ref, mc_*).
pub(crate) fn canonicalize_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };
    parsed.set_fragment(None);
    // Lowercase the host in place (Url doesn't expose a setter that doesn't
    // re-percent-encode; just rebuild from parts).
    if let Some(host) = parsed.host_str().map(|h| h.to_lowercase()) {
        let _ = parsed.set_host(Some(&host));
    }
    // Strip tracking params.
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| {
            let k_lc = k.to_lowercase();
            !(k_lc.starts_with("utm_")
                || k_lc == "fbclid"
                || k_lc == "gclid"
                || k_lc == "msclkid"
                || k_lc == "ref"
                || k_lc == "ref_src"
                || k_lc.starts_with("mc_"))
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        let q: String = kept
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        parsed.set_query(Some(&q));
    }
    // Collapse trailing slash on non-root paths.
    let path = parsed.path().to_string();
    if path.ends_with('/') && path.len() > 1 {
        parsed.set_path(path.trim_end_matches('/'));
    }
    parsed.to_string()
}

/// Undo Git Bash / MSYS automatic path translation on Windows. When the
/// user types `--exclude /home`, Git Bash silently rewrites that into
/// `C:/Program Files/Git/home` (or whatever the MSYS root is) before
/// invoking the binary. Detect that and recover the user's intended
/// leading-slash pattern. No-op on other shells / OSes.
pub(crate) fn unmsys_pattern(s: &str) -> String {
    // Heuristic: a pattern starting with a Windows drive letter that
    // includes a path segment after the MSYS root is almost certainly an
    // accidentally-translated `/foo` (no user would type that as a URL
    // substring pattern). Recover the last path component as `/last`.
    let bytes = s.as_bytes();
    let has_drive_prefix = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\');
    if !has_drive_prefix {
        return s.to_string();
    }
    // Common MSYS translation we want to undo cleanly.
    let known_roots = [
        "C:/Program Files/Git/",
        "C:\\Program Files\\Git\\",
        "C:/Program Files (x86)/Git/",
        "C:/msys64/",
        "C:/cygwin64/",
    ];
    for root in known_roots {
        if let Some(rest) = s.strip_prefix(root) {
            return format!("/{}", rest.trim_start_matches('/'));
        }
    }
    // Fallback for other MSYS roots: take just the trailing path segment.
    let trail = s.rsplit(['/', '\\']).next().unwrap_or(s);
    format!("/{trail}")
}

pub(crate) fn build_exclude_patterns(args: &Args) -> Vec<String> {
    let mut patterns: Vec<String> = if args.no_default_excludes {
        Vec::new()
    } else {
        DEFAULT_EXCLUDE_PATTERNS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    };
    patterns.extend(args.excludes.iter().map(|s| unmsys_pattern(s)));
    patterns
}

pub(crate) fn build_include_patterns(args: &Args) -> Vec<String> {
    args.includes.iter().map(|s| unmsys_pattern(s)).collect()
}

/// Map a `Content-Type` header value to a canonical image extension.
/// Returns `None` for non-image content types so the caller can fall back
/// to URL-based detection.
pub(crate) fn extension_from_content_type(ct: &str) -> Option<&'static str> {
    let ct_lc = ct.to_lowercase();
    if ct_lc.contains("image/jpeg") || ct_lc.contains("image/jpg") {
        Some("jpg")
    } else if ct_lc.contains("image/png") {
        Some("png")
    } else if ct_lc.contains("image/webp") {
        Some("webp")
    } else if ct_lc.contains("image/svg") {
        Some("svg")
    } else if ct_lc.contains("image/gif") {
        Some("gif")
    } else if ct_lc.contains("image/avif") {
        Some("avif")
    } else if ct_lc.contains("image/x-icon") || ct_lc.contains("image/vnd.microsoft.icon") {
        Some("ico")
    } else if ct_lc.contains("image/bmp") {
        Some("bmp")
    } else {
        None
    }
}

pub(crate) fn image_extension_from_url(url: &str) -> &'static str {
    let url_lc = url.to_lowercase();
    for ext_with_dot in [
        ".svg", ".png", ".jpeg", ".jpg", ".webp", ".gif", ".avif", ".bmp", ".ico",
    ] {
        if url_lc.contains(ext_with_dot) {
            return match ext_with_dot {
                ".svg" => "svg",
                ".png" => "png",
                ".jpeg" | ".jpg" => "jpg",
                ".webp" => "webp",
                ".gif" => "gif",
                ".avif" => "avif",
                ".bmp" => "bmp",
                ".ico" => "ico",
                _ => "jpg",
            };
        }
    }
    "jpg"
}

/// All human-readable text on a page (body text content), excluding script,
/// style, svg, and noscript subtrees. SVG `<path d="...">` data otherwise
/// looks a lot like phone numbers to a regex.
pub(crate) fn body_text_only(doc: &Html) -> String {
    let mut out = String::new();
    let Some(body) = doc.select(&SEL_BODY).next() else {
        return out;
    };
    for node in body.descendants() {
        if let Some(text) = node.value().as_text() {
            let mut skip = false;
            for anc in node.ancestors() {
                if let Some(anc_el) = ElementRef::wrap(anc) {
                    let n = anc_el.value().name();
                    if matches!(n, "script" | "style" | "svg" | "noscript") {
                        skip = true;
                        break;
                    }
                }
            }
            if !skip {
                out.push_str(text);
                out.push(' ');
            }
        }
    }
    out
}

pub(crate) fn heading_level_from_tag(tag: &str) -> u8 {
    tag.chars()
        .next_back()
        .and_then(|c| c.to_digit(10))
        .map(|d| d as u8)
        .unwrap_or(2)
}

/// Adjacent-duplicate filter that defends against JS-slider clones (Slick /
/// Swiper duplicate visible slide content for infinite-loop animation). Only
/// applies to long text so we don't accidentally collapse legitimate short
/// repeated labels.
pub(crate) fn dedup_adjacent_long_text(blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
    let mut result: Vec<ContentBlock> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let cur: Option<(u8, &str)> = match &block {
            ContentBlock::Paragraph { text } if text.len() > 30 => Some((0, text.as_str())),
            ContentBlock::Heading { level, text } if text.len() > 30 => {
                Some((*level, text.as_str()))
            }
            _ => None,
        };
        if let Some((cur_level, cur_text)) = cur {
            if let Some(prev) = result.last() {
                let prev_sig: Option<(u8, &str)> = match prev {
                    ContentBlock::Paragraph { text } => Some((0, text.as_str())),
                    ContentBlock::Heading { level, text } => Some((*level, text.as_str())),
                    _ => None,
                };
                if prev_sig == Some((cur_level, cur_text)) {
                    continue;
                }
            }
        }
        result.push(block);
    }
    result
}

/// Concatenate the text content of every Heading / Paragraph / List block
/// into a single newline-separated string. Used for `PageData.plain_text`.
pub(crate) fn blocks_to_plain_text(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            ContentBlock::Heading { text, .. } | ContentBlock::Paragraph { text } => {
                if !text.is_empty() {
                    out.push_str(text);
                    out.push('\n');
                }
            }
            ContentBlock::List { items } => {
                for item in items {
                    if !item.is_empty() {
                        out.push_str(item);
                        out.push('\n');
                    }
                }
            }
            _ => {}
        }
    }
    out.trim_end().to_string()
}

pub(crate) fn count_words(blocks: &[ContentBlock]) -> usize {
    blocks.iter().fold(0, |acc, b| {
        acc + match b {
            ContentBlock::Heading { text, .. } | ContentBlock::Paragraph { text } => {
                text.split_whitespace().count()
            }
            ContentBlock::List { items } => {
                items.iter().map(|s| s.split_whitespace().count()).sum()
            }
            ContentBlock::Table {
                headers,
                rows,
                caption,
            } => {
                let mut n = 0;
                if let Some(c) = caption {
                    n += c.split_whitespace().count();
                }
                n += headers
                    .iter()
                    .map(|s| s.split_whitespace().count())
                    .sum::<usize>();
                n += rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .map(|s| s.split_whitespace().count())
                    .sum::<usize>();
                n
            }
            ContentBlock::Code { text, .. } | ContentBlock::Quote { text, .. } => {
                text.split_whitespace().count()
            }
            ContentBlock::DefinitionList { items } => items
                .iter()
                .map(|i| {
                    i.term.split_whitespace().count() + i.description.split_whitespace().count()
                })
                .sum(),
            ContentBlock::Image { .. }
            | ContentBlock::Form { .. }
            | ContentBlock::Embed { .. }
            | ContentBlock::Media { .. } => 0,
        }
    })
}

/// Heuristic form-purpose classifier. Looks at field types, names, and
/// placeholders to label a form as contact / newsletter / search / login /
/// signup / payment / comment / generic.
pub(crate) fn classify_form_purpose(
    fields: &[crate::model::FormField],
    submit_text: &str,
    action: &str,
) -> String {
    let action_lc = action.to_lowercase();
    let submit_lc = submit_text.to_lowercase();

    let mut has_email = false;
    let mut has_password = false;
    let mut has_message = false;
    let mut has_card = false;
    let mut has_name = false;
    let mut has_search = false;
    let mut field_count_non_hidden = 0;
    let mut joined_names = String::new();

    for f in fields {
        field_count_non_hidden += 1;
        let n = f.name.to_lowercase();
        let l = f.label.to_lowercase();
        let p = f.placeholder.to_lowercase();
        let t = f.field_type.to_lowercase();
        let combined = format!("{n} {l} {p} {t}");
        joined_names.push_str(&n);
        joined_names.push(' ');

        if t == "email" || combined.contains("email") || combined.contains("e-mail") {
            has_email = true;
        }
        if t == "password" {
            has_password = true;
        }
        if t == "search" || n == "q" || n == "s" || n == "query" || n == "search" {
            has_search = true;
        }
        if t == "textarea"
            || combined.contains("message")
            || combined.contains("comment")
            || combined.contains("zpráv")
            || combined.contains("komentář")
        {
            has_message = true;
        }
        if n.contains("card")
            || n.contains("cardnumber")
            || n.contains("cc-number")
            || combined.contains("credit card")
            || combined.contains("card number")
            || combined.contains("cvv")
            || combined.contains("cvc")
        {
            has_card = true;
        }
        if n.contains("name") || combined.contains("jméno") || combined.contains("first name") {
            has_name = true;
        }
    }

    let signup_words = action_lc.contains("signup")
        || action_lc.contains("register")
        || submit_lc.contains("sign up")
        || submit_lc.contains("create account")
        || submit_lc.contains("register")
        || submit_lc.contains("registrovat");
    let login_words = action_lc.contains("login")
        || action_lc.contains("signin")
        || submit_lc.contains("log in")
        || submit_lc.contains("sign in")
        || submit_lc.contains("přihlásit");
    let comment_words = action_lc.contains("comment")
        || action_lc.contains("wp-comments-post")
        || submit_lc.contains("comment")
        || submit_lc.contains("odeslat komentář");
    let newsletter_words = action_lc.contains("subscribe")
        || action_lc.contains("newsletter")
        || submit_lc.contains("subscribe")
        || submit_lc.contains("odebírat");

    if has_search && field_count_non_hidden <= 2 {
        return "search".to_string();
    }
    if has_card {
        return "payment".to_string();
    }
    if comment_words || (has_message && has_email && has_name && field_count_non_hidden <= 6) {
        // The Wordpress comment form classically has name + email + url + comment.
        if comment_words {
            return "comment".to_string();
        }
        // Otherwise fall through to contact (it's also a contact-shaped form).
    }
    if has_password && (login_words || !signup_words) && field_count_non_hidden <= 4 {
        return "login".to_string();
    }
    if has_password && signup_words {
        return "signup".to_string();
    }
    if newsletter_words || (has_email && !has_message && field_count_non_hidden <= 2) {
        return "newsletter".to_string();
    }
    if has_email && has_message {
        return "contact".to_string();
    }
    "generic".to_string()
}

pub(crate) fn embed_provider_from_src(src: &str) -> &'static str {
    let s = src.to_lowercase();
    if s.contains("youtube.com") || s.contains("youtu.be") || s.contains("youtube-nocookie.com") {
        "youtube"
    } else if s.contains("vimeo.com") {
        "vimeo"
    } else if s.contains("google.com/maps") || s.contains("maps.google.") {
        "maps"
    } else if s.contains("spotify.com") {
        "spotify"
    } else if s.contains("soundcloud.com") {
        "soundcloud"
    } else if s.contains("twitter.com") || s.contains("x.com") {
        "twitter"
    } else if s.contains("instagram.com") {
        "instagram"
    } else if s.contains("facebook.com") {
        "facebook"
    } else if s.contains("typeform.com") {
        "typeform"
    } else if s.contains("calendly.com") {
        "calendly"
    } else if s.contains("hubspot") {
        "hubspot"
    } else {
        "iframe"
    }
}

pub(crate) fn element_in_skip_zone(el: &ElementRef) -> bool {
    if SEL_SKIP.matches(el) {
        return true;
    }
    for anc in el.ancestors() {
        if let Some(anc_el) = ElementRef::wrap(anc) {
            if SEL_SKIP.matches(&anc_el) {
                return true;
            }
        }
    }
    false
}

/// HTTP fetch with exponential backoff. Retries on 5xx + connect/timeout
/// errors with 200ms → 600ms → 1800ms → 5400ms delays (capped at 10s).
/// Returns `Some(response)` on success or final non-retriable response;
/// `None` only when every attempt errored out.
pub(crate) async fn fetch_with_retry(
    client: &reqwest::Client,
    url: &str,
    max_retries: u32,
) -> Option<reqwest::Response> {
    let mut delay = Duration::from_millis(200);
    for attempt in 0..=max_retries {
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() || !status.is_server_error() {
                    // Success or non-retriable (4xx) — return as-is.
                    return Some(resp);
                }
                if attempt == max_retries {
                    return Some(resp);
                }
                tracing::warn!(
                    "Retry {}/{} for {url} (status {})",
                    attempt + 1,
                    max_retries,
                    status
                );
            }
            Err(e) => {
                let transient = e.is_timeout() || e.is_connect() || e.is_request();
                if !transient || attempt == max_retries {
                    // Always log so the caller knows WHY None is being returned.
                    // Previously non-transient errors (redirect loops, body
                    // decode, TLS issues) were swallowed silently.
                    tracing::error!("Fetch failed for {url}: {e}");
                    return None;
                }
                tracing::warn!("Retry {}/{} for {url}: {e}", attempt + 1, max_retries);
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 3).min(Duration::from_secs(10));
    }
    None
}

/// Parsed robots.txt rules that apply to our user-agent (`*` or `DumpIt`).
pub(crate) struct RobotsRules {
    pub disallow: Vec<String>,
    pub crawl_delay_ms: Option<u64>,
}

/// Parse a robots.txt body and return Disallow paths + Crawl-delay for our
/// user-agent. The largest Crawl-delay across matching groups wins.
pub(crate) fn parse_robots(body: &str) -> RobotsRules {
    let mut disallow: Vec<String> = Vec::new();
    let mut crawl_delay_ms: Option<u64> = None;
    let mut applies = false;
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or(raw).trim();
        if line.is_empty() {
            applies = false;
            continue;
        }
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("user-agent:") {
            let ua = rest.trim();
            applies = ua == "*" || ua.contains("dumpit");
        } else if applies {
            if let Some(idx) = lower.find("disallow:") {
                let path = line[idx + "disallow:".len()..].trim().to_string();
                if !path.is_empty() {
                    disallow.push(path);
                }
            } else if let Some(idx) = lower.find("crawl-delay:") {
                let val = line[idx + "crawl-delay:".len()..].trim();
                if let Ok(secs) = val.parse::<f64>() {
                    let ms = (secs * 1000.0) as u64;
                    if crawl_delay_ms.is_none_or(|cur| cur < ms) {
                        crawl_delay_ms = Some(ms);
                    }
                }
            }
        }
    }
    RobotsRules {
        disallow,
        crawl_delay_ms,
    }
}

/// Rate limiter that enforces a minimum gap between requests across all
/// concurrent tasks. Holds the lock across the sleep so the next caller
/// naturally queues behind us.
pub(crate) struct RateLimiter {
    last: tokio::sync::Mutex<std::time::Instant>,
    min_gap: Duration,
}

impl RateLimiter {
    pub fn new(delay_ms: u64) -> Option<std::sync::Arc<Self>> {
        if delay_ms == 0 {
            return None;
        }
        Some(std::sync::Arc::new(Self {
            // Initialise far enough in the past that the first request fires
            // immediately.
            last: tokio::sync::Mutex::new(std::time::Instant::now() - Duration::from_secs(3600)),
            min_gap: Duration::from_millis(delay_ms),
        }))
    }

    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        let now = std::time::Instant::now();
        let target = *last + self.min_gap;
        if now < target {
            tokio::time::sleep(target - now).await;
            *last = std::time::Instant::now();
        } else {
            *last = now;
        }
    }
}

pub(crate) fn is_disallowed_by_robots(url: &str, rules: &[String]) -> bool {
    if rules.is_empty() {
        return false;
    }
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let path = parsed.path();
    for rule in rules {
        if rule == "/" {
            return true;
        }
        // robots.txt uses prefix matching; `$` anchors end, `*` is wildcard.
        // For now we honour prefix + `$` (anchored). Wildcards in the middle
        // are uncommon; treat them as literal "*" matching themselves.
        if let Some(stripped) = rule.strip_suffix('$') {
            if path == stripped {
                return true;
            }
        } else if path.starts_with(rule.as_str()) {
            return true;
        }
    }
    false
}

/// Filesystem-safe slug for a page URL. `https://x.com/foo/bar` → `foo-bar`,
/// the bare root URL → `home`.
pub(crate) fn url_to_slug(url: &str) -> String {
    let parsed = Url::parse(url).ok();
    let path = parsed.as_ref().map(|u| u.path()).unwrap_or("/");
    let mut slug: String = path
        .trim_matches('/')
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            '/' => '-',
            _ => '-',
        })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "home".to_string()
    } else if slug.len() > 80 {
        slug[..80].to_string()
    } else {
        slug
    }
}

/// Sort key that pushes high-value pages to the front so `--max-pages`
/// truncation drops low-value pages first.
///
/// Order:
///   0   home page (`/`)
///   1   contact / get-in-touch
///   2   about
///   3   legal / privacy / terms / imprint
/// 100   default content (products, blog, services)
/// 200   utility pages — login, signup, cart, checkout, search, account.
///       Schoolhouse smoketest wasted 4/10 slots on /account/login,
///       /account/password/recover, /search, /pages/terms — pushing
///       these to the BOTTOM of the priority list means they only get
///       scraped when the cap is generous.
pub(crate) fn url_priority(url: &str) -> u32 {
    let Ok(parsed) = Url::parse(url) else {
        return 100;
    };
    let path = parsed.path().to_lowercase();
    if path == "/" || path.is_empty() {
        return 0;
    }
    if path.contains("/contact") || path.contains("/kontakt") || path.ends_with("/get-in-touch") {
        return 1;
    }
    if path.contains("/about") {
        return 2;
    }
    if path.contains("/imprint")
        || path.contains("/impressum")
        || path.contains("/legal")
        || path.contains("/privacy")
        || path.contains("/terms")
    {
        return 3;
    }
    // Utility / auth / checkout endpoints — useful for the agent to KNOW
    // they exist, but the bundle shouldn't waste page-cap slots on them.
    if path.contains("/account/")
        || path.contains("/cart")
        || path.contains("/checkout")
        || path.starts_with("/search")
        || path.contains("/login")
        || path.contains("/signup")
        || path.contains("/register")
        || path.contains("/password")
        || path.contains("/forgot")
        || path.contains("/sign-in")
        || path.contains("/sign-up")
        || path.contains("/api/")
    {
        return 200;
    }
    100
}

/// Filesystem-safe host slug for a URL. Strips `www.`, replaces dots with
/// underscores, drops non-alphanumeric characters. `https://www.foo-bar.cz/x`
/// → `foo-bar_cz`. Used by `--test-run` to derive `test_runs/<slug>/`.
pub(crate) fn url_to_host_slug(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "site".to_string();
    };
    let host = parsed.host_str().unwrap_or("site");
    let stripped = host.strip_prefix("www.").unwrap_or(host);
    let mut slug = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' => slug.push(c),
            _ => slug.push('_'),
        }
    }
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    let trimmed = slug.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "site".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_priority_pushes_chrome_pages_to_front() {
        assert_eq!(url_priority("https://x.com/"), 0);
        assert_eq!(url_priority("https://x.com/contact"), 1);
        assert_eq!(url_priority("https://x.com/contact-us"), 1);
        assert_eq!(url_priority("https://x.com/kontakt"), 1);
        assert_eq!(url_priority("https://x.com/about"), 2);
        assert_eq!(url_priority("https://x.com/legal"), 3);
        assert_eq!(url_priority("https://x.com/privacy-policy"), 3);
        assert_eq!(url_priority("https://x.com/products/cookies"), 100);
        assert_eq!(url_priority("https://x.com/blog/post"), 100);
        // Utility / auth / checkout endpoints get pushed to the BOTTOM —
        // Round I regression: Schoolhouse wasted 4/10 slots on
        // /account/login, /account/password/recover, /search.
        assert_eq!(url_priority("https://x.com/account/login"), 200);
        assert_eq!(url_priority("https://x.com/cart"), 200);
        assert_eq!(url_priority("https://x.com/checkout/step-1"), 200);
        assert_eq!(url_priority("https://x.com/search"), 200);
        assert_eq!(url_priority("https://x.com/account/password/recover"), 200);
        // Regression: sort key keeps home above everything and utility at end.
        let mut urls = vec![
            "https://x.com/products/widget".to_string(),
            "https://x.com/account/login".to_string(),
            "https://x.com/blog/post".to_string(),
            "https://x.com/contact".to_string(),
            "https://x.com/".to_string(),
        ];
        urls.sort_by_key(|u| url_priority(u));
        assert_eq!(urls[0], "https://x.com/");
        assert_eq!(urls[1], "https://x.com/contact");
        assert_eq!(urls[4], "https://x.com/account/login");
    }

    #[test]
    fn url_to_host_slug_strips_www_and_normalises() {
        assert_eq!(url_to_host_slug("https://sportujusa.cz/"), "sportujusa_cz");
        assert_eq!(
            url_to_host_slug("https://www.example.com/path"),
            "example_com"
        );
        assert_eq!(
            url_to_host_slug("https://foo-bar.co.uk/x"),
            "foo-bar_co_uk"
        );
        assert_eq!(url_to_host_slug("not-a-url"), "site");
        assert_eq!(url_to_host_slug("https://x.com"), "x_com");
    }

    #[test]
    fn image_ext_from_url_handles_proxy() {
        assert_eq!(
            image_extension_from_url("https://x.com/_next/image?url=%2Flogo.png&w=96"),
            "png"
        );
        assert_eq!(image_extension_from_url("https://x.com/a/b/c.SVG"), "svg");
        assert_eq!(image_extension_from_url("https://x.com/a.jpg?v=1"), "jpg");
        assert_eq!(image_extension_from_url("https://x.com/a.jpeg"), "jpg");
        assert_eq!(image_extension_from_url("https://x.com/photo"), "jpg"); // fallback
        assert_eq!(image_extension_from_url("https://x.com/x.webp"), "webp");
    }

    #[test]
    fn extension_from_content_type_maps_correctly() {
        assert_eq!(extension_from_content_type("image/png"), Some("png"));
        assert_eq!(extension_from_content_type("image/jpeg"), Some("jpg"));
        assert_eq!(extension_from_content_type("image/svg+xml"), Some("svg"));
        assert_eq!(extension_from_content_type("text/html"), None);
        assert_eq!(
            extension_from_content_type("image/webp; charset=utf-8"),
            Some("webp")
        );
    }

    #[test]
    fn parse_robots_picks_star_and_dumpit_groups() {
        let body = r#"
User-agent: *
Disallow: /admin/
Crawl-delay: 2

User-agent: DumpIt
Disallow: /private/

User-agent: BadBot
Disallow: /
"#;
        let rules = parse_robots(body);
        assert!(rules.disallow.contains(&"/admin/".to_string()));
        assert!(rules.disallow.contains(&"/private/".to_string()));
        assert!(!rules.disallow.iter().any(|r| r == "/"));
        assert_eq!(rules.crawl_delay_ms, Some(2000));
    }

    #[test]
    fn is_disallowed_by_robots_prefix_match() {
        let rules = vec!["/admin/".to_string(), "/api$".to_string()];
        assert!(is_disallowed_by_robots("https://x.com/admin/users", &rules));
        assert!(!is_disallowed_by_robots("https://x.com/", &rules));
        assert!(is_disallowed_by_robots("https://x.com/api", &rules));
        assert!(!is_disallowed_by_robots("https://x.com/api/", &rules));
    }

    #[test]
    fn unmsys_pattern_recovers_git_bash_paths() {
        // Git Bash translates `/home` to `C:/Program Files/Git/home` before
        // it reaches the binary. Recover the original.
        assert_eq!(unmsys_pattern("C:/Program Files/Git/home"), "/home");
        assert_eq!(unmsys_pattern("C:/Program Files/Git/contact"), "/contact");
        assert_eq!(
            unmsys_pattern("C:\\Program Files\\Git\\services"),
            "/services"
        );
        // Generic MSYS root fallback.
        assert_eq!(unmsys_pattern("D:/msys-root/wp-admin"), "/wp-admin");
        // Non-translated input passes through unchanged.
        assert_eq!(unmsys_pattern("/home"), "/home");
        assert_eq!(unmsys_pattern("?elementor"), "?elementor");
        assert_eq!(unmsys_pattern(""), "");
    }

    #[test]
    fn url_to_slug_normalises() {
        assert_eq!(url_to_slug("https://x.com/"), "home");
        assert_eq!(url_to_slug("https://x.com/foo/bar"), "foo-bar");
        assert_eq!(url_to_slug("https://x.com/foo--bar"), "foo-bar");
        assert_eq!(url_to_slug("https://x.com/o-nas/lukas"), "o-nas-lukas");
        assert!(!url_to_slug("https://x.com/").contains('/'));
    }

    #[test]
    fn classify_form_purpose_recognises_common_shapes() {
        use crate::model::FormField;
        let f = |name: &str, ty: &str| FormField {
            field_type: ty.to_string(),
            name: name.to_string(),
            label: String::new(),
            placeholder: String::new(),
            required: false,
            options: vec![],
        };
        let contact = vec![
            f("name", "text"),
            f("email", "email"),
            f("message", "textarea"),
        ];
        assert_eq!(
            classify_form_purpose(&contact, "Send", "/contact"),
            "contact"
        );

        let newsletter = vec![f("email", "email")];
        assert_eq!(
            classify_form_purpose(&newsletter, "Subscribe", ""),
            "newsletter"
        );

        let search = vec![f("q", "search")];
        assert_eq!(classify_form_purpose(&search, "Search", ""), "search");

        let login = vec![f("email", "email"), f("password", "password")];
        assert_eq!(classify_form_purpose(&login, "Log in", "/login"), "login");

        let payment = vec![f("cc-number", "text"), f("cvc", "text")];
        assert_eq!(classify_form_purpose(&payment, "Pay", "/pay"), "payment");
    }

    #[test]
    fn blocks_to_plain_text_skips_non_text() {
        use crate::model::ContentBlock;
        let blocks = vec![
            ContentBlock::Heading {
                level: 1,
                text: "Hello".to_string(),
            },
            ContentBlock::Paragraph {
                text: "World".to_string(),
            },
            ContentBlock::Image {
                original_url: "x".to_string(),
                local_path: "".to_string(),
                alt_text: "".to_string(),
            },
            ContentBlock::List {
                items: vec!["a".to_string(), "b".to_string()],
            },
        ];
        assert_eq!(blocks_to_plain_text(&blocks), "Hello\nWorld\na\nb");
    }
}
