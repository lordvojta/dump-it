use futures::future;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::fs;
use url::Url;

use crate::model::{ContentBlock, DefinitionItem, FormField, HreflangAlternate, NavLink};
use crate::selectors::{
    SEL_CANONICAL, SEL_CAPTION, SEL_CODE_INSIDE_PRE, SEL_DD, SEL_DT, SEL_FAVICON, SEL_FIGCAPTION,
    SEL_FOOTER, SEL_HEADER_IMG, SEL_HREFLANG, SEL_HTML, SEL_INPUT, SEL_JSONLD, SEL_LI, SEL_LINK,
    SEL_MAIN, SEL_META, SEL_NAV, SEL_OPTION, SEL_STYLESHEET, SEL_STYLE_BLOCK, SEL_SUBMIT, SEL_TD,
    SEL_TH, SEL_TITLE, SEL_TR, SEL_VIDEO_SOURCE,
};
use crate::util::{
    classify_form_purpose, element_in_skip_zone, element_text, embed_provider_from_src,
    fetch_with_retry, heading_level_from_tag, image_extension_from_url, normalize_path,
};

#[allow(clippy::type_complexity)]
pub(crate) fn extract_meta(
    doc: &Html,
) -> (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let title = doc
        .select(&SEL_TITLE)
        .next()
        .map(|el| element_text(&el))
        .unwrap_or_else(|| "No title".to_string());

    let mut meta_title = String::new();
    let mut meta_description = String::new();
    let mut og_image: Option<String> = None;
    let mut twitter_card: Option<String> = None;
    let mut meta_robots: Option<String> = None;

    for element in doc.select(&SEL_META) {
        if let Some(property) = element.value().attr("property") {
            match property {
                "og:title" => {
                    meta_title = element.value().attr("content").unwrap_or("").to_string();
                }
                "og:description" if meta_description.is_empty() => {
                    meta_description = element.value().attr("content").unwrap_or("").to_string();
                }
                "og:image" if og_image.is_none() => {
                    let v = element.value().attr("content").unwrap_or("").trim();
                    if !v.is_empty() {
                        og_image = Some(v.to_string());
                    }
                }
                _ => {}
            }
        } else if let Some(name) = element.value().attr("name") {
            match name {
                "title" if meta_title.is_empty() => {
                    meta_title = element.value().attr("content").unwrap_or("").to_string();
                }
                "description" if meta_description.is_empty() => {
                    meta_description = element.value().attr("content").unwrap_or("").to_string();
                }
                "twitter:card" if twitter_card.is_none() => {
                    let v = element.value().attr("content").unwrap_or("").trim();
                    if !v.is_empty() {
                        twitter_card = Some(v.to_string());
                    }
                }
                "twitter:image" if og_image.is_none() => {
                    let v = element.value().attr("content").unwrap_or("").trim();
                    if !v.is_empty() {
                        og_image = Some(v.to_string());
                    }
                }
                "robots" if meta_robots.is_none() => {
                    let v = element.value().attr("content").unwrap_or("").trim();
                    if !v.is_empty() {
                        meta_robots = Some(v.to_lowercase());
                    }
                }
                _ => {}
            }
        }
    }

    if meta_title.is_empty() {
        meta_title = title.clone();
    }

    (
        title,
        meta_title,
        meta_description,
        og_image,
        twitter_card,
        meta_robots,
    )
}

pub(crate) fn extract_canonical(doc: &Html, base: &Url) -> Option<String> {
    doc.select(&SEL_CANONICAL)
        .next()
        .and_then(|el| el.value().attr("href"))
        .and_then(|href| base.join(href).ok())
        .map(|u| u.to_string())
}

pub(crate) fn extract_language(doc: &Html) -> Option<String> {
    doc.select(&SEL_HTML)
        .next()
        .and_then(|el| el.value().attr("lang"))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

pub(crate) fn extract_favicon(doc: &Html, base: &Url) -> Option<String> {
    let mut best: Option<(i32, String)> = None;
    for el in doc.select(&SEL_FAVICON) {
        let rel = el.value().attr("rel").unwrap_or("").to_lowercase();
        let href = el.value().attr("href").unwrap_or("");
        if href.is_empty() {
            continue;
        }
        let pri = if rel.contains("apple-touch-icon") {
            3
        } else if rel == "icon" {
            2
        } else if rel.contains("shortcut") {
            1
        } else {
            0
        };
        if let Ok(abs) = base.join(href) {
            if best.as_ref().is_none_or(|(p, _)| pri > *p) {
                best = Some((pri, abs.to_string()));
            }
        }
    }
    best.map(|(_, u)| u)
}

pub(crate) fn extract_hreflang(doc: &Html, base: &Url) -> Vec<HreflangAlternate> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for el in doc.select(&SEL_HREFLANG) {
        let Some(lang) = el.value().attr("hreflang") else {
            continue;
        };
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        let lang_lc = lang.to_lowercase();
        // Filter accidental empties / `x-default` keep
        if lang_lc.is_empty() {
            continue;
        }
        let Ok(abs) = base.join(href) else { continue };
        let url = abs.to_string();
        if seen.insert((lang_lc.clone(), url.clone())) {
            out.push(HreflangAlternate { lang: lang_lc, url });
        }
    }
    out
}

pub(crate) fn extract_structured_data(doc: &Html) -> Vec<JsonValue> {
    doc.select(&SEL_JSONLD)
        .filter_map(|el| {
            let text = el.text().collect::<String>();
            serde_json::from_str::<JsonValue>(&text).ok()
        })
        .collect()
}

/// Best-effort logo URL. Walks header / logo-class images and the
/// `Organization.logo.url` field from JSON-LD as a fallback.
pub(crate) fn extract_logo_url(doc: &Html, base: &Url, structured: &[JsonValue]) -> Option<String> {
    for el in doc.select(&SEL_HEADER_IMG) {
        let tag = el.value().name();
        if tag == "img" {
            let src = el
                .value()
                .attr("src")
                .or_else(|| el.value().attr("data-src"))
                .unwrap_or("");
            if src.is_empty() {
                continue;
            }
            if let Ok(abs) = base.join(src) {
                return Some(abs.to_string());
            }
        } else if tag == "svg" {
            return Some("inline-svg://logo".to_string());
        }
    }

    fn walk(value: &JsonValue) -> Option<String> {
        if let Some(obj) = value.as_object() {
            if let Some(t) = obj.get("@type").and_then(|v| v.as_str()) {
                if t.eq_ignore_ascii_case("Organization")
                    || t.eq_ignore_ascii_case("LocalBusiness")
                    || t.eq_ignore_ascii_case("WebSite")
                {
                    if let Some(logo) = obj.get("logo") {
                        if let Some(s) = logo.as_str() {
                            return Some(s.to_string());
                        }
                        if let Some(o) = logo.as_object() {
                            if let Some(url) = o.get("url").and_then(|v| v.as_str()) {
                                return Some(url.to_string());
                            }
                        }
                    }
                }
            }
            for v in obj.values() {
                if let Some(found) = walk(v) {
                    return Some(found);
                }
            }
        }
        if let Some(arr) = value.as_array() {
            for v in arr {
                if let Some(found) = walk(v) {
                    return Some(found);
                }
            }
        }
        None
    }
    for v in structured {
        if let Some(s) = walk(v) {
            return Some(s);
        }
    }
    None
}

/// Extract nav links from the page chrome. Splits mega-menu anchors
/// (heading + paragraph inside one `<a>`) into a short `text` label and a
/// longer `description`, and tags each link with a `role` so the agent
/// can rebuild header / mega-menu / utility / social separately.
pub(crate) fn extract_nav_links(doc: &Html, base: &Url) -> Vec<NavLink> {
    use crate::selectors::SOCIAL_DOMAINS;

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut links = Vec::new();
    for nav_el in doc.select(&SEL_NAV) {
        let nav_tag = nav_el.value().name();
        for link in nav_el.select(&SEL_LINK) {
            let href = link.value().attr("href").unwrap_or("");
            if href.is_empty()
                || href.starts_with("javascript:")
                || href.starts_with('#')
                || href.starts_with("mailto:")
                || href.starts_with("tel:")
            {
                continue;
            }

            // Probe for a nested heading. Mega-menu anchors typically wrap
            // <h3>Title</h3><p>Description</p>. When present, use the
            // heading text as the label and treat the rest as description.
            let label: String;
            let mut description: Option<String> = None;
            let heading_sel = Selector::parse("h1, h2, h3, h4, h5, h6")
                .expect("static selector");
            if let Some(heading) = link.select(&heading_sel).next() {
                label = element_text(&heading);
                let full = element_text(&link);
                let trimmed = full.trim_start_matches(&label).trim();
                if !trimmed.is_empty() && trimmed.len() < 500 {
                    description = Some(trimmed.to_string());
                }
            } else {
                // No heading — use first text node only if the anchor has
                // mixed text+inline content; otherwise the whole text.
                let full = element_text(&link);
                // If text is suspiciously long for a nav label (>80 chars
                // AND has multiple sentences), take the first sentence as
                // label and shove the rest into description.
                if full.len() > 80 && (full.contains(". ") || full.contains(": ")) {
                    let split_at = full
                        .find(". ")
                        .or_else(|| full.find(": "))
                        .unwrap_or(full.len());
                    label = full[..split_at].trim().to_string();
                    let desc = full[split_at..].trim_start_matches([':', '.', ' ']).trim();
                    if !desc.is_empty() {
                        description = Some(desc.to_string());
                    }
                } else {
                    label = full;
                }
            }
            if label.is_empty() {
                continue;
            }

            let Ok(abs) = base.join(href) else { continue };
            let url_str = abs.to_string();
            if !seen.insert((label.clone(), url_str.clone())) {
                continue;
            }

            // Classify role.
            let host_lc = abs.host_str().unwrap_or("").to_lowercase();
            let is_social = SOCIAL_DOMAINS
                .iter()
                .any(|(_, d)| host_lc == *d || host_lc.ends_with(&format!(".{d}")));
            let label_lc = label.to_lowercase();
            let utility_keywords =
                ["login", "log in", "sign in", "sign up", "pricing", "search", "cart", "checkout", "account"];
            let is_utility = utility_keywords.iter().any(|k| label_lc == *k);
            let role = if is_social {
                Some("social".to_string())
            } else if description.is_some() {
                Some("mega_menu".to_string())
            } else if is_utility {
                Some("utility".to_string())
            } else if matches!(nav_tag, "footer") {
                Some("footer".to_string())
            } else {
                Some("header".to_string())
            };

            links.push(NavLink {
                text: label,
                href: url_str,
                description,
                role,
            });
        }
    }
    links
}

pub(crate) fn extract_footer_blocks(doc: &Html) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let mut seen_texts: HashSet<String> = HashSet::new();

    // Collect footer-like elements. Primary path: <footer> + [role='contentinfo'].
    // Fallback for React/Tailwind/SPAs that don't emit a semantic <footer>:
    // try [class*='footer' i] and [id*='footer' i]. Only use the fallback
    // when the primary path returned nothing — otherwise we double-count.
    let primary: Vec<_> = doc.select(&SEL_FOOTER).collect();
    let fallback_sel = Selector::parse("[class*='footer' i], [id*='footer' i]").ok();
    let fallback: Vec<_> = if primary.is_empty() {
        match fallback_sel.as_ref() {
            Some(sel) => doc
                .select(sel)
                // Drop fallbacks that live inside another fallback (avoid
                // capturing a nested .footer-link inside .footer-wrapper).
                .filter(|el| {
                    !el.ancestors().filter_map(ElementRef::wrap).any(|anc| {
                        let cls = anc.value().attr("class").unwrap_or("").to_lowercase();
                        let id = anc.value().attr("id").unwrap_or("").to_lowercase();
                        anc.value().name() != el.value().name()
                            && (cls.contains("footer") || id.contains("footer"))
                    })
                })
                .collect(),
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let footers = if primary.is_empty() { fallback } else { primary };

    for footer_el in footers {
        for node in footer_el.descendants() {
            let Some(el) = ElementRef::wrap(node) else {
                continue;
            };

            let mut skip = false;
            for anc in el.ancestors() {
                if let Some(anc_el) = ElementRef::wrap(anc) {
                    let n = anc_el.value().name();
                    if matches!(n, "script" | "style" | "noscript")
                        || anc_el.value().attr("aria-hidden") == Some("true")
                    {
                        skip = true;
                        break;
                    }
                }
            }
            if skip {
                continue;
            }

            let tag = el.value().name();
            if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                let level = heading_level_from_tag(tag);
                let text = element_text(&el);
                if !text.is_empty() && seen_texts.insert(text.clone()) {
                    blocks.push(ContentBlock::Heading { level, text });
                }
            } else if tag == "p" {
                let text = element_text(&el);
                if text.len() > 5 && seen_texts.insert(text.clone()) {
                    blocks.push(ContentBlock::Paragraph { text });
                }
            } else if matches!(tag, "ul" | "ol") {
                let parent_is_list = el
                    .parent()
                    .and_then(ElementRef::wrap)
                    .is_some_and(|p| matches!(p.value().name(), "ul" | "ol" | "li"));
                if parent_is_list {
                    continue;
                }
                let items: Vec<String> = el
                    .select(&SEL_LI)
                    .map(|li| element_text(&li))
                    .filter(|s| !s.is_empty())
                    .collect();
                if !items.is_empty() {
                    blocks.push(ContentBlock::List { items });
                }
            }
        }
    }
    blocks
}

pub(crate) fn extract_style_text(doc: &Html) -> String {
    let mut buf = String::new();
    for el in doc.select(&SEL_STYLE_BLOCK) {
        buf.push_str(&el.text().collect::<String>());
        buf.push('\n');
    }
    use crate::selectors::SEL_BODY;
    if let Some(body) = doc.select(&SEL_BODY).next() {
        for node in body.descendants() {
            if let Some(el) = ElementRef::wrap(node) {
                if let Some(style) = el.value().attr("style") {
                    buf.push_str(style);
                    buf.push('\n');
                }
                // Also pull HTML4-style `bgcolor=` and `color=` attributes
                // — Hacker News and other legacy table-based sites put their
                // brand color here, not in CSS. Wrap as faux CSS so the
                // existing color regexes match them.
                for attr in ["bgcolor", "color"] {
                    if let Some(v) = el.value().attr(attr) {
                        let v = v.trim();
                        if !v.is_empty() {
                            buf.push_str("background:");
                            if !v.starts_with('#') && !v.starts_with("rgb") && !v.starts_with("hsl")
                            {
                                buf.push('#');
                            }
                            buf.push_str(v);
                            buf.push(';');
                            buf.push('\n');
                        }
                    }
                }
            }
        }
    }
    buf
}

/// Collect every `<link rel="stylesheet">` URL, resolved to absolute.
pub(crate) fn extract_stylesheet_urls(doc: &Html, base: &Url) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for el in doc.select(&SEL_STYLESHEET) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if href.starts_with("javascript:") || href.is_empty() {
            continue;
        }
        if let Ok(abs) = base.join(href) {
            let s = abs.to_string();
            if seen.insert(s.clone()) {
                urls.push(s);
            }
        }
    }
    urls
}

/// All internal anchor hrefs (same-host as base_url), resolved to absolute.
/// Used to build the link graph.
pub(crate) fn extract_internal_links(doc: &Html, base: &Url) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let Some(base_host) = base.host_str() else {
        return out;
    };
    for el in doc.select(&SEL_LINK) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if href.starts_with("javascript:")
            || href.starts_with("mailto:")
            || href.starts_with("tel:")
            || href.starts_with('#')
        {
            continue;
        }
        let Ok(abs) = base.join(href) else { continue };
        if abs.host_str() != Some(base_host) {
            continue;
        }
        // Strip fragment for stable comparison.
        let mut clean = abs.clone();
        clean.set_fragment(None);
        let s = clean.to_string();
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

pub(crate) async fn download_image(
    client: &Client,
    img_url: &str,
    output_dir: &str,
) -> Option<String> {
    let lower_url = img_url.to_lowercase();
    let tracking_domains = [
        "googletagmanager",
        "google-analytics",
        "facebook.com/tr",
        "doubleclick",
        "analytics",
        "tracking",
        "pixel",
        "beacon",
    ];
    for domain in &tracking_domains {
        if lower_url.contains(domain) {
            return None;
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(img_url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let extension = image_extension_from_url(img_url);
    let filename = format!("{}.{}", &hash[..16], extension);
    let filepath = format!("{output_dir}/{filename}");

    if Path::new(&filepath).exists() {
        return Some(normalize_path(&filepath));
    }

    match fetch_with_retry(client, img_url, 2).await {
        Some(response) if response.status().is_success() => {
            if let Ok(bytes) = response.bytes().await {
                if bytes.len() < 1024 {
                    return None;
                }
                if fs::write(&filepath, &bytes).await.is_ok() {
                    return Some(normalize_path(&filepath));
                }
            }
        }
        _ => {}
    }
    None
}

pub(crate) async fn extract_content_blocks(
    client: &Client,
    doc: &Html,
    page_url: &Url,
    output_dir: &str,
    max_images: usize,
) -> Vec<ContentBlock> {
    let content_root = doc
        .select(&SEL_MAIN)
        .next()
        .or_else(|| doc.select(&crate::selectors::SEL_BODY).next());
    let Some(content_root) = content_root else {
        return Vec::new();
    };

    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut seen_image_urls: HashSet<String> = HashSet::new();
    let mut images_kept: usize = 0;
    let cap_images = max_images > 0;

    // Containers we emit as a single ContentBlock — descendants must NOT be
    // re-extracted as paragraphs / headings / etc. or we'd double-count.
    let is_in_emitted_container = |el: &ElementRef| -> bool {
        el.ancestors()
            .skip(1)
            .filter_map(ElementRef::wrap)
            .any(|a| {
                matches!(
                    a.value().name(),
                    "blockquote" | "pre" | "dl" | "table" | "video" | "audio"
                )
            })
    };

    for node in content_root.descendants() {
        let Some(el) = ElementRef::wrap(node) else {
            continue;
        };
        if element_in_skip_zone(&el) {
            continue;
        }
        let tag = el.value().name();
        // Skip elements whose subtree is owned by an already-emitted block.
        // (e.g. <p> inside <blockquote>, <li> inside <dl>, <span> inside <pre>.)
        // Containers themselves still match on their own tag below.
        if !matches!(
            tag,
            "blockquote" | "pre" | "dl" | "table" | "video" | "audio"
        ) && is_in_emitted_container(&el)
        {
            continue;
        }

        if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            let level = heading_level_from_tag(tag);
            let text = element_text(&el);
            if !text.is_empty() {
                blocks.push(ContentBlock::Heading { level, text });
            }
        } else if tag == "p" {
            let text = element_text(&el);
            if !text.is_empty() && text.len() > 20 {
                blocks.push(ContentBlock::Paragraph { text });
            }
        } else if tag == "iframe" {
            let src_raw = el
                .value()
                .attr("src")
                .or_else(|| el.value().attr("data-src"))
                .unwrap_or("");
            if src_raw.is_empty() || src_raw.starts_with("about:") {
                continue;
            }
            let src = page_url
                .join(src_raw)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| src_raw.to_string());
            let title = el.value().attr("title").unwrap_or("").to_string();
            let provider = embed_provider_from_src(&src).to_string();
            blocks.push(ContentBlock::Embed {
                provider,
                src,
                title,
            });
        } else if tag == "svg" {
            let svg_outer = el.html();
            if svg_outer.len() < 50 || svg_outer.len() > 200_000 {
                continue;
            }
            let mut hasher = Sha256::new();
            hasher.update(svg_outer.as_bytes());
            let hash = format!("{:x}", hasher.finalize());
            let short = &hash[..16];
            let filename = format!("svg-{short}.svg");
            let filepath = format!("{output_dir}/{filename}");
            if !seen_image_urls.insert(format!("inline-svg://{short}")) {
                continue;
            }
            if cap_images && images_kept >= max_images {
                continue;
            }
            images_kept += 1;
            if !Path::new(&filepath).exists() {
                if let Err(e) = std::fs::write(&filepath, &svg_outer) {
                    tracing::warn!("Failed to save inline SVG to {filepath}: {e}");
                    continue;
                }
            }
            let mut alt = el.value().attr("aria-label").unwrap_or("").to_string();
            if alt.is_empty() {
                if let Ok(title_sel) = Selector::parse("title") {
                    if let Some(t) = el.select(&title_sel).next() {
                        alt = element_text(&t);
                    }
                }
            }
            blocks.push(ContentBlock::Image {
                original_url: format!("inline-svg://{short}"),
                local_path: normalize_path(&filepath),
                alt_text: alt,
            });
        } else if tag == "img" {
            // <picture><source srcset></picture> best candidate.
            let picture_source = el
                .parent()
                .and_then(ElementRef::wrap)
                .filter(|p| p.value().name() == "picture")
                .and_then(|picture| {
                    let mut best: Option<String> = None;
                    let mut best_w: u32 = 0;
                    for child in picture.children() {
                        if let Some(c_el) = ElementRef::wrap(child) {
                            if c_el.value().name() == "source" {
                                if let Some(ss) = c_el.value().attr("srcset") {
                                    for part in ss.split(',') {
                                        let bits: Vec<&str> = part.split_whitespace().collect();
                                        let url = bits.first().copied().unwrap_or("");
                                        let w: u32 = bits
                                            .get(1)
                                            .and_then(|s| s.trim_end_matches('w').parse().ok())
                                            .unwrap_or(0);
                                        if !url.is_empty() && (best.is_none() || w > best_w) {
                                            best = Some(url.to_string());
                                            best_w = w;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    best
                });

            let primary = el.value().attr("src").map(str::to_string);
            let data_src = el.value().attr("data-src").map(str::to_string);
            let srcset = el.value().attr("srcset").map(str::to_string);
            let mut alt = el.value().attr("alt").unwrap_or("").to_string();
            // Fall back to <figcaption> when alt is empty and the image
            // sits inside a <figure>. Most figures use caption-as-description.
            if alt.is_empty() {
                if let Some(fig) = el
                    .ancestors()
                    .filter_map(ElementRef::wrap)
                    .find(|a| a.value().name() == "figure")
                {
                    if let Some(cap) = fig.select(&SEL_FIGCAPTION).next() {
                        let cap_text = element_text(&cap);
                        if !cap_text.is_empty() {
                            alt = cap_text;
                        }
                    }
                }
            }

            let mut candidates: Vec<String> = Vec::new();
            candidates.extend(picture_source);
            candidates.extend(primary);
            candidates.extend(data_src);
            if let Some(srcset_str) = srcset {
                for part in srcset_str.split(',') {
                    if let Some(first) = part.split_whitespace().next() {
                        candidates.push(first.to_string());
                    }
                }
            }

            for src in candidates {
                if let Ok(abs) = page_url.join(&src) {
                    let url_str = abs.to_string();
                    if url_str.starts_with("data:")
                        || url_str.contains("1x1")
                        || url_str.contains("placeholder")
                    {
                        continue;
                    }
                    if !seen_image_urls.insert(url_str.clone()) {
                        break;
                    }
                    if cap_images && images_kept >= max_images {
                        break;
                    }
                    images_kept += 1;
                    blocks.push(ContentBlock::Image {
                        original_url: url_str,
                        local_path: String::new(),
                        alt_text: alt.clone(),
                    });
                    break;
                }
            }
        } else if matches!(tag, "ul" | "ol") {
            let parent_is_list = el
                .parent()
                .and_then(ElementRef::wrap)
                .is_some_and(|p| matches!(p.value().name(), "ul" | "ol" | "li"));
            if parent_is_list {
                continue;
            }
            let items: Vec<String> = el
                .select(&SEL_LI)
                .map(|li| element_text(&li))
                .filter(|s| !s.is_empty())
                .collect();
            if !items.is_empty() {
                blocks.push(ContentBlock::List { items });
            }
        } else if tag == "form" {
            let action_raw = el.value().attr("action").unwrap_or("");
            let action = if action_raw.is_empty() {
                String::new()
            } else {
                page_url
                    .join(action_raw)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| action_raw.to_string())
            };
            let method = el.value().attr("method").unwrap_or("get").to_uppercase();

            let mut fields = Vec::new();
            for input in el.select(&SEL_INPUT) {
                let field_type = input
                    .value()
                    .attr("type")
                    .unwrap_or(input.value().name())
                    .to_string();
                if matches!(field_type.as_str(), "hidden" | "submit" | "button") {
                    continue;
                }
                let name = input.value().attr("name").unwrap_or("").to_string();
                let placeholder = input.value().attr("placeholder").unwrap_or("").to_string();
                let required = input.value().attr("required").is_some();

                let mut label = String::new();
                if let Some(id) = input.value().attr("id") {
                    if let Ok(label_sel) = Selector::parse(&format!("label[for='{id}']")) {
                        if let Some(label_elem) = doc.select(&label_sel).next() {
                            label = element_text(&label_elem);
                        }
                    }
                }
                if label.is_empty() {
                    for anc in input.ancestors() {
                        if let Some(anc_el) = ElementRef::wrap(anc) {
                            if anc_el.value().name() == "label" {
                                label = element_text(&anc_el);
                                break;
                            }
                        }
                    }
                }

                let mut options = Vec::new();
                if input.value().name() == "select" {
                    for option in input.select(&SEL_OPTION) {
                        let opt_text = element_text(&option);
                        if !opt_text.is_empty() {
                            options.push(opt_text);
                        }
                    }
                }

                fields.push(FormField {
                    field_type,
                    name,
                    label,
                    placeholder,
                    required,
                    options,
                });
            }

            let mut submit_text = String::from("Submit");
            if let Some(submit_btn) = el.select(&SEL_SUBMIT).next() {
                if submit_btn.value().name() == "input" {
                    submit_text = submit_btn
                        .value()
                        .attr("value")
                        .unwrap_or("Submit")
                        .to_string();
                } else {
                    let t = element_text(&submit_btn);
                    if !t.is_empty() {
                        submit_text = t;
                    }
                }
            }

            let purpose = classify_form_purpose(&fields, &submit_text, &action);
            blocks.push(ContentBlock::Form {
                action,
                method,
                fields,
                submit_text,
                purpose,
            });
        } else if tag == "pre" {
            // Detect language from `<code class="language-rust">` or
            // `<pre class="hljs rust">` etc.
            let code_el = el.select(&SEL_CODE_INSIDE_PRE).next();
            let class_str = code_el
                .and_then(|c| c.value().attr("class"))
                .or_else(|| el.value().attr("class"))
                .unwrap_or("");
            let language = class_str
                .split_whitespace()
                .find_map(|c| {
                    c.strip_prefix("language-")
                        .or_else(|| c.strip_prefix("lang-"))
                })
                .map(|s| s.to_string())
                .or_else(|| {
                    // Fallback: well-known short language identifiers used
                    // by hljs / Prism without a `language-` prefix.
                    let known = [
                        "rust",
                        "javascript",
                        "typescript",
                        "python",
                        "go",
                        "java",
                        "kotlin",
                        "swift",
                        "ruby",
                        "php",
                        "csharp",
                        "cpp",
                        "c",
                        "html",
                        "css",
                        "json",
                        "yaml",
                        "toml",
                        "bash",
                        "shell",
                        "sh",
                        "sql",
                        "markdown",
                        "md",
                        "xml",
                    ];
                    class_str
                        .split_whitespace()
                        .find(|c| known.contains(&c.to_lowercase().as_str()))
                        .map(|s| s.to_lowercase())
                });
            let text: String = code_el
                .map(|c| c.text().collect::<String>())
                .unwrap_or_else(|| el.text().collect::<String>());
            // Skip empty / trivial blocks.
            if text.trim().len() >= 4 {
                blocks.push(ContentBlock::Code {
                    language,
                    text: text.trim_end().to_string(),
                });
            }
        } else if tag == "blockquote" {
            let text = element_text(&el);
            if text.len() >= 8 {
                let cite = el
                    .value()
                    .attr("cite")
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
                blocks.push(ContentBlock::Quote { text, cite });
            }
        } else if tag == "video" || tag == "audio" {
            let primary = el.value().attr("src").map(str::to_string);
            let from_source = el
                .select(&SEL_VIDEO_SOURCE)
                .next()
                .and_then(|s| s.value().attr("src"))
                .map(str::to_string);
            let Some(src_raw) = primary.or(from_source) else {
                continue;
            };
            let src = page_url
                .join(&src_raw)
                .map(|u| u.to_string())
                .unwrap_or(src_raw);
            let poster = el.value().attr("poster").and_then(|p| {
                page_url
                    .join(p)
                    .ok()
                    .map(|u| u.to_string())
                    .filter(|s| !s.is_empty())
            });
            let title = el.value().attr("title").unwrap_or("").to_string();
            blocks.push(ContentBlock::Media {
                kind: tag.to_string(),
                src,
                poster,
                title,
            });
        } else if tag == "dl" {
            // Pair each <dt> with the <dd> sibling(s) immediately after it.
            let mut items: Vec<DefinitionItem> = Vec::new();
            let mut current_term: Option<String> = None;
            for child in el.children().filter_map(ElementRef::wrap) {
                let cname = child.value().name();
                if cname == "dt" {
                    let t = element_text(&child);
                    if !t.is_empty() {
                        current_term = Some(t);
                    }
                } else if cname == "dd" {
                    let d = element_text(&child);
                    if let Some(term) = current_term.clone() {
                        if !d.is_empty() {
                            items.push(DefinitionItem {
                                term,
                                description: d,
                            });
                        }
                    }
                }
            }
            // Skip if the heuristic didn't match — fall back to a flat list.
            if items.is_empty() {
                let terms: Vec<String> = el.select(&SEL_DT).map(|e| element_text(&e)).collect();
                let descs: Vec<String> = el.select(&SEL_DD).map(|e| element_text(&e)).collect();
                let len = terms.len().min(descs.len());
                for i in 0..len {
                    if !terms[i].is_empty() && !descs[i].is_empty() {
                        items.push(DefinitionItem {
                            term: terms[i].clone(),
                            description: descs[i].clone(),
                        });
                    }
                }
            }
            if !items.is_empty() {
                blocks.push(ContentBlock::DefinitionList { items });
            }
        } else if tag == "table" {
            // Skip layout tables (no <th>, no data, deeply nested) and tables
            // we've already walked into via an outer table.
            let parent_is_table = el
                .ancestors()
                .skip(1)
                .filter_map(ElementRef::wrap)
                .any(|a| a.value().name() == "table");
            if parent_is_table {
                continue;
            }

            let caption: Option<String> = el
                .select(&SEL_CAPTION)
                .next()
                .map(|c| element_text(&c))
                .filter(|s| !s.is_empty());

            let mut headers: Vec<String> = Vec::new();
            let mut rows: Vec<Vec<String>> = Vec::new();
            for tr in el.select(&SEL_TR) {
                // If this <tr> is inside a nested <table>, skip — that
                // table will be emitted separately or filtered out.
                let inside_nested = tr
                    .ancestors()
                    .skip(1)
                    .filter_map(ElementRef::wrap)
                    .take_while(|a| a.value().name() != "table")
                    .any(|_| false)
                    || {
                        let mut depth = 0;
                        for a in tr.ancestors().filter_map(ElementRef::wrap) {
                            if a.value().name() == "table" {
                                depth += 1;
                            }
                        }
                        depth > 1
                    };
                if inside_nested {
                    continue;
                }
                let th_cells: Vec<String> = tr.select(&SEL_TH).map(|c| element_text(&c)).collect();
                if !th_cells.is_empty() && headers.is_empty() && rows.is_empty() {
                    headers = th_cells;
                    continue;
                }
                let td_cells: Vec<String> = tr.select(&SEL_TD).map(|c| element_text(&c)).collect();
                if !td_cells.is_empty() {
                    rows.push(td_cells);
                }
            }

            // Skip empty / single-cell layout tables; keep tables with real
            // tabular data (2+ rows OR 2+ columns).
            let has_data =
                rows.len() >= 2 || rows.iter().any(|r| r.len() >= 2) || !headers.is_empty();
            if has_data && !rows.is_empty() {
                blocks.push(ContentBlock::Table {
                    caption,
                    headers,
                    rows,
                });
            }
        }
    }

    let blocks = crate::util::dedup_adjacent_long_text(blocks);

    let mut download_futs = Vec::new();
    for (idx, block) in blocks.iter().enumerate() {
        if let ContentBlock::Image { original_url, .. } = block {
            if original_url.starts_with("inline-svg://") {
                continue;
            }
            let url = original_url.clone();
            let dir = output_dir.to_string();
            download_futs.push(async move { (idx, download_image(client, &url, &dir).await) });
        }
    }
    let download_results: Vec<(usize, Option<String>)> = future::join_all(download_futs).await;
    let mut idx_to_path: HashMap<usize, Option<String>> = download_results.into_iter().collect();

    let mut final_blocks = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.into_iter().enumerate() {
        match block {
            ContentBlock::Image {
                original_url,
                local_path,
                alt_text,
            } if original_url.starts_with("inline-svg://") => {
                final_blocks.push(ContentBlock::Image {
                    original_url,
                    local_path,
                    alt_text,
                });
            }
            ContentBlock::Image {
                original_url,
                alt_text,
                ..
            } => {
                if let Some(Some(path)) = idx_to_path.remove(&i) {
                    final_blocks.push(ContentBlock::Image {
                        original_url,
                        local_path: path,
                        alt_text,
                    });
                }
            }
            other => final_blocks.push(other),
        }
    }

    final_blocks
}
