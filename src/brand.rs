use image::GenericImageView;
use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use url::Url;

use crate::model::{ColorUse, CssVariable, FontUse, PageData, WebfontUrl};
use crate::selectors::{RE_COLOR_HEX, RE_COLOR_HSL, RE_COLOR_RGB, RE_CSS_VAR, RE_FONT_FAMILY};
use crate::util::{
    extension_from_content_type, fetch_with_retry, image_extension_from_url, normalize_path,
};

/// Pygments default class names — short 1-3 letter classes assigned to
/// token types (`.k` keyword, `.s1` single-quoted string, `.cm` multi-line
/// comment, etc.). A selector is "Pygments-shaped" if every comma-separated
/// part is a `.x[a-z0-9]?[a-z0-9]?` class.
fn is_pygments_short_class(selector: &str) -> bool {
    let parts: Vec<&str> = selector.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return false;
    }
    parts.iter().all(|p| {
        let p = p.trim();
        // Drop trailing pseudoclasses / combinators.
        let core = p.split_whitespace().last().unwrap_or(p);
        let core = core.split(':').next().unwrap_or(core);
        if !core.starts_with('.') {
            return false;
        }
        let rest = &core[1..];
        if rest.is_empty() || rest.len() > 4 {
            return false;
        }
        rest.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

/// Mine the concatenated style text for hex/rgb/hsl colors and
/// `font-family` declarations. Returns sorted lists by frequency.
pub(crate) fn aggregate_brand_palette(
    pages: &[PageData],
    extra_css: &str,
    top_n: usize,
) -> (Vec<ColorUse>, Vec<FontUse>, Vec<CssVariable>) {
    let mut colors: HashMap<String, usize> = HashMap::new();
    let mut fonts: HashMap<String, usize> = HashMap::new();
    let mut css_vars: HashMap<(String, String), usize> = HashMap::new();

    let ignore_colors: HashSet<&str> = ["#fff", "#ffffff", "#000", "#000000", "#0000", "#00000000"]
        .into_iter()
        .collect();

    // Selectors / class names that indicate syntax-highlighting rules.
    // Colours defined inside these rules are noise (Atom One Dark, Dracula,
    // Prism themes, etc. dominate the brand palette on dev-docs sites).
    // We scan rule-by-rule and skip any whose selector contains one of these.
    let is_syntax_selector = |selector: &str| {
        let s = selector.to_lowercase();
        s.contains(".hljs")
            || s.contains(".token")
            || s.contains("prism")
            || s.contains(".highlight")
            || s.contains("pre code")
            || s.contains(".code-")
            || s.contains("[data-language")
            || s.contains(".chroma")
            || s.contains(".rouge")
            // Pygments default classes (used by MkDocs, Sphinx, Jekyll).
            // Selectors look like ".highlight .c", ".highlight .k",
            // ".highlight .s1". Detect by short single/double-letter class
            // immediately preceded by ".highlight" — too narrow on its own,
            // so also catch `.codehilite` (Jekyll/Pygments default wrapper).
            || s.contains(".codehilite")
            // Generic single-letter Pygments classes — only ever appear in
            // syntax-highlighting CSS. Each Pygments rule is a separate
            // selector like `.c1`, `.kc`, etc.
            || is_pygments_short_class(&s)
    };

    // Fully-transparent values to drop. `rgba(0,0,0,0)`, `rgba(255,255,255,0)`,
    // any `#xxxxxx00` hex literal (8-digit hex ending in zero alpha).
    let is_transparent = |s: &str| -> bool {
        let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        let lc = cleaned.to_lowercase();
        // 8-digit hex literal with zero alpha (last 2 chars == "00").
        if lc.starts_with('#') && lc.len() == 9 && lc.ends_with("00") {
            return true;
        }
        // rgba(*, *, *, 0) / rgba(*, *, *, 0.0).
        if let Some(args) = lc.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
            let parts: Vec<&str> = args.split(',').map(str::trim).collect();
            if parts.len() == 4 {
                if let Ok(alpha) = parts[3].parse::<f32>() {
                    return alpha == 0.0;
                }
            }
        }
        false
    };
    let scan_colors_in = |text: &str, colors: &mut HashMap<String, usize>| {
        for m in RE_COLOR_HEX.find_iter(text) {
            let v = m.as_str().to_lowercase();
            if ignore_colors.contains(v.as_str()) || is_transparent(&v) {
                continue;
            }
            *colors.entry(v).or_default() += 1;
        }
        for m in RE_COLOR_RGB.find_iter(text) {
            let v = m.as_str().to_lowercase();
            if is_transparent(&v) {
                continue;
            }
            *colors.entry(v).or_default() += 1;
        }
        for m in RE_COLOR_HSL.find_iter(text) {
            *colors.entry(m.as_str().to_lowercase()).or_default() += 1;
        }
    };

    let scan_one = |text: &str,
                    colors: &mut HashMap<String, usize>,
                    fonts: &mut HashMap<String, usize>,
                    css_vars: &mut HashMap<(String, String), usize>| {
        // Walk CSS rule by rule and skip syntax-highlighting rules.
        // We split on `}` and treat each chunk as `selector { body }`.
        for chunk in text.split('}') {
            let (selector, body) = match chunk.split_once('{') {
                Some((s, b)) => (s, b),
                None => ("", chunk),
            };
            if is_syntax_selector(selector) {
                continue;
            }
            scan_colors_in(body, colors);
        }
        for cap in RE_FONT_FAMILY.captures_iter(text) {
            if let Some(fam_raw) = cap.get(1) {
                let first = fam_raw
                    .as_str()
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string();
                if first.is_empty() {
                    continue;
                }
                let first_lc = first.to_lowercase();
                if matches!(
                    first_lc.as_str(),
                    "inherit"
                        | "initial"
                        | "unset"
                        | "sans-serif"
                        | "serif"
                        | "monospace"
                        | "cursive"
                        | "fantasy"
                        | "system-ui"
                        | "ui-sans-serif"
                        | "ui-serif"
                        | "ui-monospace"
                        | "ui-rounded"
                        | "emoji"
                        | "math"
                        | "fangsong"
                ) || first.starts_with("var(")
                    || first.starts_with("--")
                {
                    continue;
                }
                // Filter CSS-implementation artifacts: Next.js / Tailwind /
                // fontaine generate fallback families like "Inter-fallback",
                // "md-io-fallback", "AtlasGrotesk Fallback", "__Inter_e8ce0c".
                // Also reject tokens that still contain "!important".
                // Round K: "AtlasGrotesk Fallback" (space + capital F) on
                // Prusa3D leaked because the old check only matched the
                // kebab-case form. Now match -fallback / _fallback /
                // " fallback" suffixes case-insensitively.
                if first_lc.ends_with("-fallback")
                    || first_lc.ends_with("_fallback")
                    || first_lc.ends_with(" fallback")
                    || first_lc.contains("!important")
                    || first.starts_with("__")
                {
                    continue;
                }
                // URL-encoded font name fragments. Brooklyn Brewery's
                // CSS-in-JS captured `Libre Franklin%3A300%2C300i…` as a
                // family name — that's URL-encoded font-weight metadata.
                if first.contains("%2C")
                    || first.contains("%3A")
                    || first.contains("%2F")
                    || first.contains("%2c")
                    || first.contains("%3a")
                    || first.contains("%2f")
                {
                    continue;
                }
                // Adobe Typekit weight-encoded aliases. Footshop.cz leaked
                // `tk-neue-haas-unica-n4` and `…-n5` — these are Typekit's
                // weight-suffix family names (n4 = weight 400, n5 = 500).
                // The real font (`neue-haas-unica`) is captured separately;
                // these are CSS-implementation artifacts. Pattern:
                // `tk-<anything>-n<digits>`.
                if first_lc.starts_with("tk-") {
                    if let Some(tail) = first_lc.rsplit('-').next() {
                        if tail.starts_with('n')
                            && tail.len() >= 2
                            && tail[1..].chars().all(|c| c.is_ascii_digit())
                        {
                            continue;
                        }
                    }
                }
                // Icon-font filter. Catbird's `catbird-icons` (77×) and
                // Schoolhouse's `swiper-icons` were ranking #3 in the
                // brand fonts list. Icon fonts are never typographic
                // brand choices. Match common patterns + known names.
                if first_lc.ends_with("-icons")
                    || first_lc.ends_with("-icon")
                    || first_lc.ends_with(" icons")
                    || first_lc.ends_with(" icon")
                    || first_lc.contains("iconic")
                    || first_lc.contains("glyph")
                    || first_lc.starts_with("font awesome")
                    || first_lc.starts_with("fontawesome")
                    || first_lc.starts_with("material icons")
                    || first_lc.starts_with("material symbols")
                    || matches!(
                        first_lc.as_str(),
                        "etmodules"
                            | "swiper-icons"
                            | "slick-icons"
                            | "ionicons"
                            | "feather"
                            | "octicons"
                            | "pagebuilder-font"
                            | "solid-icons"
                            | "wp-icons"
                            // Slider / carousel library CSS-class names
                            // that get caught as "font" because the CSS
                            // declares `font-family: 'slick'` on a pseudo
                            // element for the arrow glyphs. Round K:
                            // Mejuri leaked `slick (6×)`.
                            | "slick"
                            | "swiper"
                            | "splide"
                            | "flickity"
                            | "owl"
                            | "owl carousel"
                            | "owl-carousel"
                            // Bare icon-font generator / convention names.
                            // These are universal icon-font naming
                            // conventions, not brand typography.
                            | "icons"
                            | "icon"
                            | "icomoon"
                            | "iconmoon"
                            | "icofont"
                            | "themify"
                            | "lineicons"
                            | "dripicons"
                            | "linearicons"
                            // Elementor / ElementsKit page-builder icon
                            // libraries — universal across thousands of
                            // WordPress + Elementor sites.
                            | "eicons"
                            | "elementskit"
                            | "essential-addons"
                            | "ekiticons"
                            | "uicons"
                    )
                {
                    continue;
                }
                // Character-class whitelist: real font names are
                // alphanumeric + space + hyphen, length 2–40. Reject
                // anything else — kills CSS-property leakage like
                // "object-fit:cover", URL fragments, calc() values, etc.
                if first.len() < 2 || first.len() > 40 {
                    continue;
                }
                if !first
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
                {
                    continue;
                }
                *fonts.entry(first).or_default() += 1;
            }
        }
        for cap in RE_CSS_VAR.captures_iter(text) {
            let name = cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let value = cap
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            if name.is_empty() || value.is_empty() {
                continue;
            }
            // Skip data-URLs and very long values (likely embedded images).
            if value.len() > 200 || value.starts_with("data:") {
                continue;
            }
            *css_vars.entry((name, value)).or_default() += 1;
        }
    };

    for p in pages {
        if !p.style_text.is_empty() {
            scan_one(&p.style_text, &mut colors, &mut fonts, &mut css_vars);
        }
    }
    if !extra_css.is_empty() {
        scan_one(extra_css, &mut colors, &mut fonts, &mut css_vars);
    }

    let mut colors_v: Vec<ColorUse> = colors
        .into_iter()
        .map(|(value, count)| ColorUse { value, count })
        .collect();
    colors_v.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));

    // Known popular syntax-highlighting palettes. If 3+ colors from one of
    // these appear in our scanned palette, drop all of them — they're the
    // theme used for `<pre><code>` blocks, not the site's brand. Without
    // this, dev-docs sites (htmx.org, etc.) report code colours as brand.
    let known_themes: &[&[&str]] = &[
        // Atom One Dark
        &[
            "#e06c75", "#98c379", "#d19a66", "#61afef", "#c678dd", "#56b6c2", "#abb2bf", "#282c34",
        ],
        // Dracula
        &[
            "#ff79c6", "#bd93f9", "#50fa7b", "#ffb86c", "#8be9fd", "#f1fa8c", "#ff5555", "#282a36",
        ],
        // Monokai (classic)
        &[
            "#f92672", "#a6e22e", "#fd971f", "#66d9ef", "#ae81ff", "#e6db74", "#272822", "#75715e",
        ],
        // Tomorrow Night
        &[
            "#cc6666", "#b5bd68", "#f0c674", "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6", "#1d1f21",
        ],
        // Solarized Dark
        &[
            "#268bd2", "#dc322f", "#859900", "#b58900", "#6c71c4", "#d33682", "#2aa198", "#cb4b16",
        ],
        // Nord
        &[
            "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#d08770", "#5e81ac",
        ],
        // Catppuccin Mocha
        &[
            "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#cba6f7", "#94e2d5", "#fab387", "#eba0ac",
        ],
        // Bootstrap 3 utility palette — danger/success/warning/info button
        // colors. Death & Co was reporting these as brand colours because
        // Bootstrap's stylesheet declares them hundreds of times in default
        // button / alert / badge rules. Real brand sits beneath. These
        // hexes are specifically utility-semantic (success-green,
        // danger-red, warning-yellow) and rarely match a real brand by
        // coincidence — 3+ matches is a strong signal.
        &[
            "#d9534f", "#5cb85c", "#f0ad4e", "#5bc0de", "#428bca", "#337ab7", "#3071a9", "#46b8da",
        ],
        // Bootstrap 4 utility palette.
        &[
            "#dc3545", "#28a745", "#ffc107", "#17a2b8", "#007bff", "#6c757d", "#343a40", "#f8f9fa",
        ],
    ];
    // NOTE: Tailwind/Material default grays are deliberately NOT in this
    // list. Many real brands legitimately use those exact hexes as their
    // neutral text palette (Plausible's `#374151` is genuinely their
    // primary text colour). Filtering them would strip real brand data.
    let theme_block: std::collections::HashSet<String> = {
        let scanned: std::collections::HashSet<&str> =
            colors_v.iter().map(|c| c.value.as_str()).collect();
        let mut block = std::collections::HashSet::new();
        for theme in known_themes {
            let hits = theme.iter().filter(|c| scanned.contains(*c)).count();
            if hits >= 3 {
                block.extend(theme.iter().map(|s| s.to_string()));
            }
        }
        block
    };
    if !theme_block.is_empty() {
        colors_v.retain(|c| !theme_block.contains(&c.value));
    }

    colors_v.truncate(top_n);

    // Merge font weight-suffix variants into the base family. Mahabis.com
    // smoketest emitted `AktivGrotest`, `AktivGrotestBold`,
    // `AktivGrotestHairline`, `AktivGrotesk` as four separate entries —
    // they're the same family. The agent only needs to know "Aktiv
    // Grotesk". `normalize_font_family()` strips a trailing weight word
    // (Bold / Light / Medium / Black / Thin / Hairline / etc.) when
    // detected via space / hyphen / camelCase boundary, then we re-group
    // by the normalized base name and sum counts. The base name with the
    // highest individual count wins as the display variant.
    let mut by_base: HashMap<String, (String, usize)> = HashMap::new();
    for (family, count) in fonts {
        let base = normalize_font_family(&family);
        match by_base.get(&base) {
            Some((existing_disp, existing_count)) => {
                let new_total = existing_count + count;
                let new_disp = if count > *existing_count {
                    family.clone()
                } else {
                    existing_disp.clone()
                };
                by_base.insert(base, (new_disp, new_total));
            }
            None => {
                by_base.insert(base, (family.clone(), count));
            }
        }
    }
    let mut fonts_v: Vec<FontUse> = by_base
        .into_iter()
        .map(|(_base, (family, count))| FontUse { family, count })
        .collect();
    fonts_v.sort_by(|a, b| b.count.cmp(&a.count).then(a.family.cmp(&b.family)));
    fonts_v.truncate(top_n);

    let mut vars_v: Vec<CssVariable> = css_vars
        .into_iter()
        .map(|((name, value), count)| CssVariable { name, value, count })
        .collect();
    vars_v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    vars_v.truncate(top_n * 2);

    (colors_v, fonts_v, vars_v)
}

/// Normalize a font family name by stripping common weight / style
/// suffixes. Returns the lowercased base form for dedup keying.
///
/// `AktivGrotestBold`, `Aktiv Grotest Bold`, `Aktiv-Grotest-Bold` all
/// collapse to `aktivgrotest`. `Helvetica` stays `helvetica`.
/// Handles weight words: Thin, ExtraLight/UltraLight, Light, Regular,
/// Book, Medium, SemiBold/DemiBold, Bold, ExtraBold, Black, Heavy,
/// Hairline. Plus style words: Italic, Oblique.
///
/// Conservative: only strips a suffix when the result is ≥3 chars and
/// the suffix is clearly a separate token (preceded by space / hyphen /
/// camelCase boundary) so we don't break legitimate names ending in
/// these words by coincidence.
pub(crate) fn normalize_font_family(family: &str) -> String {
    let weight_words = [
        "hairline",
        "ultralight",
        "extralight",
        "thin",
        "light",
        "book",
        "regular",
        "normal",
        "medium",
        "demibold",
        "semibold",
        "bold",
        "extrabold",
        "ultrabold",
        "heavy",
        "black",
        "italic",
        "oblique",
    ];
    let mut result = family.to_string();
    // Iterate: a font name may have stacked suffixes ("Bold Italic").
    loop {
        let lc = result.to_lowercase();
        let mut stripped = false;
        for word in weight_words.iter() {
            // 1. Space-separated suffix: "Helvetica Bold" → "Helvetica"
            if let Some(idx) = lc.rfind(&format!(" {word}")) {
                if idx + 1 + word.len() == lc.len() && idx >= 3 {
                    result.truncate(idx);
                    stripped = true;
                    break;
                }
            }
            // 2. Hyphen-separated suffix: "Helvetica-Bold" → "Helvetica"
            if let Some(idx) = lc.rfind(&format!("-{word}")) {
                if idx + 1 + word.len() == lc.len() && idx >= 3 {
                    result.truncate(idx);
                    stripped = true;
                    break;
                }
            }
            // 3. CamelCase suffix: "AktivGrotestBold" → "AktivGrotest"
            // Detect: the word appears at end, preceded by a lowercase
            // letter (so we know it's a separate camelCase token, not
            // just a coincidental substring at end).
            let word_at_end_idx = lc.len().saturating_sub(word.len());
            if word_at_end_idx >= 3 && &lc[word_at_end_idx..] == *word {
                let prev_char = result[..word_at_end_idx]
                    .chars()
                    .next_back();
                let next_char = result[word_at_end_idx..].chars().next();
                if matches!(prev_char, Some(c) if c.is_ascii_lowercase())
                    && matches!(next_char, Some(c) if c.is_ascii_uppercase())
                {
                    result.truncate(word_at_end_idx);
                    stripped = true;
                    break;
                }
            }
        }
        if !stripped {
            break;
        }
        // Strip any trailing whitespace / hyphen / underscore left behind.
        result = result.trim_end_matches([' ', '-', '_']).to_string();
        if result.len() < 3 {
            // Stripped too aggressively; revert and stop.
            return family.to_lowercase();
        }
    }
    // Final lower-case for dedup keying.
    result.to_lowercase()
}

/// Merge webfont-URL family names into the existing fonts list. Each family
/// in the webfont URL gets a synthetic +5 boost — if a site loads
/// `fonts.googleapis.com?family=Inter` we know Inter is in use even when the
/// CSS only references it via `var(--font-sans)`. Naturally-scanned
/// occurrences still dominate when present.
pub(crate) fn merge_webfont_families(
    fonts: &mut Vec<FontUse>,
    webfonts: &[crate::model::WebfontUrl],
    top_n: usize,
) {
    let mut by_family: HashMap<String, usize> =
        fonts.iter().map(|f| (f.family.clone(), f.count)).collect();
    for wf in webfonts {
        for fam in &wf.families {
            // Drop "*-fallback" if any leak in from a webfont parse.
            if fam.to_lowercase().ends_with("-fallback") {
                continue;
            }
            *by_family.entry(fam.clone()).or_default() += 5;
        }
    }
    let mut merged: Vec<FontUse> = by_family
        .into_iter()
        .map(|(family, count)| FontUse { family, count })
        .collect();
    merged.sort_by(|a, b| b.count.cmp(&a.count).then(a.family.cmp(&b.family)));
    merged.truncate(top_n);
    *fonts = merged;
}

/// Extract dominant colors from a local image file by quantising pixels to
/// a coarse colour space and counting frequencies. Skips near-white,
/// near-black, and transparent pixels (those dominate raster logos and
/// aren't useful brand signals).
pub(crate) fn dominant_colors_from_image(path: &Path, top_n: usize) -> Vec<ColorUse> {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };
    // Downscale to cap pixel count regardless of original size.
    let (w, h) = img.dimensions();
    let cap = 200u32;
    let img = if w > cap || h > cap {
        let scale = (cap as f32) / (w.max(h) as f32);
        let new_w = ((w as f32) * scale) as u32;
        let new_h = ((h as f32) * scale) as u32;
        img.resize(
            new_w.max(1),
            new_h.max(1),
            image::imageops::FilterType::Nearest,
        )
    } else {
        img
    };
    let rgba = img.to_rgba8();

    // Quantise each channel into 4-bit (16 buckets per channel) → 4096 colours.
    let mut counts: HashMap<(u8, u8, u8), usize> = HashMap::new();
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        if a < 200 {
            continue; // skip near-transparent
        }
        // Skip near-white and near-black (common background / outline)
        let luma = (r as u16 + g as u16 + b as u16) / 3;
        if !(20..=240).contains(&luma) {
            continue;
        }
        let qr = r & 0b1111_0000;
        let qg = g & 0b1111_0000;
        let qb = b & 0b1111_0000;
        *counts.entry((qr, qg, qb)).or_default() += 1;
    }

    let mut entries: Vec<((u8, u8, u8), usize)> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(top_n);

    entries
        .into_iter()
        .map(|((r, g, b), count)| ColorUse {
            value: format!("#{r:02x}{g:02x}{b:02x}"),
            count,
        })
        .collect()
}

/// Detect webfont CDN URLs (Google Fonts, Adobe Fonts, Bunny Fonts) from a
/// list of stylesheet URLs and parse the families they load.
pub(crate) fn detect_webfont_urls(stylesheet_urls: &[String]) -> Vec<WebfontUrl> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for url in stylesheet_urls {
        if !seen.insert(url.clone()) {
            continue;
        }
        let url_lc = url.to_lowercase();
        let (provider, families) = if url_lc.contains("fonts.googleapis.com") {
            ("google", parse_google_fonts_url(url))
        } else if url_lc.contains("use.typekit.net") || url_lc.contains("typekit.com") {
            ("adobe", Vec::new())
        } else if url_lc.contains("fonts.bunny.net") {
            ("bunny", parse_google_fonts_url(url))
        } else if url_lc.contains("fonts.cdnfonts.com") {
            ("cdnfonts", Vec::new())
        } else {
            continue;
        };
        out.push(WebfontUrl {
            provider: provider.to_string(),
            families,
            url: url.clone(),
        });
    }
    out
}

/// Minimal `%XX` → byte decoder. Only handles the two characters we
/// actually care about for Google Fonts URLs: `%3A` (`:`) and `%2C` (`,`).
/// Both case variations. Falls back to leaving unrecognised escapes as-is
/// — we don't need full RFC 3986 decoding here.
fn percent_decode_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            match hex.to_ascii_uppercase().as_str() {
                "3A" => {
                    out.push(':');
                    i += 3;
                    continue;
                }
                "2C" => {
                    out.push(',');
                    i += 3;
                    continue;
                }
                "20" => {
                    out.push(' ');
                    i += 3;
                    continue;
                }
                "2B" => {
                    out.push('+');
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_google_fonts_url(url: &str) -> Vec<String> {
    // Family params look like `?family=Inter:wght@400;700&family=Roboto&display=swap`
    // OR — and this was the Brooklyn Brewery bug — fully URL-encoded:
    // `?family=Libre+Franklin%3A300%2C300i%2C400…`. Without URL-decoding
    // the `%3A`/`%2C` sequences, the split-on-`:` step doesn't find the
    // weight delimiter and we emit the whole encoded blob as a font name.
    let Some(query_start) = url.find('?') else {
        return Vec::new();
    };
    let query = &url[query_start + 1..];
    let mut out: HashSet<String> = HashSet::new();
    for pair in query.split('&') {
        if let Some(rest) = pair.strip_prefix("family=") {
            // Decode `+` (form-encoded space) and `%XX` escapes.
            let decoded = percent_decode_minimal(&rest.replace('+', " "));
            // Take everything before the first `:` or `,` (both delimit
            // weight/style params).
            let family = decoded
                .split([':', ','])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !family.is_empty() {
                out.insert(family);
            }
        }
    }
    let mut v: Vec<String> = out.into_iter().collect();
    v.sort();
    v
}

/// Download a remote asset (favicon, logo) using the reqwest client and a
/// fixed filename. Returns the relative `output/...` path on success.
/// Sniffs the `Content-Type` header to pick the correct extension when the
/// URL is something like `_next/image?url=…` where the path doesn't tell us.
pub(crate) async fn download_asset(
    client: &Client,
    url: &str,
    output_dir: &str,
    name: &str,
) -> Option<String> {
    match fetch_with_retry(client, url, 2).await {
        Some(resp) if resp.status().is_success() => {
            let ext = resp
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .and_then(extension_from_content_type)
                .unwrap_or_else(|| image_extension_from_url(url));
            let filename = format!("{name}.{ext}");
            let filepath = format!("{output_dir}/{filename}");

            if Path::new(&filepath).exists() {
                return Some(normalize_path(&filepath));
            }

            if let Ok(bytes) = resp.bytes().await {
                if bytes.is_empty() {
                    return None;
                }
                if tokio::fs::write(&filepath, &bytes).await.is_ok() {
                    return Some(normalize_path(&filepath));
                }
            }
        }
        _ => {}
    }
    None
}

/// Fetch external stylesheets, concatenate them. Returns (combined_text,
/// per-url byte sizes for the asset manifest).
pub(crate) async fn fetch_external_css(client: &Client, urls: &[String]) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = String::new();
    for url in urls {
        if !seen.insert(url.clone()) {
            continue;
        }
        // Cap on stylesheets per site to avoid runaway fetches.
        if seen.len() > 20 {
            break;
        }
        // Resolve relative or protocol-relative URLs.
        let parsed = match Url::parse(url) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let parsed_str = parsed.to_string();
        if let Some(resp) = fetch_with_retry(client, &parsed_str, 1).await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if text.len() < 5_000_000 {
                        out.push_str(&text);
                        out.push('\n');
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_font_family_strips_weight_suffixes() {
        // CamelCase (Mahabis regression: AktivGrotestBold vs AktivGrotest).
        assert_eq!(normalize_font_family("AktivGrotestBold"), "aktivgrotest");
        assert_eq!(normalize_font_family("AktivGrotestHairline"), "aktivgrotest");
        assert_eq!(normalize_font_family("AktivGrotest"), "aktivgrotest");
        // Space-separated.
        assert_eq!(normalize_font_family("Helvetica Bold"), "helvetica");
        assert_eq!(normalize_font_family("Roboto Medium"), "roboto");
        assert_eq!(normalize_font_family("Inter Black"), "inter");
        // Hyphen-separated.
        assert_eq!(normalize_font_family("Helvetica-Bold"), "helvetica");
        // Stacked suffixes.
        assert_eq!(normalize_font_family("Helvetica Bold Italic"), "helvetica");
        // Plain name unchanged.
        assert_eq!(normalize_font_family("Helvetica"), "helvetica");
        assert_eq!(normalize_font_family("Open Sans"), "open sans");
        // Don't strip when result would be too short.
        assert_eq!(normalize_font_family("Bold"), "bold");
        // CamelCase only triggers on lowercase→uppercase boundary, so
        // legitimate name "Helvetica" with no boundary stays.
        assert_eq!(normalize_font_family("BMW"), "bmw");
    }

    #[test]
    fn parse_google_fonts_url_handles_plus_encoded() {
        let urls = vec![
            "https://fonts.googleapis.com/css?family=Inter:wght@400;700&family=Roboto&display=swap"
                .to_string(),
        ];
        let mut out: Vec<String> = urls
            .iter()
            .flat_map(|u| parse_google_fonts_url(u))
            .collect();
        out.sort();
        assert_eq!(out, vec!["Inter".to_string(), "Roboto".to_string()]);
    }

    #[test]
    fn parse_google_fonts_url_handles_percent_encoded() {
        // Brooklyn Brewery regression: fully URL-encoded query string
        // produced `Libre Franklin%3A300%2C300i%2C400%2C400i%2C600%2C600i%2C800%2C800i`
        // as a font family. percent_decode_minimal + split-on-`,|:` fix.
        let url = "https://fonts.googleapis.com/css?family=Libre+Franklin%3A300%2C300i%2C400%2C400i%2C600%2C600i%2C800%2C800i";
        let out = parse_google_fonts_url(url);
        assert_eq!(out, vec!["Libre Franklin".to_string()]);
    }

    #[test]
    fn percent_decode_minimal_handles_known_escapes() {
        assert_eq!(percent_decode_minimal("Libre%20Franklin"), "Libre Franklin");
        assert_eq!(percent_decode_minimal("a%3Ab%2Cc"), "a:b,c");
        assert_eq!(percent_decode_minimal("a%2bb"), "a+b");
        // Unknown escape preserved as-is.
        assert_eq!(percent_decode_minimal("a%99b"), "a%99b");
        // No escapes — identity.
        assert_eq!(percent_decode_minimal("hello"), "hello");
    }
}
