use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use url::Url;

use crate::model::{
    AssetEntry, BrandPalette, ContactInfo, ContentBlock, FrameworkHint, HreflangGroup, PageData,
    PageSection, PageSummary, PageTemplate, ScrapedData, SiteData, SocialLink,
};
use crate::util::normalize_path;

pub(crate) fn categorize_page(url: &str, page: &PageData) -> String {
    let url_lc = url.to_lowercase();
    let path = Url::parse(url)
        .ok()
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let path_lc = path.to_lowercase();

    if path_lc == "/" || path_lc.is_empty() {
        return "home".to_string();
    }
    let title_lc = page.title.to_lowercase();
    let first_heading = page
        .content_blocks
        .iter()
        .find_map(|b| match b {
            ContentBlock::Heading { text, .. } => Some(text.to_lowercase()),
            _ => None,
        })
        .unwrap_or_default();

    let any = |needles: &[&str]| -> bool {
        needles.iter().any(|n| {
            path_lc.contains(n)
                || url_lc.contains(n)
                || title_lc.contains(n)
                || first_heading.contains(n)
        })
    };

    if any(&["/contact", "/kontakt", "kontakt", "contact"]) {
        return "contact".to_string();
    }
    if any(&["/about", "/o-nas", "/o-spolocnosti", "/about-us", "/team"]) {
        return "about".to_string();
    }
    if any(&[
        "/privacy",
        "/cookies",
        "/gdpr",
        "/terms",
        "/obchodni-podminky",
        "/zasady",
        "/legal",
        "/imprint",
    ]) {
        return "legal".to_string();
    }
    if path_lc == "/blog" || path_lc.ends_with("/blog/") || any(&["/news", "/aktuality"]) {
        return "blog-index".to_string();
    }
    if path_lc.starts_with("/blog/") && path_lc != "/blog/" {
        return "blog-post".to_string();
    }
    // Product / e-commerce — check BEFORE service so Shopify product pages
    // don't get bucketed into the generic services bucket. JSON-LD type
    // "Product" is the strongest signal; URL paths `/products/`, `/shop/`,
    // `/collections/`, `/store/` are next.
    let has_jsonld_product = page
        .structured_data
        .iter()
        .any(|v| {
            v.get("@type")
                .and_then(|t| t.as_str())
                .map(|s| s.eq_ignore_ascii_case("Product"))
                .unwrap_or(false)
        });
    if has_jsonld_product
        || any(&[
            "/products/",
            "/product/",
            "/shop/",
            "/store/",
            "/collections/",
            "/produkty/",
        ])
    {
        return "product".to_string();
    }
    if any(&["/service", "/sluzby", "/produkt"]) {
        return "service".to_string();
    }
    if any(&["/pricing", "/cenik", "/cenika"]) {
        return "pricing".to_string();
    }
    if any(&["/case-stud", "/pripadova", "/reference"]) {
        return "case-study".to_string();
    }
    "page".to_string()
}

/// Short text snippet representing a slice of content blocks — used in
/// PageSection.summary so the agent can read the index without opening
/// scraped.json.
fn summarize_section_blocks(blocks: &[ContentBlock]) -> String {
    for b in blocks {
        if let ContentBlock::Heading { text, .. } = b {
            return text.chars().take(80).collect();
        }
    }
    for b in blocks {
        if let ContentBlock::Paragraph { text } = b {
            return text.chars().take(80).collect();
        }
    }
    String::new()
}

/// Detect a "features grid" run starting at `from`: 3+ consecutive
/// (Heading + Paragraph) pairs at the same heading level.
fn detect_features_run(blocks: &[ContentBlock], from: usize) -> Option<usize> {
    let n = blocks.len();
    let mut i = from;
    let mut heading_level: Option<u8> = None;
    let mut pairs = 0;

    while i + 1 < n {
        if let ContentBlock::Heading { level, .. } = &blocks[i] {
            if let ContentBlock::Paragraph { .. } = &blocks[i + 1] {
                match heading_level {
                    None => heading_level = Some(*level),
                    Some(l) if l == *level => {}
                    _ => break,
                }
                pairs += 1;
                i += 2;
                continue;
            }
        }
        break;
    }
    if pairs >= 3 {
        Some(i)
    } else {
        None
    }
}

/// Detect a "team / portfolio grid": 3+ consecutive Image + short-Heading
/// pairs (typical team profile cards).
fn detect_team_run(blocks: &[ContentBlock], from: usize) -> Option<usize> {
    let n = blocks.len();
    let mut i = from;
    let mut pairs = 0;
    while i + 1 < n {
        let img_ok = matches!(&blocks[i], ContentBlock::Image { .. });
        let head_ok =
            matches!(&blocks[i + 1], ContentBlock::Heading { text, .. } if text.len() < 80);
        if img_ok && head_ok {
            pairs += 1;
            i += 2;
            continue;
        }
        break;
    }
    if pairs >= 3 {
        Some(i)
    } else {
        None
    }
}

/// Detect a "gallery": 3+ consecutive Image blocks with no text between.
fn detect_gallery_run(blocks: &[ContentBlock], from: usize) -> Option<usize> {
    let n = blocks.len();
    let mut i = from;
    let mut count = 0;
    while i < n && matches!(blocks[i], ContentBlock::Image { .. }) {
        count += 1;
        i += 1;
    }
    if count >= 3 {
        Some(i)
    } else {
        None
    }
}

/// **Removed.** Was producing too many false positives on doc / blog pages
/// (332 on htmx.org, 178 on daringfireball.net during QA). Replacement
/// approach is for the coding agent to infer testimonials from context;
/// reintroduce only if we add a higher-precision signal (e.g. detecting
/// `<blockquote>` or schema.org `Review` markup).
fn detect_testimonials_run(_blocks: &[ContentBlock], _from: usize) -> Option<usize> {
    None
}

/// Detect a "FAQ" run: 3+ (h3 OR h4 question) + paragraph pairs.
fn detect_faq_run(blocks: &[ContentBlock], from: usize) -> Option<usize> {
    let n = blocks.len();
    let mut i = from;
    let mut pairs = 0;
    while i + 1 < n {
        let q_ok = matches!(
            &blocks[i],
            ContentBlock::Heading { level, text }
                if (*level == 3 || *level == 4) && text.len() < 200
        );
        let a_ok = matches!(&blocks[i + 1], ContentBlock::Paragraph { .. });
        if q_ok && a_ok {
            pairs += 1;
            i += 2;
            continue;
        }
        break;
    }
    if pairs >= 3 {
        Some(i)
    } else {
        None
    }
}

/// True if a string looks like a price label (short text, currency symbol
/// or code, and digits — but NOT a sentence mentioning a price). Real
/// pricing tier labels are short headers like "$29 / mo" or "Pro — €99",
/// not "He paid $29 for the meal". The length cap is key.
fn looks_like_price(text: &str) -> bool {
    if text.len() > 40 {
        return false;
    }
    let has_currency = text.contains('$')
        || text.contains('€')
        || text.contains('£')
        || text.contains('¥')
        || text.to_lowercase().contains("kč")
        || text.to_lowercase().contains("eur")
        || text.to_lowercase().contains("usd")
        || text.to_lowercase().contains("czk")
        || text.to_lowercase().contains("pln")
        || text.to_lowercase().contains("/mo")
        || text.to_lowercase().contains("/month")
        || text.to_lowercase().contains("/year");
    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    has_currency && has_digit
}

/// Detect a "pricing grid": 3+ price-shaped strings inside a short window,
/// where each price is in a Heading (not buried in body paragraph text).
/// Prior version triggered on blog posts that happened to mention money;
/// this one only fires on structured tier rows.
fn detect_pricing_run(blocks: &[ContentBlock], from: usize) -> Option<usize> {
    let n = blocks.len();
    let mut hits = 0;
    let mut last_hit = from;
    let mut i = from;
    while i < n && (i - from) < 12 {
        let has_price = match &blocks[i] {
            // Only count prices that appear in headings — pricing tiers
            // are always labelled with structured headings, not buried
            // in body text. Paragraph mentions like "costs $29" don't count.
            ContentBlock::Heading { text, .. } => looks_like_price(text),
            _ => false,
        };
        if has_price {
            hits += 1;
            last_hit = i + 1;
        }
        i += 1;
    }
    if hits >= 3 {
        Some(last_hit)
    } else {
        None
    }
}

/// Per-page heuristic section inference. Returns a list of `{section_type,
/// block_start, block_end, summary}` spans over `page.content_blocks`. The
/// agent uses these to figure out "this is a hero, this is a features grid,
/// this is a CTA" rather than reading a flat block list.
pub(crate) fn detect_sections(blocks: &[ContentBlock]) -> Vec<PageSection> {
    let n = blocks.len();
    let mut sections = Vec::new();
    if n == 0 {
        return sections;
    }
    let mut i = 0;

    // Pass 1: hero — if the page starts with a heading, take up to 6 blocks
    // of (heading + paragraph + small heading + maybe one image).
    if matches!(blocks[0], ContentBlock::Heading { .. }) {
        let mut end = 1;
        let mut img_count = 0;
        while end < n.min(8) {
            match &blocks[end] {
                ContentBlock::Paragraph { .. } => end += 1,
                ContentBlock::Image { .. } if img_count == 0 => {
                    img_count += 1;
                    end += 1;
                }
                ContentBlock::Heading { level, .. } if *level >= 4 => end += 1,
                _ => break,
            }
        }
        if end >= 2 {
            sections.push(PageSection {
                section_type: "hero".to_string(),
                block_start: 0,
                block_end: end,
                summary: summarize_section_blocks(&blocks[..end]),
            });
            i = end;
        }
    }

    while i < n {
        // Form → CTA
        if matches!(blocks[i], ContentBlock::Form { .. }) {
            sections.push(PageSection {
                section_type: "cta".to_string(),
                block_start: i,
                block_end: i + 1,
                summary: "form".to_string(),
            });
            i += 1;
            continue;
        }
        // Embed → embed section (often a YouTube hero or testimonial video)
        if matches!(blocks[i], ContentBlock::Embed { .. }) {
            let provider = match &blocks[i] {
                ContentBlock::Embed { provider, .. } => provider.clone(),
                _ => "embed".to_string(),
            };
            sections.push(PageSection {
                section_type: "embed".to_string(),
                block_start: i,
                block_end: i + 1,
                summary: provider,
            });
            i += 1;
            continue;
        }
        // Pricing first because currency is the most specific signal.
        if let Some(end) = detect_pricing_run(blocks, i) {
            sections.push(PageSection {
                section_type: "pricing-grid".to_string(),
                block_start: i,
                block_end: end,
                summary: summarize_section_blocks(&blocks[i..end]),
            });
            i = end;
            continue;
        }
        if let Some(end) = detect_faq_run(blocks, i) {
            let count = (end - i) / 2;
            sections.push(PageSection {
                section_type: "faq".to_string(),
                block_start: i,
                block_end: end,
                summary: format!("{count} Q&A pairs"),
            });
            i = end;
            continue;
        }
        if let Some(end) = detect_features_run(blocks, i) {
            let count = (end - i) / 2;
            sections.push(PageSection {
                section_type: "features".to_string(),
                block_start: i,
                block_end: end,
                summary: format!("{count} feature items"),
            });
            i = end;
            continue;
        }
        if let Some(end) = detect_team_run(blocks, i) {
            let count = (end - i) / 2;
            sections.push(PageSection {
                section_type: "team".to_string(),
                block_start: i,
                block_end: end,
                summary: format!("{count} cards"),
            });
            i = end;
            continue;
        }
        if let Some(end) = detect_gallery_run(blocks, i) {
            let count = end - i;
            sections.push(PageSection {
                section_type: "gallery".to_string(),
                block_start: i,
                block_end: end,
                summary: format!("{count} images"),
            });
            i = end;
            continue;
        }
        if let Some(end) = detect_testimonials_run(blocks, i) {
            let count = end - i;
            sections.push(PageSection {
                section_type: "testimonials".to_string(),
                block_start: i,
                block_end: end,
                summary: format!("{count} quotes"),
            });
            i = end;
            continue;
        }
        // Default: a content run — grow until we hit a special pattern.
        let start = i;
        let mut end = i + 1;
        while end < n {
            if matches!(
                blocks[end],
                ContentBlock::Form { .. } | ContentBlock::Embed { .. }
            ) {
                break;
            }
            if detect_features_run(blocks, end).is_some()
                || detect_team_run(blocks, end).is_some()
                || detect_pricing_run(blocks, end).is_some()
                || detect_faq_run(blocks, end).is_some()
                || detect_gallery_run(blocks, end).is_some()
                || detect_testimonials_run(blocks, end).is_some()
            {
                break;
            }
            end += 1;
        }
        sections.push(PageSection {
            section_type: "content".to_string(),
            block_start: start,
            block_end: end,
            summary: summarize_section_blocks(&blocks[start..end]),
        });
        i = end;
    }

    sections
}

/// Per-page SEO / accessibility quality flags. Cheap heuristics — the agent
/// can decide whether to preserve or fix them.
pub(crate) fn detect_quality_flags(page: &PageData) -> Vec<String> {
    let mut flags = Vec::new();

    let has_h1 = page
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Heading { level: 1, .. }));
    if !has_h1 && !page.content_blocks.is_empty() {
        flags.push("no_h1".to_string());
    }
    let h1_count = page
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Heading { level: 1, .. }))
        .count();
    if h1_count > 1 {
        flags.push(format!("multiple_h1:{h1_count}"));
    }

    if page.meta_description.is_empty() {
        flags.push("no_meta_description".to_string());
    } else if page.meta_description.len() > 160 {
        flags.push("meta_description_too_long".to_string());
    } else if page.meta_description.len() < 50 {
        flags.push("meta_description_too_short".to_string());
    }

    if page.title.is_empty() || page.title == "No title" {
        flags.push("no_title".to_string());
    } else if page.title.len() > 70 {
        flags.push("title_too_long".to_string());
    }

    if page.canonical_url.is_none() {
        flags.push("no_canonical".to_string());
    }

    if let Some(robots) = &page.meta_robots {
        if robots.contains("noindex") || robots.contains("none") {
            flags.push("meta_robots_noindex".to_string());
        }
        if robots.contains("nofollow") {
            flags.push("meta_robots_nofollow".to_string());
        }
    }

    let images_no_alt = page
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image { alt_text, .. } if alt_text.is_empty()))
        .count();
    if images_no_alt > 0 {
        flags.push(format!("images_missing_alt:{images_no_alt}"));
    }

    // Low-quality alt text: present but useless ("image", filenames, etc.)
    let images_low_quality_alt = page
        .content_blocks
        .iter()
        .filter(|b| {
            if let ContentBlock::Image { alt_text, .. } = b {
                if alt_text.is_empty() {
                    return false;
                }
                let lc = alt_text.to_lowercase();
                let trimmed = lc.trim();
                let placeholder = matches!(
                    trimmed,
                    "image"
                        | "photo"
                        | "picture"
                        | "logo"
                        | "icon"
                        | "img"
                        | "title"
                        | "alt"
                        | "untitled"
                        | "thumbnail"
                        | "banner"
                );
                let filename = trimmed.contains('.')
                    && (trimmed.ends_with(".jpg")
                        || trimmed.ends_with(".jpeg")
                        || trimmed.ends_with(".png")
                        || trimmed.ends_with(".gif")
                        || trimmed.ends_with(".webp")
                        || trimmed.ends_with(".svg"));
                let numeric = trimmed
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '_' || c == '-');
                placeholder || filename || numeric
            } else {
                false
            }
        })
        .count();
    if images_low_quality_alt > 0 {
        flags.push(format!("images_low_quality_alt:{images_low_quality_alt}"));
    }

    if page.total_words < 100 && !page.content_blocks.is_empty() {
        flags.push("thin_content".to_string());
    }

    // Parked-domain detection: body is essentially nothing but an iframe
    // pointing at a parked-page provider (afternic, sedo, rapidresultsearch,
    // bodis, dan.com etc.) or any single iframe with zero text content. The
    // agent should be told this is a dead site instead of rebuilding it.
    if page.total_words == 0
        && page
            .content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Embed { provider, .. } if provider == "iframe"))
        && page
            .content_blocks
            .iter()
            .all(|b| matches!(b, ContentBlock::Embed { .. }))
    {
        flags.push("parked_domain".to_string());
    } else if let Some(host) = page
        .content_blocks
        .iter()
        .find_map(|b| match b {
            ContentBlock::Embed { src, .. } => url::Url::parse(src).ok().and_then(|u| u.host_str().map(str::to_lowercase)),
            _ => None,
        })
    {
        let parked_hosts = [
            "rapidresultsearch.com",
            "sedoparking.com",
            "afternic.com",
            "bodis.com",
            "parkingcrew.net",
            "dan.com",
            "godaddy.com/forsale",
        ];
        if parked_hosts.iter().any(|h| host.contains(h)) && page.total_words < 50 {
            flags.push("parked_domain".to_string());
        }
    }

    flags
}

/// Signature string for a page's content shape — used by template detection.
fn page_signature(page: &PageData) -> String {
    if page.content_blocks.is_empty() {
        return String::new();
    }
    page.content_blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Heading { level, .. } => format!("h{level}"),
            ContentBlock::Paragraph { .. } => "p".to_string(),
            ContentBlock::Image { .. } => "img".to_string(),
            ContentBlock::List { .. } => "ul".to_string(),
            ContentBlock::Form { .. } => "form".to_string(),
            ContentBlock::Embed { .. } => "embed".to_string(),
            ContentBlock::Table { .. } => "table".to_string(),
            ContentBlock::Code { .. } => "code".to_string(),
            ContentBlock::Quote { .. } => "quote".to_string(),
            ContentBlock::Media { kind, .. } => kind.clone(),
            ContentBlock::DefinitionList { .. } => "dl".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Group pages that share the same block-pattern signature into templates.
/// Returns only groups with 2+ members so the agent gets actionable
/// "rebuild one component for these N pages" guidance.
/// Cluster pages by `hreflang` so the agent can rebuild the locale switcher.
/// For each language declared on any page, record every URL that declares
/// that hreflang as an alternate.
pub(crate) fn build_hreflang_groups(pages: &[PageData]) -> Vec<HreflangGroup> {
    let mut by_lang: HashMap<String, HashSet<String>> = HashMap::new();
    for p in pages {
        for alt in &p.hreflang_alternates {
            by_lang
                .entry(alt.lang.clone())
                .or_default()
                .insert(alt.url.clone());
        }
    }
    let mut groups: Vec<HreflangGroup> = by_lang
        .into_iter()
        .map(|(lang, urls)| {
            let mut urls: Vec<String> = urls.into_iter().collect();
            urls.sort();
            HreflangGroup { lang, urls }
        })
        .collect();
    groups.sort_by(|a, b| a.lang.cmp(&b.lang));
    groups
}

pub(crate) fn detect_templates(pages: &[PageData]) -> Vec<PageTemplate> {
    let mut by_sig: HashMap<String, Vec<&PageData>> = HashMap::new();
    for p in pages {
        let sig = page_signature(p);
        if sig.is_empty() {
            continue;
        }
        by_sig.entry(sig).or_default().push(p);
    }
    let mut templates: Vec<PageTemplate> = by_sig
        .into_iter()
        .filter(|(_, ps)| ps.len() >= 2)
        .map(|(sig, ps)| {
            let pattern: Vec<String> = sig.split(',').map(|s| s.to_string()).collect();
            let mut hasher = Sha256::new();
            hasher.update(sig.as_bytes());
            let hash = format!("{:x}", hasher.finalize());
            let template_id = format!("tpl_{}", &hash[..8]);
            let mut page_urls: Vec<String> = ps.iter().map(|p| p.url.clone()).collect();
            page_urls.sort();
            let sample_page = page_urls[0].clone();
            PageTemplate {
                template_id,
                block_pattern: pattern,
                page_count: page_urls.len(),
                pages: page_urls,
                sample_page,
            }
        })
        .collect();
    templates.sort_by(|a, b| b.page_count.cmp(&a.page_count));
    templates
}

pub(crate) fn build_page_summary(page: &PageData) -> PageSummary {
    let has_form = page
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Form { .. }));
    let image_count = page
        .content_blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image { .. }))
        .count();
    let primary_heading = page.content_blocks.iter().find_map(|b| match b {
        ContentBlock::Heading { text, .. } => Some(text.clone()),
        _ => None,
    });
    PageSummary {
        url: page.url.clone(),
        title: page.title.clone(),
        meta_description: page.meta_description.clone(),
        category: categorize_page(&page.url, page),
        word_count: page.total_words,
        block_count: page.content_blocks.len(),
        image_count,
        has_form,
        primary_heading,
        file: None,
        markdown_file: None,
        screenshot_desktop: page.screenshot_desktop.clone(),
        screenshot_mobile: page.screenshot_mobile.clone(),
        internal_links_out: page.internal_links_out.len(),
        internal_links_in: 0,
    }
}

pub(crate) fn build_site_data(pages: &[PageData], base_url: &str) -> SiteData {
    let language = pages.iter().find_map(|p| p.language.clone());
    let favicon_url = pages.iter().find_map(|p| p.favicon_url.clone());
    let primary_nav = pages
        .iter()
        .find_map(|p| (!p.nav_links.is_empty()).then(|| p.nav_links.clone()))
        .unwrap_or_default();
    let footer_blocks = pages
        .iter()
        .find_map(|p| (!p.footer_blocks.is_empty()).then(|| p.footer_blocks.clone()))
        .unwrap_or_default();

    let mut sitemap: Vec<PageSummary> = pages.iter().map(build_page_summary).collect();

    // Inbound-link counts.
    let mut inbound: HashMap<String, usize> = HashMap::new();
    for p in pages {
        for target in &p.internal_links_out {
            *inbound.entry(target.clone()).or_default() += 1;
        }
    }
    for s in sitemap.iter_mut() {
        if let Some(c) = inbound.get(&s.url) {
            s.internal_links_in = *c;
        }
    }

    // Most-common logo URL across all pages.
    let mut logo_counts: HashMap<String, usize> = HashMap::new();
    for p in pages {
        if let Some(l) = &p.logo_url {
            if !l.starts_with("inline-svg://") {
                *logo_counts.entry(l.clone()).or_default() += 1;
            }
        }
    }
    let logo_url = logo_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(u, _)| u);

    let brand = BrandPalette {
        favicon_url,
        logo_url,
        ..Default::default()
    };

    SiteData {
        base_url: base_url.to_string(),
        language,
        frameworks: Vec::new(),
        primary_nav,
        footer_blocks,
        contact: ContactInfo::default(),
        brand,
        templates: Vec::new(),
        hreflang_groups: Vec::new(),
        sitemap,
        total_pages: pages.len(),
        assets: Vec::new(),
        error_pages: Vec::new(),
        output_files: Vec::new(),
        quality_warnings: Vec::new(),
        skipped_pages: Vec::new(),
    }
}

/// Detect bundle-level quality warnings (post-scrape diagnostics).
///
/// Currently checks:
///   - `spa_loading_shell` — ≥80% of pages share the same template AND
///     that template has fewer than 5 content blocks. This usually means
///     the JS-rendered site hadn't hydrated when Chrome snapshotted, so
///     every page looks like the loading skeleton. Brooklyn Brewery
///     Round I regression: 10 identical pages of `img,img,h1,p`.
pub(crate) fn detect_quality_warnings(
    pages: &[crate::model::PageData],
    templates: &[crate::model::PageTemplate],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if pages.len() >= 3 {
        let total = pages.len();
        // Compute mean words per page across all scraped pages. True SPA
        // loading shells extract ~0-30 words per page (just the skeleton
        // text). Real WordPress + Elementor templates with consistent
        // shapes typically have 100+ words per page.
        let total_words: usize = pages.iter().map(|p| p.total_words).sum();
        let mean_words = total_words / total.max(1);
        for tpl in templates {
            let share = tpl.page_count as f64 / total as f64;
            // SPA shell signature: lots of pages share the same tiny
            // template AND the average page is very thin on content.
            // The word-count condition prevents false positives on
            // page-builder sites (Elementor / Divi / Webflow) where
            // pages legitimately share a template shape but each one
            // has substantial unique content.
            if share >= 0.8 && tpl.block_pattern.len() < 5 && mean_words < 50 {
                warnings.push(format!(
                    "spa_loading_shell:{}_of_{}_pages_share_{}_block_template",
                    tpl.page_count,
                    total,
                    tpl.block_pattern.len()
                ));
                break;
            }
        }
    }
    warnings
}

pub(crate) fn aggregate_contact(pages: &[PageData]) -> ContactInfo {
    let mut emails: HashSet<String> = HashSet::new();
    let mut phones: HashSet<String> = HashSet::new();
    let mut socials: Vec<SocialLink> = Vec::new();
    let mut seen_social: HashSet<(String, String)> = HashSet::new();
    let mut addresses: HashSet<String> = HashSet::new();
    let mut org: Option<JsonValue> = None;

    for p in pages {
        let Some(c) = &p.page_contact else { continue };
        emails.extend(c.emails.iter().cloned());
        phones.extend(c.phones.iter().cloned());
        for s in &c.social_links {
            if seen_social.insert((s.platform.clone(), s.url.clone())) {
                socials.push(s.clone());
            }
        }
        addresses.extend(c.addresses.iter().cloned());
        if org.is_none() {
            if let Some(o) = &c.organization {
                org = Some(o.clone());
            }
        }
    }

    // Case-insensitive email dedup also at the aggregate level (covers
    // cases where per-page extraction yielded one case and a different
    // page yielded another).
    let emails = crate::contact::dedup_emails(emails);
    let phones = crate::contact::dedup_phones(phones.into_iter().collect());
    let mut addresses: Vec<String> = addresses.into_iter().collect();
    addresses.sort();

    // Contact-form endpoints. Footshop / Bohemia Bagel / many EU sites
    // prefer contact forms to `mailto:` for anti-spam reasons, leaving
    // emails/phones empty. We surface the `action` URLs of every form
    // classified as `contact` so the agent's rebuild can POST to the
    // same endpoint. Deduped by URL.
    let mut form_endpoints: HashSet<String> = HashSet::new();
    for p in pages {
        for b in &p.content_blocks {
            if let ContentBlock::Form { action, purpose, .. } = b {
                if purpose == "contact" && !action.is_empty() {
                    form_endpoints.insert(action.clone());
                }
            }
        }
    }
    let mut form_endpoints: Vec<String> = form_endpoints.into_iter().collect();
    form_endpoints.sort();

    ContactInfo {
        emails,
        phones,
        social_links: socials,
        addresses,
        organization: org,
        contact_form_endpoints: form_endpoints,
    }
}

/// Detect the underlying tech stack from page HTML signatures. We don't have
/// the raw HTML at this point, so we infer from the structured signals we
/// already captured (script blobs, meta generator, etc.). This is a stub
/// here — real detection is done in `detect_frameworks_from_html` below
/// before we drop the bodies.
#[allow(clippy::drop_non_drop)]
pub(crate) fn detect_frameworks_from_html(html: &str) -> Vec<FrameworkHint> {
    let mut hints: Vec<FrameworkHint> = Vec::new();
    let lc = html.to_lowercase();

    let mut record = |name: &str, conf: &str, evidence: Vec<String>| {
        hints.push(FrameworkHint {
            framework: name.to_string(),
            confidence: conf.to_string(),
            evidence,
        });
    };

    if lc.contains("__next_data__") || lc.contains("/_next/static/") {
        record(
            "Next.js",
            "high",
            vec!["__NEXT_DATA__ or /_next/static asset path".to_string()],
        );
    }
    if lc.contains("data-astro") || lc.contains("astro-island") {
        record(
            "Astro",
            "high",
            vec!["astro-island / data-astro".to_string()],
        );
    }
    if lc.contains("name=\"generator\" content=\"hugo") {
        record("Hugo", "high", vec!["meta generator=Hugo".to_string()]);
    }
    if lc.contains("name=\"generator\" content=\"gatsby") {
        record("Gatsby", "high", vec!["meta generator=Gatsby".to_string()]);
    }
    // WordPress detector — require multiple corroborating signals so we
    // don't false-positive on third-party widgets that happen to reference
    // `/wp-content/` (e.g. embedded WordPress.com images on a non-WP site).
    {
        let has_wp_path = lc.contains("/wp-content/") || lc.contains("/wp-includes/");
        let has_wp_api = lc.contains("/wp-json/");
        let has_wp_admin_link = lc.contains("wp-admin");
        let has_wp_generator = lc.contains("name=\"generator\" content=\"wordpress");
        let has_wp_emoji = lc.contains("wp-emoji");
        let strong_signals = [
            has_wp_api,
            has_wp_admin_link,
            has_wp_generator,
            has_wp_emoji,
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if has_wp_path && strong_signals >= 1 {
            let mut ev = vec!["/wp-content/ or /wp-includes/ asset path".to_string()];
            if has_wp_api {
                ev.push("/wp-json/ REST API endpoint".to_string());
            }
            if has_wp_admin_link {
                ev.push("wp-admin link".to_string());
            }
            if has_wp_generator {
                ev.push("meta generator=WordPress".to_string());
            }
            if has_wp_emoji {
                ev.push("wp-emoji.js".to_string());
            }
            if lc.contains("/wp-content/plugins/elementor") {
                ev.push("Elementor plugin asset".to_string());
            }
            record("WordPress", "high", ev);
        } else if has_wp_path {
            // wp-content path with no corroborating signal — likely a 3rd
            // party widget. Record at low confidence so it doesn't mislead.
            record(
                "WordPress",
                "low",
                vec!["/wp-content/ reference (possibly 3rd-party widget)".to_string()],
            );
        }
    }
    // Shopify — multiple possible signals because Shopify sites use varying
    // CDN aliases (cdn.shopify.com, cdn.shop). Match any one.
    if lc.contains("cdn.shopify.com")
        || lc.contains("shopify-section")
        || lc.contains("shopify.shop")
        || lc.contains("//cdn.shop/")
        || lc.contains("shopifycdn.com")
        || lc.contains("monorail-edge.shopifysvc.com")
        || lc.contains("name=\"shopify-")
    {
        record(
            "Shopify",
            "high",
            vec!["Shopify CDN / monorail / namespace".to_string()],
        );
    }
    if lc.contains("webflow.io") || lc.contains("data-wf-page") {
        record(
            "Webflow",
            "high",
            vec!["data-wf-page / webflow.io".to_string()],
        );
    }
    if lc.contains("squarespace.com") || lc.contains("static1.squarespace.com") {
        record(
            "Squarespace",
            "high",
            vec!["static1.squarespace.com asset".to_string()],
        );
    }
    // Vite-bundled SPA detector. Railway.app and many small-biz sites
    // ship Vite-built apps that use hash-suffixed asset names like
    // `/assets/main-CnJ5sBrs.js` and `/assets/globals-CVRqehSE.css`.
    // The 8-character base64-ish hash is Vite's default.
    {
        let vite_asset_re = regex::Regex::new(
            r#"/assets/[a-zA-Z][a-zA-Z0-9_-]*-[A-Za-z0-9_-]{6,10}\.(?:js|css)"#,
        );
        if let Ok(re) = vite_asset_re {
            if re.is_match(html) {
                record(
                    "Vite",
                    "medium",
                    vec!["/assets/<name>-<hash>.{js,css} bundle (Vite/Rollup)".to_string()],
                );
            }
        }
    }
    // Remix detector — common alternative React framework.
    if lc.contains("__remix_run_") || lc.contains("window.__remixcontext") {
        record(
            "Remix",
            "high",
            vec!["__remix_run_ / Remix context global".to_string()],
        );
    }
    // SvelteKit (in addition to the existing /_app/immutable/ check).
    if lc.contains("data-sveltekit") || lc.contains("window.__sveltekit") {
        record(
            "SvelteKit",
            "high",
            vec!["data-sveltekit-* attribute".to_string()],
        );
    }
    // Solid.js — `_$HY` hydration global.
    if lc.contains("window._$hy") || lc.contains("_$hy=") {
        record(
            "Solid.js",
            "high",
            vec!["_$HY hydration global".to_string()],
        );
    }
    // Qwik — `q:container` / `q:script` attributes.
    if lc.contains("q:container") || lc.contains("q:script") {
        record("Qwik", "high", vec!["q:container attribute".to_string()]);
    }
    if lc.contains("data-react") || lc.contains("react-dom") || lc.contains("__reactrootcontainer")
    {
        record(
            "React",
            "medium",
            vec!["data-react / react-dom signature".to_string()],
        );
    }
    if lc.contains("data-v-app") || lc.contains("__nuxt") {
        record(
            "Nuxt / Vue",
            "high",
            vec!["__nuxt / data-v-app".to_string()],
        );
    }
    if lc.contains("/_app/immutable/") {
        record(
            "SvelteKit",
            "high",
            vec!["/_app/immutable/ asset path".to_string()],
        );
    }
    // Tailwind heuristic: lots of utility-class names on the same element.
    // We check for a few telltale combos.
    if lc.contains("class=\"flex ") && lc.contains("text-") && lc.contains("bg-") {
        record(
            "Tailwind CSS",
            "medium",
            vec!["heavy utility-class usage (flex / text-* / bg-*)".to_string()],
        );
    }
    // Phoenix / LiveView — common on Plausible (Elixir SaaS), Fly.io, etc.
    if lc.contains("phx-track-static")
        || lc.contains("phx-mounted")
        || lc.contains("phx-window")
        || lc.contains("data-phx-")
    {
        record(
            "Phoenix LiveView",
            "high",
            vec!["phx-* attribute signature".to_string()],
        );
    }
    // Rails — Hotwire / Turbo signals.
    if lc.contains("turbo-cable-stream-source")
        || lc.contains("data-turbo-")
        || lc.contains("name=\"csrf-token\"") && lc.contains("rails")
    {
        record(
            "Ruby on Rails",
            "medium",
            vec!["data-turbo-* / csrf-token + Rails marker".to_string()],
        );
    }
    // Django — csrfmiddlewaretoken is the canonical signal.
    if lc.contains("csrfmiddlewaretoken") {
        record(
            "Django",
            "high",
            vec!["csrfmiddlewaretoken hidden input".to_string()],
        );
    }
    // Laravel — typical _token hidden input + sometimes the X-CSRF-TOKEN meta.
    if lc.contains("name=\"_token\"")
        && (lc.contains("laravel_session") || lc.contains("name=\"csrf-token\""))
    {
        record(
            "Laravel",
            "medium",
            vec!["_token + csrf-token meta signature".to_string()],
        );
    }
    // Jekyll — generator meta.
    if lc.contains("name=\"generator\" content=\"jekyll") {
        record("Jekyll", "high", vec!["meta generator=Jekyll".to_string()]);
    }
    // Eleventy / 11ty — generator meta or asset path.
    if lc.contains("name=\"generator\" content=\"eleventy")
        || lc.contains("name=\"generator\" content=\"11ty")
    {
        record("Eleventy", "high", vec!["meta generator=Eleventy".to_string()]);
    }
    // Drop the closure so we can borrow `hints` immutably for the catch-all.
    drop(record);
    // Generic meta-generator catch-all when none of the above matched.
    if hints.is_empty() {
        if let Some(start) = lc.find("name=\"generator\" content=\"") {
            let from = start + r#"name="generator" content=""#.len();
            if let Some(end_rel) = lc[from..].find('"') {
                let value = &lc[from..from + end_rel];
                if !value.is_empty() && value.len() < 60 {
                    hints.push(FrameworkHint {
                        framework: value.to_string(),
                        confidence: "medium".to_string(),
                        evidence: vec!["meta name=generator content=\"…\"".to_string()],
                    });
                }
            }
        }
    }
    hints
}

pub(crate) fn build_index_md(site: &SiteData, pages: &[PageData]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Site Export — {}\n\n", site.base_url));
    out.push_str("> Generated by [dump-it](https://github.com/lordvojta/dump-it). This bundle is intended to be passed to a coding agent (Claude Code, Cursor, etc.) to rebuild the site on a new stack.\n\n");

    // Cross-domain sitemap warning — input URL was example.com but the
    // sitemap points at a different host (acquisition / merger redirect).
    for w in &site.quality_warnings {
        if let Some(rest) = w.strip_prefix("cross_domain_sitemap:") {
            let parts: Vec<&str> = rest.splitn(2, "_urls_at_").collect();
            let pct = parts.first().unwrap_or(&"50%");
            let foreign = parts.get(1).copied().unwrap_or("");
            out.push_str("## ⚠️ Cross-domain sitemap\n\n");
            out.push_str(&format!(
                "{pct} of sitemap URLs point at `{foreign}` rather than `{}`. The bundle is named after the input URL, but the content was scraped from a different domain. Common cause: merger or acquisition redirect (e.g. `damejidlo.cz` → `foodora.cz`). Verify the agent rebuilds the intended site, not the destination of the redirect.\n\n",
                site.base_url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').trim_start_matches("www.")
            ));
            break;
        }
    }
    // SPA loading-shell warning — agent should NOT trust the page content
    // if Chrome captured the loading skeleton. Surfaced ABOVE the failure
    // banner because the bundle technically has pages but they're junk.
    for w in &site.quality_warnings {
        if let Some(rest) = w.strip_prefix("spa_loading_shell:") {
            // rest format: "10_of_10_pages_share_4_block_template"
            let parts: Vec<&str> = rest.split('_').collect();
            let summary = if parts.len() >= 7 {
                format!(
                    "{} of {} pages share a {}-block template",
                    parts[0], parts[2], parts[5]
                )
            } else {
                rest.replace('_', " ")
            };
            out.push_str("## ⚠️ SPA loading shell suspected\n\n");
            out.push_str(&format!(
                "{summary}. Chrome likely captured the page before JS hydration completed. **The content blocks in this bundle are almost certainly placeholder skeletons, not the real page bodies.**\n\n"
            ));
            out.push_str("Suggested fix: re-run with `--js-wait 5000` or `--js-wait-selector \"main:not(:empty)\"`. If the site uses Cloudflare Turnstile / similar challenges, no headless workaround will succeed.\n\n");
            break;
        }
    }
    // Partial-scrape warning — when most pages were skipped by bot
    // protection or render failure, surface the skip rate prominently.
    if !site.skipped_pages.is_empty() {
        let total = site.total_pages + site.skipped_pages.len();
        let pct = (site.skipped_pages.len() as f64 / total as f64 * 100.0).round() as u32;
        if pct >= 50 {
            let bot_count = site
                .skipped_pages
                .iter()
                .filter(|s| s.reason == "bot_protected")
                .count();
            let render_count = site
                .skipped_pages
                .iter()
                .filter(|s| s.reason == "render_failed")
                .count();
            out.push_str(&format!(
                "## ⚠️ Partial scrape — {}/{} pages ({}%) blocked or unrenderable\n\n",
                site.skipped_pages.len(),
                total,
                pct
            ));
            out.push_str(&format!(
                "Skipped reasons: bot-protected = {bot_count}, render-failed = {render_count}. **The bundle is incomplete — verify the agent isn't rebuilding a partial site.** See `site.json:skipped_pages` for the full list.\n\n",
            ));
        }
    }
    // Failure banner — when no pages were scraped, the bundle is essentially
    // empty and the agent will waste context trying to make sense of it.
    // Surface the likely cause at the very top so the user sees it first.
    if site.total_pages == 0 {
        out.push_str("## ❌ Scrape failed — empty bundle\n\n");
        out.push_str(
            "**No pages were successfully scraped.** Likely causes:\n\n\
             - The site is behind a bot-protection / WAF challenge (Cloudflare, PerimeterX, Akamai) that blocked headless Chrome.\n\
             - The hostname did not resolve (DNS error) or the server refused the connection.\n\
             - The site requires JS that didn't render in time — try a longer `--js-wait` or a `--js-wait-selector`.\n\
             - Robots.txt disallowed every URL (pass `--ignore-robots` to override).\n\
             - The sitemap returned no usable HTML URLs (e.g. only `.xml` / `.txt` sub-resources).\n\n\
             Re-run with `--verbose` to see the per-page render errors.\n\n",
        );
    }

    out.push_str("## At a glance\n\n");
    out.push_str(&format!("- **Base URL**: `{}`\n", site.base_url));
    out.push_str(&format!("- **Pages**: {}\n", site.total_pages));
    if let Some(lang) = &site.language {
        out.push_str(&format!("- **Language**: `{lang}`\n"));
    }
    if !site.frameworks.is_empty() {
        let names: Vec<String> = site
            .frameworks
            .iter()
            .map(|f| format!("{} ({})", f.framework, f.confidence))
            .collect();
        out.push_str(&format!("- **Detected stack**: {}\n", names.join(", ")));
    }
    if let Some(fav) = &site.brand.favicon_local_path {
        out.push_str(&format!("- **Favicon**: `{fav}`\n"));
    } else if let Some(fav) = &site.brand.favicon_url {
        out.push_str(&format!("- **Favicon (remote)**: {fav}\n"));
    }
    if let Some(logo) = &site.brand.logo_local_path {
        out.push_str(&format!("- **Logo**: `{logo}`\n"));
    }
    if !site.brand.colors.is_empty() {
        let top: Vec<String> = site
            .brand
            .colors
            .iter()
            .take(5)
            .map(|c| format!("`{}` ({}×)", c.value, c.count))
            .collect();
        out.push_str(&format!("- **Top colors**: {}\n", top.join(", ")));
    }
    // Brand confidence — when low or medium, surface so the agent knows
    // the palette is a hint, not gospel. Common on Next.js / Tailwind
    // CSS-in-JS sites where styling lives in runtime-generated CSS.
    if let Some(conf) = &site.brand.confidence {
        if conf == "low" || conf == "medium" {
            out.push_str(&format!(
                "- **Brand confidence**: `{conf}` ⚠️ — palette is thin (likely CSS-in-JS / runtime styles). Verify colors and fonts against screenshots or the live site.\n"
            ));
        }
    }
    if !site.brand.fonts.is_empty() {
        let top: Vec<String> = site
            .brand
            .fonts
            .iter()
            .take(5)
            .map(|f| format!("`{}` ({}×)", f.family, f.count))
            .collect();
        out.push_str(&format!("- **Top fonts**: {}\n", top.join(", ")));
    }
    if !site.contact.emails.is_empty() {
        out.push_str(&format!(
            "- **Emails found**: {}\n",
            site.contact.emails.join(", ")
        ));
    }
    if !site.contact.phones.is_empty() {
        out.push_str(&format!(
            "- **Phones found**: {}\n",
            site.contact.phones.join(", ")
        ));
    }
    if !site.contact.social_links.is_empty() {
        let socials: Vec<String> = site
            .contact
            .social_links
            .iter()
            .map(|s| format!("{}: {}", s.platform, s.url))
            .collect();
        out.push_str(&format!("- **Social**: {}\n", socials.join(" · ")));
    }
    // Contact-form endpoints. Surfaced when the site has no mailto: /
    // tel: links — common on EU sites that prefer forms for anti-spam.
    // Agent can wire its rebuild's contact form to POST at the same URL.
    if !site.contact.contact_form_endpoints.is_empty()
        && site.contact.emails.is_empty()
        && site.contact.phones.is_empty()
    {
        out.push_str(&format!(
            "- **Contact form endpoint(s)**: {} _(no `mailto:` / `tel:` found; agent's rebuilt form should POST here)_\n",
            site.contact
                .contact_form_endpoints
                .iter()
                .map(|u| format!("`{u}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push('\n');

    if !site.templates.is_empty() {
        out.push_str("## Page templates\n\n");
        out.push_str("Pages that share the same content shape — rebuild as one component, then bind each page's content:\n\n");
        out.push_str("| Template | Pages | Sample | Block pattern |\n");
        out.push_str("|----------|-------|--------|---------------|\n");
        for t in &site.templates {
            out.push_str(&format!(
                "| `{}` | {} | {} | `{}` |\n",
                t.template_id,
                t.page_count,
                t.sample_page,
                t.block_pattern.join(",")
            ));
        }
        out.push('\n');
    }

    // Quality-flag roll-up across all pages.
    let mut flag_counts: HashMap<&str, usize> = HashMap::new();
    for p in pages {
        for f in &p.quality_flags {
            // Truncate the value-bearing flags so the same family rolls up.
            let key = f.split(':').next().unwrap_or(f.as_str());
            *flag_counts.entry(key).or_default() += 1;
        }
    }
    if !flag_counts.is_empty() {
        let mut rows: Vec<(&&str, &usize)> = flag_counts.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        out.push_str("## Quality flags\n\n");
        out.push_str("Pages affected by each SEO / accessibility issue (inspect `quality_flags` on each page in `scraped.json` for the per-page breakdown):\n\n");
        out.push_str("| Flag | Page count |\n|------|------------|\n");
        for (k, v) in rows {
            out.push_str(&format!("| `{k}` | {v} |\n"));
        }
        out.push('\n');
    }

    out.push_str("## Where to look\n\n");
    out.push_str("| File | What's in it |\n");
    out.push_str("|------|--------------|\n");
    out.push_str("| `scraped.json` | Master file. Every page with full content blocks, forms, structured data. Start here. |\n");
    out.push_str("| `site.json` | Site-wide aggregated data: nav, footer, contact, brand, sitemap, frameworks, assets, link graph |\n");
    out.push_str(
        "| `compact.json` | Stripped-down version of scraped.json for tight context windows |\n",
    );
    out.push_str("| `contact.json` | Extracted emails, phones, socials, addresses |\n");
    out.push_str(
        "| `brand.json` | Favicon, logo, color palette, fonts, CSS variables, webfont URLs |\n",
    );
    out.push_str("| `index.md` | This file |\n");
    out.push_str(
        "| `images/` | All downloaded assets (favicon, logo, content images, inline SVGs) |\n",
    );
    if site.output_files.iter().any(|f| f.starts_with("pages/")) {
        out.push_str("| `pages/<slug>.json` | One file per page (so the agent can `cat` them individually) |\n");
    }
    if site.output_files.iter().any(|f| f.starts_with("markdown/")) {
        out.push_str("| `markdown/<slug>.md` | Markdown rendering of each page (LLM-friendly) |\n");
    }
    if site
        .output_files
        .iter()
        .any(|f| f.starts_with("screenshots/"))
    {
        out.push_str("| `screenshots/<slug>.{desktop,mobile}.png` | Visual reference per page |\n");
    }
    out.push('\n');

    out.push_str("## Pages\n\n");
    out.push_str("| URL | Category | Title | Words | Form | Images | Links→ | →Links |\n");
    out.push_str("|-----|----------|-------|-------|------|--------|--------|--------|\n");
    for p in &site.sitemap {
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} | {} |\n",
            p.url,
            p.category,
            p.title.replace('|', "\\|"),
            p.word_count,
            if p.has_form { "yes" } else { "—" },
            p.image_count,
            p.internal_links_out,
            p.internal_links_in,
        ));
    }
    out.push('\n');

    out.push_str("## How to use this with a coding agent\n\n");
    out.push_str("1. Open this folder in your agent's working directory.\n");
    out.push_str(
        "2. Tell the agent the target framework (Next.js, Astro, Hugo, plain HTML, etc.).\n",
    );
    out.push_str("3. Have it read `index.md` first, then `site.json` for the chrome, then `scraped.json` (or `compact.json` if context is tight) for content.\n");
    out.push_str("4. For binary assets, reference `images/<filename>`. Inline-SVGs land as `images/svg-<hash>.svg`.\n");
    out.push_str("5. If `screenshots/` is present, use them as visual ground-truth for layout.\n");
    out
}

/// Convert a page's content_blocks to Markdown.
pub(crate) fn page_to_markdown(page: &PageData) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", page.title));
    out.push_str(&format!("<!-- url: {} -->\n", page.url));
    if let Some(canon) = &page.canonical_url {
        out.push_str(&format!("<!-- canonical: {canon} -->\n"));
    }
    if let Some(lang) = &page.language {
        out.push_str(&format!("<!-- lang: {lang} -->\n"));
    }
    if !page.meta_description.is_empty() {
        out.push_str(&format!(
            "<!-- description: {} -->\n",
            page.meta_description
        ));
    }
    out.push('\n');

    for block in &page.content_blocks {
        match block {
            ContentBlock::Heading { level, text } => {
                let hashes: String = (0..*level).map(|_| '#').collect();
                out.push_str(&format!("{hashes} {text}\n\n"));
            }
            ContentBlock::Paragraph { text } => {
                out.push_str(text);
                out.push_str("\n\n");
            }
            ContentBlock::List { items } => {
                for item in items {
                    out.push_str(&format!("- {item}\n"));
                }
                out.push('\n');
            }
            ContentBlock::Image {
                local_path,
                alt_text,
                original_url,
            } => {
                let alt = if alt_text.is_empty() {
                    "image"
                } else {
                    alt_text
                };
                let target = if local_path.is_empty() {
                    original_url.as_str()
                } else {
                    local_path.as_str()
                };
                out.push_str(&format!("![{alt}]({target})\n\n"));
            }
            ContentBlock::Form {
                action,
                method,
                fields,
                submit_text,
                purpose,
            } => {
                out.push_str(&format!(
                    "> **Form** ({purpose}) — {method} `{}`\n>\n",
                    if action.is_empty() {
                        "(no action)"
                    } else {
                        action.as_str()
                    }
                ));
                for f in fields {
                    out.push_str(&format!(
                        "> - `{}` ({}{}): {}\n",
                        f.name,
                        f.field_type,
                        if f.required { ", required" } else { "" },
                        if f.label.is_empty() {
                            f.placeholder.as_str()
                        } else {
                            f.label.as_str()
                        },
                    ));
                }
                out.push_str(&format!("> - submit: **{submit_text}**\n\n"));
            }
            ContentBlock::Embed {
                provider,
                src,
                title,
            } => {
                out.push_str(&format!(
                    "> **Embed** ({}): [{}]({})\n\n",
                    provider,
                    if title.is_empty() {
                        provider.as_str()
                    } else {
                        title.as_str()
                    },
                    src
                ));
            }
            ContentBlock::Table {
                caption,
                headers,
                rows,
            } => {
                if let Some(c) = caption {
                    out.push_str(&format!("**{c}**\n\n"));
                }
                if !headers.is_empty() {
                    out.push_str(&format!("| {} |\n", headers.join(" | ")));
                    out.push_str(&format!(
                        "|{}|\n",
                        headers.iter().map(|_| "---").collect::<Vec<_>>().join("|")
                    ));
                }
                for row in rows {
                    let escaped: Vec<String> = row.iter().map(|c| c.replace('|', "\\|")).collect();
                    out.push_str(&format!("| {} |\n", escaped.join(" | ")));
                }
                out.push('\n');
            }
            ContentBlock::Code { language, text } => {
                let lang = language.as_deref().unwrap_or("");
                out.push_str(&format!("```{lang}\n{text}\n```\n\n"));
            }
            ContentBlock::Quote { text, cite } => {
                for line in text.lines() {
                    out.push_str(&format!("> {line}\n"));
                }
                if let Some(c) = cite {
                    out.push_str(&format!("> — <{c}>\n"));
                }
                out.push('\n');
            }
            ContentBlock::Media {
                kind,
                src,
                poster,
                title,
            } => {
                let label = if title.is_empty() {
                    kind.as_str()
                } else {
                    title.as_str()
                };
                out.push_str(&format!("> **{kind}**: [{label}]({src})\n"));
                if let Some(p) = poster {
                    out.push_str(&format!("> poster: {p}\n"));
                }
                out.push('\n');
            }
            ContentBlock::DefinitionList { items } => {
                for item in items {
                    out.push_str(&format!("**{}**\n: {}\n\n", item.term, item.description));
                }
            }
        }
    }
    out
}

/// Compact summary suitable for fitting in a constrained LLM context window.
/// Drops content_blocks bodies, structured_data, and style_text; keeps URLs,
/// titles, summaries, and the site/contact/brand aggregates.
#[derive(serde::Serialize)]
pub(crate) struct CompactDump<'a> {
    pub base_url: &'a str,
    pub language: Option<&'a String>,
    pub total_pages: usize,
    pub frameworks: &'a [FrameworkHint],
    pub brand: &'a BrandPalette,
    pub contact: &'a ContactInfo,
    pub primary_nav: &'a [crate::model::NavLink],
    pub footer_blocks: &'a [ContentBlock],
    pub pages: Vec<CompactPage<'a>>,
}

#[derive(serde::Serialize)]
pub(crate) struct CompactPage<'a> {
    pub url: &'a str,
    pub title: &'a str,
    pub category: String,
    pub meta_description: &'a str,
    pub primary_heading: Option<String>,
    pub word_count: usize,
    pub image_count: usize,
    pub has_form: bool,
    pub internal_links_out: usize,
    pub headings: Vec<&'a str>,
}

/// JSON Schema describing the scraped.json + site.json shape. Hand-written;
/// concise enough to fit in any LLM context window so an agent can validate
/// the bundle against it before consuming.
pub(crate) fn build_schema_json() -> JsonValue {
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/lordvojta/dump-it/schema.json",
        "title": "dump-it bundle",
        "description": "Schema for the multi-file output produced by dump-it (scraped.json + site.json + auxiliaries).",
        "$defs": {
            "ContentBlock": {
                "oneOf": [
                    { "type": "object", "properties": { "type": {"const": "heading"}, "level": {"type": "integer", "minimum": 1, "maximum": 6}, "text": {"type": "string"} }, "required": ["type", "level", "text"] },
                    { "type": "object", "properties": { "type": {"const": "paragraph"}, "text": {"type": "string"} }, "required": ["type", "text"] },
                    { "type": "object", "properties": { "type": {"const": "list"}, "items": {"type": "array", "items": {"type": "string"}} }, "required": ["type", "items"] },
                    { "type": "object", "properties": { "type": {"const": "image"}, "original_url": {"type": "string"}, "local_path": {"type": "string"}, "alt_text": {"type": "string"} }, "required": ["type", "original_url", "local_path", "alt_text"] },
                    { "type": "object", "properties": { "type": {"const": "form"}, "action": {"type": "string"}, "method": {"type": "string"}, "fields": {"type": "array"}, "submit_text": {"type": "string"}, "purpose": {"type": "string", "enum": ["contact", "newsletter", "search", "login", "signup", "payment", "comment", "generic"]} }, "required": ["type", "action", "method", "fields", "submit_text"] },
                    { "type": "object", "properties": { "type": {"const": "embed"}, "provider": {"type": "string"}, "src": {"type": "string"}, "title": {"type": "string"} }, "required": ["type", "provider", "src", "title"] }
                ]
            },
            "PageSection": {
                "type": "object",
                "properties": {
                    "section_type": {"type": "string", "enum": ["hero", "features", "team", "gallery", "testimonials", "faq", "pricing-grid", "cta", "embed", "content"]},
                    "block_start": {"type": "integer"},
                    "block_end": {"type": "integer"},
                    "summary": {"type": "string"}
                },
                "required": ["section_type", "block_start", "block_end", "summary"]
            },
            "PageData": {
                "type": "object",
                "properties": {
                    "url": {"type": "string"},
                    "title": {"type": "string"},
                    "meta_title": {"type": "string"},
                    "meta_description": {"type": "string"},
                    "canonical_url": {"type": ["string", "null"]},
                    "language": {"type": ["string", "null"]},
                    "favicon_url": {"type": ["string", "null"]},
                    "logo_url": {"type": ["string", "null"]},
                    "og_image_url": {"type": ["string", "null"]},
                    "og_image_local_path": {"type": ["string", "null"]},
                    "twitter_card": {"type": ["string", "null"]},
                    "meta_robots": {"type": ["string", "null"]},
                    "hreflang_alternates": {"type": "array", "items": {"type": "object", "properties": {"lang": {"type": "string"}, "url": {"type": "string"}}}},
                    "nav_links": {"type": "array", "items": {"type": "object", "properties": {"text": {"type": "string"}, "href": {"type": "string"}}}},
                    "footer_blocks": {"type": "array", "items": {"$ref": "#/$defs/ContentBlock"}},
                    "structured_data": {"type": "array"},
                    "content_blocks": {"type": "array", "items": {"$ref": "#/$defs/ContentBlock"}},
                    "plain_text": {"type": "string"},
                    "page_assets": {"type": "array", "items": {"type": "string"}},
                    "sections": {"type": "array", "items": {"$ref": "#/$defs/PageSection"}},
                    "quality_flags": {"type": "array", "items": {"type": "string"}},
                    "total_words": {"type": "integer"},
                    "page_contact": {"type": ["object", "null"]},
                    "internal_links_out": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["url", "title", "content_blocks", "total_words"]
            }
        },
        "type": "object",
        "properties": {
            "scraped.json": {
                "type": "object",
                "properties": {
                    "total_pages": {"type": "integer"},
                    "pages": {"type": "array", "items": {"$ref": "#/$defs/PageData"}}
                },
                "required": ["total_pages", "pages"]
            },
            "site.json": {
                "type": "object",
                "properties": {
                    "base_url": {"type": "string"},
                    "language": {"type": ["string", "null"]},
                    "frameworks": {"type": "array"},
                    "primary_nav": {"type": "array"},
                    "footer_blocks": {"type": "array"},
                    "contact": {"type": "object"},
                    "brand": {"type": "object"},
                    "templates": {"type": "array"},
                    "hreflang_groups": {"type": "array"},
                    "sitemap": {"type": "array"},
                    "total_pages": {"type": "integer"},
                    "assets": {"type": "array"},
                    "error_pages": {"type": "array"},
                    "output_files": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["base_url", "total_pages"]
            }
        }
    })
}

pub(crate) fn build_compact<'a>(site: &'a SiteData, scraped: &'a ScrapedData) -> CompactDump<'a> {
    let pages = scraped
        .pages
        .iter()
        .map(|p| {
            let summary = build_page_summary(p);
            let headings: Vec<&str> = p
                .content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Heading { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .take(20)
                .collect();
            CompactPage {
                url: &p.url,
                title: &p.title,
                category: summary.category,
                meta_description: &p.meta_description,
                primary_heading: summary.primary_heading,
                word_count: p.total_words,
                image_count: summary.image_count,
                has_form: summary.has_form,
                internal_links_out: p.internal_links_out.len(),
                headings,
            }
        })
        .collect();
    CompactDump {
        base_url: &site.base_url,
        language: site.language.as_ref(),
        total_pages: site.total_pages,
        frameworks: &site.frameworks,
        brand: &site.brand,
        contact: &site.contact,
        primary_nav: &site.primary_nav,
        footer_blocks: &site.footer_blocks,
        pages,
    }
}

/// Walk the `images/`, `screenshots/`, `pages/`, `markdown/` directories and
/// build a flat manifest of every file produced.
pub(crate) fn build_asset_manifest(output_dir: &Path) -> Vec<AssetEntry> {
    let mut out = Vec::new();
    let candidates: &[(&str, &str)] = &[
        ("images", "image"),
        ("screenshots", "screenshot"),
        ("pages", "page-json"),
        ("markdown", "page-md"),
        ("css", "stylesheet"),
    ];
    for (dir, kind_default) in candidates {
        let d: PathBuf = output_dir.join(dir);
        if !d.exists() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let kind = if *dir == "images" {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("favicon") {
                    "favicon"
                } else if name.starts_with("logo") {
                    "logo"
                } else if name.starts_with("svg-") {
                    "svg"
                } else {
                    "image"
                }
            } else {
                *kind_default
            };
            out.push(AssetEntry {
                path: normalize_path(&p.to_string_lossy()),
                size_bytes: size,
                kind: kind.to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentBlock, PageData};

    fn page(url: &str, title: &str, blocks: Vec<ContentBlock>) -> PageData {
        PageData {
            url: url.to_string(),
            title: title.to_string(),
            meta_title: title.to_string(),
            meta_description: String::new(),
            canonical_url: None,
            language: None,
            favicon_url: None,
            logo_url: None,
            og_image_url: None,
            og_image_local_path: None,
            twitter_card: None,
            meta_robots: None,
            hreflang_alternates: vec![],
            nav_links: vec![],
            footer_blocks: vec![],
            structured_data: vec![],
            content_blocks: blocks,
            plain_text: String::new(),
            content_hash: String::new(),
            token_estimate: 0,
            summary: String::new(),
            page_assets: vec![],
            sections: vec![],
            quality_flags: vec![],
            total_words: 0,
            page_contact: None,
            internal_links_out: vec![],
            style_text: String::new(),
            stylesheet_urls: vec![],
            screenshot_desktop: None,
            screenshot_mobile: None,
        }
    }

    fn h(level: u8, text: &str) -> ContentBlock {
        ContentBlock::Heading {
            level,
            text: text.to_string(),
        }
    }
    fn p(text: &str) -> ContentBlock {
        ContentBlock::Paragraph {
            text: text.to_string(),
        }
    }

    #[test]
    fn categorize_page_routes_by_url_and_heading() {
        let home = page("https://x.com/", "Home", vec![]);
        assert_eq!(categorize_page(&home.url, &home), "home");
        let contact = page("https://x.com/kontakt", "Kontakt", vec![]);
        assert_eq!(categorize_page(&contact.url, &contact), "contact");
        let legal = page("https://x.com/gdpr", "GDPR", vec![]);
        assert_eq!(categorize_page(&legal.url, &legal), "legal");
        let blog_index = page("https://x.com/blog", "Blog", vec![]);
        assert_eq!(categorize_page(&blog_index.url, &blog_index), "blog-index");
        let blog_post = page("https://x.com/blog/foo", "Foo", vec![]);
        assert_eq!(categorize_page(&blog_post.url, &blog_post), "blog-post");
        // Regression: Shopify product pages used to fall through to `service`.
        let product = page(
            "https://levainbakery.com/products/oatmeal-raisin",
            "Oatmeal Raisin",
            vec![],
        );
        assert_eq!(categorize_page(&product.url, &product), "product");
        let collection = page("https://x.com/collections/all", "All Items", vec![]);
        assert_eq!(categorize_page(&collection.url, &collection), "product");
    }

    #[test]
    fn detect_sections_identifies_hero_features_faq() {
        let blocks = vec![
            h(1, "Big Hero"),
            p("Welcome to the page"),
            h(2, "Feature A"),
            p("Details A"),
            h(2, "Feature B"),
            p("Details B"),
            h(2, "Feature C"),
            p("Details C"),
            h(3, "Q1?"),
            p("A1"),
            h(3, "Q2?"),
            p("A2"),
            h(3, "Q3?"),
            p("A3"),
        ];
        let sections = detect_sections(&blocks);
        let types: Vec<&str> = sections.iter().map(|s| s.section_type.as_str()).collect();
        assert!(types.contains(&"hero"), "missing hero: {types:?}");
        assert!(types.contains(&"features"), "missing features: {types:?}");
        assert!(types.contains(&"faq"), "missing faq: {types:?}");
    }

    #[test]
    fn detect_quality_flags_flags_thin_no_h1_no_canonical() {
        let mut pg = page("https://x.com/foo", "Foo", vec![p("Just a tiny page")]);
        pg.total_words = 4;
        let flags = detect_quality_flags(&pg);
        assert!(flags.contains(&"no_h1".to_string()));
        assert!(flags.contains(&"no_canonical".to_string()));
        assert!(flags.contains(&"thin_content".to_string()));
        assert!(flags.contains(&"no_meta_description".to_string()));
    }

    #[test]
    fn detect_quality_warnings_flags_spa_loading_shell() {
        // Brooklyn Brewery regression: 10 pages all sharing 4-block
        // template `img,img,h1,p` is the SPA loading skeleton.
        let shell_blocks = || {
            vec![
                ContentBlock::Image {
                    original_url: "logo".to_string(),
                    local_path: "img/logo.png".to_string(),
                    alt_text: String::new(),
                },
                ContentBlock::Image {
                    original_url: "hero".to_string(),
                    local_path: "img/hero.png".to_string(),
                    alt_text: String::new(),
                },
                h(1, "Loading…"),
                p("Please wait"),
            ]
        };
        let pages: Vec<PageData> = (0..10)
            .map(|i| {
                page(
                    &format!("https://shell.example/page{i}"),
                    &format!("Page {i}"),
                    shell_blocks(),
                )
            })
            .collect();
        let templates = detect_templates(&pages);
        assert_eq!(templates.len(), 1, "all 10 pages should cluster");
        assert_eq!(templates[0].page_count, 10);
        assert_eq!(templates[0].block_pattern.len(), 4);

        let warnings = detect_quality_warnings(&pages, &templates);
        assert!(
            warnings.iter().any(|w| w.starts_with("spa_loading_shell:")),
            "expected spa_loading_shell warning, got {warnings:?}",
        );

        // Negative: a single page that happens to have 4 blocks shouldn't
        // trip the warning. Need 3+ pages for the warning to fire at all.
        let warnings_one = detect_quality_warnings(&pages[..1], &templates);
        assert!(
            warnings_one.is_empty(),
            "single-page bundles should not warn: {warnings_one:?}",
        );

        // Negative: same 4-block template but pages have substantial
        // content (page builder, not loading shell). Round-M iter 8:
        // Elementor / Divi WordPress sites legitimately share template
        // shapes across pages, each with 100+ words of unique content.
        let pages_with_content: Vec<PageData> = (0..10)
            .map(|i| {
                let mut pg = page(
                    &format!("https://content.example/page{i}"),
                    &format!("Page {i}"),
                    shell_blocks(),
                );
                pg.total_words = 250; // ≥ 50 / page mean
                pg
            })
            .collect();
        let warnings_content =
            detect_quality_warnings(&pages_with_content, &templates);
        assert!(
            !warnings_content
                .iter()
                .any(|w| w.starts_with("spa_loading_shell:")),
            "page-builder sites with real content should NOT warn: {warnings_content:?}",
        );

        // Negative: 10 pages of varied templates shouldn't trip the warning.
        let varied: Vec<PageData> = (0..10)
            .map(|i| {
                let blocks: Vec<ContentBlock> =
                    (0..(5 + i)).map(|j| h((j % 6) as u8 + 1, "Heading")).collect();
                page(
                    &format!("https://varied.example/page{i}"),
                    &format!("Page {i}"),
                    blocks,
                )
            })
            .collect();
        let varied_templates = detect_templates(&varied);
        let varied_warnings = detect_quality_warnings(&varied, &varied_templates);
        assert!(
            !varied_warnings.iter().any(|w| w.starts_with("spa_loading_shell:")),
            "varied pages should not warn: {varied_warnings:?}",
        );
    }

    #[test]
    fn detect_templates_groups_same_shape_pages() {
        let blocks = || {
            vec![
                ContentBlock::Image {
                    original_url: "x".to_string(),
                    local_path: "p".to_string(),
                    alt_text: "".to_string(),
                },
                h(1, "Person Name"),
            ]
        };
        let pages = vec![
            page("https://x.com/team/a", "A", blocks()),
            page("https://x.com/team/b", "B", blocks()),
            page("https://x.com/team/c", "C", blocks()),
        ];
        let templates = detect_templates(&pages);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].page_count, 3);
        assert_eq!(templates[0].block_pattern, vec!["img", "h1"]);
    }
}
