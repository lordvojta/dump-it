use scraper::Html;
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use url::Url;

use scraper::ElementRef;

use crate::model::{ContactInfo, SocialLink};
use crate::selectors::{
    RE_EMAIL, RE_LOOKS_LIKE_DATE, RE_PHONE, SEL_FOOTER, SEL_LINK, SEL_NAV, SOCIAL_DOMAINS,
};
use crate::util::{body_text_only, element_text};

/// Collect rendered text from chrome zones (nav, header, footer, and any
/// element whose class/id contains "contact" / "address" / "footer"). Used
/// to bound email extraction so blog/article body mentions of personal
/// emails (e.g. founder's Gmail in a 2019 post) don't pollute the brand's
/// company-contact list.
fn chrome_text(doc: &scraper::Html) -> String {
    let mut out = String::new();
    for el in doc.select(&SEL_NAV) {
        out.push_str(&element_text(&el));
        out.push(' ');
    }
    for el in doc.select(&SEL_FOOTER) {
        out.push_str(&element_text(&el));
        out.push(' ');
    }
    // Best-effort contact/address class/id zones.
    let zone_sel = scraper::Selector::parse(
        "[class*='contact' i], [id*='contact' i], [class*='address' i], [id*='address' i], [class*='footer' i], [id*='footer' i]",
    )
    .ok();
    if let Some(sel) = zone_sel {
        for el in doc.select(&sel) {
            // Skip if already covered by nav/footer ancestor to avoid duplication.
            if el
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(|a| matches!(a.value().name(), "nav" | "header" | "footer"))
            {
                continue;
            }
            out.push_str(&element_text(&el));
            out.push(' ');
        }
    }
    out
}

/// Conservative phone validator. Filters SVG-path matches, dates, and
/// pure-digit strings that aren't actually phone numbers.
pub(crate) fn looks_like_phone(s: &str) -> bool {
    let trimmed = s.trim();
    if RE_LOOKS_LIKE_DATE.is_match(trimmed) {
        return false;
    }
    // Reject candidates that start with a 4-digit year (1900-2099) followed
    // by any separator — these are dates / thread IDs / etc., not phones.
    // The earlier date regex catches ISO dates with 2-digit month/day; this
    // covers shapes like "2026-45185" that slip past it.
    let first_group: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if first_group.len() == 4 {
        if let Ok(year) = first_group.parse::<u32>() {
            if (1900..=2099).contains(&year) {
                return false;
            }
        }
    }
    // Reject phone candidates that span line breaks — visible-text builder
    // concatenates element content with `\n`. A phone that crosses lines is
    // almost certainly two unrelated numbers stitched together (e.g. Shopify
    // product cards: "1762296503 \n       3").
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    let digits: usize = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
    if !(9..=15).contains(&digits) {
        return false;
    }
    // Reject Unix timestamps. A pure 10-digit integer starting with 1[5-9]
    // is a timestamp between 2017-04 and 2033-09 — almost certainly not a
    // phone. (Real 10-digit phones start with country/area codes that
    // aren't 15/16/17/18/19.)
    let pure_digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if pure_digits.len() == 10
        && trimmed.chars().all(|c| c.is_ascii_digit())
        && matches!(
            pure_digits.chars().next(),
            Some('1')
        )
        && matches!(
            pure_digits.chars().nth(1),
            Some('5') | Some('6') | Some('7') | Some('8') | Some('9')
        )
    {
        return false;
    }
    // Mixed separators (e.g. "059   301   71.58" — multi-space + a dot) are
    // never real phone numbers. Real phones use ONE consistent separator.
    // French format "06.12.34.56.78" uses only dots; allow that.
    //
    // Also: ANY two consecutive spaces is suspicious. Body-text
    // tokenization joins HTML elements with single spaces, so multi-space
    // only appears when source had column alignment / tables / ASCII art.
    // Round-I regression: Catbird's "48   72   96   120" delivery-zone
    // calendar leaked through with multi-space gaps.
    let has_multi_space = trimmed.contains("  ");
    let has_dot = trimmed.contains('.');
    if has_multi_space {
        return false;
    }
    if has_multi_space && has_dot {
        return false;
    }
    if trimmed.chars().filter(|c| *c == '.').count() > 1
        && !trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '+' || c.is_whitespace())
    {
        return false;
    }
    // If a dot appears, it must be a consistent separator (every gap between
    // digit groups is a dot). Reject mixed dot+other separators.
    if has_dot && trimmed.contains('-') {
        return false;
    }
    if has_dot && trimmed.contains(' ') && !trimmed.starts_with('+') {
        // "+1 555.123.4567" is plausible (country code uses space, rest dots).
        // Otherwise a stray dot in a space-separated number is suspicious.
        return false;
    }
    // Count groups AND track each group's length. Real phones have groups
    // of 2-4 digits (except the leading country code which can be 1).
    // Round K: Buckmason leaked `11301667 3` (8-digit group + 1-digit
    // group) — clearly a product SKU. Reject when any non-first group
    // has < 2 digits.
    let mut groups = 0;
    let mut in_group = false;
    let mut group_sizes: Vec<usize> = Vec::new();
    for c in trimmed.chars() {
        if c.is_ascii_digit() {
            if !in_group {
                groups += 1;
                in_group = true;
                group_sizes.push(1);
            } else {
                *group_sizes.last_mut().unwrap() += 1;
            }
        } else {
            in_group = false;
        }
    }
    // Skip the first group when checking (country codes like "1" or "44"
    // can legitimately be a single digit).
    if group_sizes.iter().skip(1).any(|&n| n < 2) {
        return false;
    }
    // Developer-placeholder detection: phones like `+420 111 222 333`
    // or compact `+420111222333` (each non-country-code segment is a
    // single repeating digit). Detect via the digit-only form: if any
    // 9-character window contains three consecutive mono-digit triples
    // (`AAABBBCCC` where A, B, C may differ), it's almost certainly a
    // placeholder. Real phones effectively never produce this pattern.
    let pure_digits_seq: String =
        trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if pure_digits_seq.len() >= 9 {
        let bytes = pure_digits_seq.as_bytes();
        for start in 0..=bytes.len().saturating_sub(9) {
            let window = &bytes[start..start + 9];
            let triple_a_mono = window[0] == window[1] && window[1] == window[2];
            let triple_b_mono = window[3] == window[4] && window[4] == window[5];
            let triple_c_mono = window[6] == window[7] && window[7] == window[8];
            if triple_a_mono && triple_b_mono && triple_c_mono {
                return false;
            }
        }
    }
    // Real phone numbers have 1-4 digit groups in most formats
    // (country + area + exchange + line). French national format breaks
    // each pair with a dot, producing 5 groups: `06.12.34.56.78`.
    // We allow 5 groups only when the separator style is exclusively
    // dots; space-separated 5+ groups (like kotn's store-hours calendar
    // `29 30 31 32 34 36`) are rejected.
    let dot_count = trimmed.chars().filter(|c| *c == '.').count();
    let space_count = trimmed.chars().filter(|c| *c == ' ').count();
    let dash_count = trimmed.chars().filter(|c| *c == '-').count();
    let max_groups = if dot_count >= 3 && space_count == 0 && dash_count == 0 {
        5
    } else {
        4
    };
    if !(1..=max_groups).contains(&groups) {
        return false;
    }
    let has_plus = trimmed.starts_with('+');
    let has_sep = trimmed.contains(' ')
        || trimmed.contains('-')
        || trimmed.contains('(')
        || trimmed.contains(')')
        || trimmed.contains('.');
    if !has_plus && !has_sep {
        return false;
    }
    true
}

/// Repair common paren-balance issues in extracted phone strings.
/// `body_text_only` concatenates HTML text nodes with single spaces and the
/// regex sometimes starts mid-token, leaving `718) 486-7422` (orphan `)`)
/// or `+1 (415` (orphan `(`). Strip the unbalanced characters and, when
/// the result is a clean US-shaped 10-digit string with an internal `)`,
/// restore the missing leading `(`.
pub(crate) fn balance_phone_parens(s: &str) -> String {
    let trimmed = s.trim();
    let opens = trimmed.chars().filter(|c| *c == '(').count();
    let closes = trimmed.chars().filter(|c| *c == ')').count();
    if opens == closes {
        return trimmed.to_string();
    }
    if closes > opens {
        // Strip leading orphan ')' characters and any whitespace they leave.
        let mut chars: Vec<char> = trimmed.chars().collect();
        let mut to_strip = closes - opens;
        while to_strip > 0 && !chars.is_empty() {
            match chars[0] {
                ')' => {
                    chars.remove(0);
                    to_strip -= 1;
                }
                c if c.is_whitespace() => {
                    chars.remove(0);
                }
                _ => break,
            }
        }
        let stripped: String = chars.into_iter().collect();
        let stripped = stripped.trim().to_string();
        // If the result still has an internal `)` and looks like a US
        // 10-digit phone (e.g. "718) 486-7422" → strip leading orphan
        // already happened, but if we had "718) 486-7422" with the orphan
        // stripped we'd get "718 486-7422"; alternatively detect US 10
        // digits and re-add `(`).
        // Simple approach: if there's still a `)` in the string, the user
        // probably wanted `(XXX) ...` form — re-add `(` at the start when
        // digits-before-`)` is exactly 3.
        if let Some(close_pos) = stripped.find(')') {
            let prefix = &stripped[..close_pos];
            if prefix.chars().filter(|c| c.is_ascii_digit()).count() == 3
                && prefix
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_whitespace())
            {
                return format!("({}", stripped);
            }
        }
        return stripped;
    }
    // opens > closes — strip trailing orphan '(' chars.
    let mut chars: Vec<char> = trimmed.chars().collect();
    let mut to_strip = opens - closes;
    while to_strip > 0 && !chars.is_empty() {
        let last = *chars.last().unwrap();
        match last {
            '(' => {
                chars.pop();
                to_strip -= 1;
            }
            c if c.is_whitespace() => {
                chars.pop();
            }
            _ => break,
        }
    }
    chars.into_iter().collect::<String>().trim().to_string()
}

/// Minimal `%XX` → byte decoder for `mailto:` hrefs. Handles `%40` (`@`),
/// `%2B` (`+`), `%2E` (`.`), `%2D` (`-`), `%5F` (`_`), `%20` (space),
/// `%2F` (`/`). Other escapes pass through unchanged. Round M3 fix:
/// knihy.cz `<a href="mailto:info%40knihy.cz">` was emitting the literal
/// `info%40knihy.cz` as an email because we didn't decode the URL escape.
pub(crate) fn percent_decode_email(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            match hex.to_ascii_uppercase().as_str() {
                "40" => {
                    out.push('@');
                    i += 3;
                    continue;
                }
                "2B" => {
                    out.push('+');
                    i += 3;
                    continue;
                }
                "2E" => {
                    out.push('.');
                    i += 3;
                    continue;
                }
                "2D" => {
                    out.push('-');
                    i += 3;
                    continue;
                }
                "5F" => {
                    out.push('_');
                    i += 3;
                    continue;
                }
                "20" => {
                    // Spaces in emails are invalid — skip rather than insert.
                    i += 3;
                    continue;
                }
                "2F" => {
                    out.push('/');
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

/// Check whether an email's domain matches the site's own host. Used to
/// filter body-text emails on legal pages: a brand mentioning their own
/// `privacy@brand.com` in a privacy notice is real; a CCPA / GDPR notice
/// citing a regulator's `info@uoou.cz` or a parent company's
/// `dpo@parent.com` is NOT the brand's contact. Same-host or subdomain
/// matches qualify (e.g. `support@help.brand.com` matches `brand.com`).
///
/// Returns `true` when no comparison can be made (no `@` in email, no
/// host in URL) — fail-open so we don't accidentally drop everything on
/// pages with weird URL shapes.
fn email_matches_site_host(email: &str, base: &Url) -> bool {
    let lc = email.to_ascii_lowercase();
    let Some((_, domain_raw)) = lc.split_once('@') else {
        return true;
    };
    let domain = domain_raw.trim_start_matches("www.").to_string();
    let Some(host) = base.host_str() else {
        return true;
    };
    let host = host.trim_start_matches("www.").to_lowercase();
    domain == host
        || domain.ends_with(&format!(".{host}"))
        || host.ends_with(&format!(".{domain}"))
}

/// Case-insensitive email dedup. Mejuri smoketest showed `PRESS@MEJURI.COM`
/// and `press@mejuri.com` as two separate entries. We group by the
/// lowercase form, keep the first-seen original case (preferring lowercase
/// when a tie occurs because that's how 95% of mailto: hrefs render).
pub(crate) fn dedup_emails(input: HashSet<String>) -> Vec<String> {
    let mut by_lower: HashMap<String, String> = HashMap::new();
    for e in input {
        let lower = e.to_lowercase();
        match by_lower.get(&lower) {
            None => {
                by_lower.insert(lower, e);
            }
            Some(existing) => {
                // Prefer the variant that is already all-lowercase; it's
                // the canonical form.
                let existing_is_lower = existing.chars().all(|c| !c.is_ascii_uppercase());
                let new_is_lower = e.chars().all(|c| !c.is_ascii_uppercase());
                if new_is_lower && !existing_is_lower {
                    by_lower.insert(lower, e);
                }
            }
        }
    }
    let mut out: Vec<String> = by_lower.into_values().collect();
    out.sort_by_key(|s| s.to_lowercase());
    out
}

/// Dedup a list of phone strings by digit-only form, keeping the best-
/// formatted variant of each unique number.
/// Common country-code prefixes used by sites we've seen in the wild.
/// Ordered longest-first so `+420` matches before `+42` (Czech vs hypothetical).
const COUNTRY_CODES: &[&str] = &[
    "420", "421", "1", "44", "49", "33", "39", "34", "31", "32", "351", "353", "354", "358", "45",
    "46", "47", "48", "30", "36", "40", "43", "41", "352", "356", "357", "359", "370", "371",
    "372", "385", "386", "387",
];

/// Compute a canonical "domestic" digit form for dedup matching: strip the
/// leading country code if present. Examples:
///   `+420 771 231 771` → `771231771`
///   `771 231 771`      → `771231771`
///   `+1 (415) 555 0123` → `4155550123`
///   `415-555-0123`     → `4155550123`
fn canonical_digits(s: &str) -> String {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    // Heuristic: if the original starts with '+', the first 1-3 digits are
    // the country code. Otherwise, only strip a leading country code that
    // matches a known prefix AND leaves a remainder ≥ 7 digits (sanity).
    let plus_form = s.trim().starts_with('+');
    if plus_form {
        // Strip 1-3 leading digits that match a known country code.
        for cc in COUNTRY_CODES {
            if digits.starts_with(cc) && digits.len() - cc.len() >= 7 {
                return digits[cc.len()..].to_string();
            }
        }
        // Unknown country code — strip 1-3 leading digits heuristically.
        if digits.len() >= 11 {
            return digits[1..].to_string();
        }
    } else {
        // Non-plus form: strip a leading "00" + country-code (international
        // dialing pattern used in some EU formats), or a bare country code
        // if it makes the remainder a plausible national 9-10 digit number.
        if let Some(rest) = digits.strip_prefix("00") {
            for cc in COUNTRY_CODES {
                if rest.starts_with(cc) && rest.len() - cc.len() >= 7 {
                    return rest[cc.len()..].to_string();
                }
            }
        }
        // NANP shortcut: a non-plus 11-digit string starting with `1`
        // (e.g. `1-855-496-1110`) is the same as `+1 855-496-1110`.
        // Strip the leading `1` to align with the `+`-form canonical.
        if digits.len() == 11 && digits.starts_with('1') {
            return digits[1..].to_string();
        }
    }
    digits
}

pub(crate) fn dedup_phones(phones: Vec<String>) -> Vec<String> {
    let mut best: HashMap<String, String> = HashMap::new();
    for p in phones {
        let key = canonical_digits(&p);
        if key.is_empty() {
            continue;
        }
        let p_quality = p.chars().filter(|c| !c.is_ascii_digit()).count();
        // Prefer the variant that explicitly includes a country code (`+`)
        // because it's unambiguous internationally. If both variants have
        // `+`, fall back to the one with more separator chars (better
        // formatting).
        match best.get(&key) {
            Some(existing) => {
                let existing_has_plus = existing.trim().starts_with('+');
                let new_has_plus = p.trim().starts_with('+');
                let existing_quality =
                    existing.chars().filter(|c| !c.is_ascii_digit()).count();
                let prefer_new = match (new_has_plus, existing_has_plus) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => p_quality > existing_quality,
                };
                if prefer_new {
                    best.insert(key, p);
                }
            }
            None => {
                best.insert(key, p);
            }
        }
    }
    let mut result: Vec<String> = best.into_values().collect();
    result.sort();
    result
}

/// Social-platform URLs that are clearly share/intent endpoints rather than
/// the company's own profile.
pub(crate) fn is_social_share_url(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("/sharing/share")
        || u.contains("/sharer/")
        || u.contains("/intent/tweet")
        || u.contains("/intent/post")
        || u.contains("?text=")
        || u.contains("&text=")
        || u.contains("/share?")
        || u.contains("share-article")
        || u.contains("/dialog/share")
}

/// Per-page contact extraction. Pulls emails / phones from rendered text
/// only (not raw HTML, which would match SVG path data as phone numbers),
/// reads `tel:` / `mailto:` links directly, finds social-platform URLs in
/// `<a href>` anchors, and extracts addresses from JSON-LD `PostalAddress`.
pub(crate) fn extract_contact(doc: &Html, base: &Url, structured: &[JsonValue]) -> ContactInfo {
    let visible = body_text_only(doc);
    // Emails are extracted from (a) `mailto:` anchors, (b) text inside
    // chrome zones (nav/header/footer/contact/address blocks), AND (c) the
    // full body text BUT ONLY when the URL slug indicates a contact-ish
    // page (`/contact`, `/about`, `/imprint`, `/legal`, `/impressum`,
    // `/kontakt`). Shopify-style sites prefer contact forms over `mailto:`
    // so chrome-only would miss legitimate addresses on those pages; blog
    // post bodies are NOT in this list, so a founder's personal Gmail
    // mentioned in a 2019 post still won't pollute the contact list.
    let chrome = chrome_text(doc);
    let path_lc = base.path().to_ascii_lowercase();
    let is_legal_page = path_lc.contains("/privacy")
        || path_lc.contains("/terms")
        || path_lc.contains("/legal")
        || path_lc.contains("/impressum")
        || path_lc.contains("/imprint")
        || path_lc.contains("/ccpa")
        || path_lc.contains("/gdpr")
        || path_lc.contains("/cookies")
        || path_lc.contains("/obchodni-podminky")
        || path_lc.contains("/zasady");
    let is_contact_page = path_lc.contains("/contact")
        || path_lc.contains("/kontakt")
        || path_lc.contains("/about")
        || path_lc.ends_with("/contact-us")
        || path_lc.ends_with("/get-in-touch");
    // Email scan: include body text on contact OR legal pages (legal
    // pages legitimately list privacy@brand.com etc.); chrome-only
    // elsewhere.
    let email_scan = if is_contact_page || is_legal_page {
        &visible
    } else {
        &chrome
    };

    let mut emails: HashSet<String> = HashSet::new();
    for el in doc.select(&SEL_LINK) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if let Some(rest) = href.strip_prefix("mailto:") {
            let addr_raw = rest.split('?').next().unwrap_or(rest).trim();
            if addr_raw.is_empty() {
                continue;
            }
            // URL-decode `mailto:` payload. M3: `mailto:info%40knihy.cz`
            // used to surface as literal `info%40knihy.cz` (the `%40` is
            // the URL-encoded `@`).
            let addr = percent_decode_email(addr_raw);
            // M5/M6: same-host rule applies to mailto: links too. Sites
            // commonly include `mailto:` for parent-company DPOs, web
            // vendor support, partner law firms etc. — those are NOT
            // the brand's primary contact. Keep only emails at the
            // site's own domain or a subdomain.
            if !addr.is_empty()
                && addr.contains('@')
                && email_matches_site_host(&addr, base)
            {
                emails.insert(addr);
            }
        }
    }
    for m in RE_EMAIL.find_iter(email_scan) {
        let s = m.as_str().to_string();
        if s.ends_with("@2x.png")
            || s.contains("example.com")
            || s.contains("sentry.io")
            || s.contains("@example")
        {
            continue;
        }
        // General rule (M5): body-text emails (anywhere on the site)
        // must match the site's own host. mailto: anchors above are
        // ALWAYS accepted — they're explicit contact intent. Body-text
        // mentions of non-site-domain emails are universally one of:
        //   - regulator contacts in privacy notices (`@uoou.cz`, etc.)
        //   - parent-company DPO / press contacts
        //   - web vendor / hosting partner contacts
        //   - third-party agency / partner emails
        // None of these are the brand's primary contact. The agent can
        // find them via the page bodies if needed; the `contact.emails`
        // list is reserved for the brand's own addresses.
        if !email_matches_site_host(&s, base) {
            continue;
        }
        emails.insert(s);
    }

    let mut phones: HashSet<String> = HashSet::new();
    for el in doc.select(&SEL_LINK) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };
        if let Some(rest) = href.strip_prefix("tel:") {
            let phone = balance_phone_parens(rest);
            if looks_like_phone(&phone) {
                phones.insert(phone);
            }
        }
    }
    // General rule (M4): on legal pages, suppress body-text phone scans
    // entirely. `tel:` anchors above are still honored, so a brand that
    // genuinely puts a `tel:+1234567890` on its privacy page still gets
    // the phone captured. Body-text on legal pages is full of regulator
    // contacts (CA AG, FTC, EU DPAs etc.) cited verbatim in CCPA / GDPR
    // notices — those are not the brand's contact info. Sites that bury
    // their customer-service number in body text of their privacy page
    // alone will be missed; that's a trade-off for not hardcoding any
    // specific number list.
    if !is_legal_page {
        for m in RE_PHONE.find_iter(&visible) {
            let raw = balance_phone_parens(m.as_str());
            if looks_like_phone(&raw) {
                phones.insert(raw);
            }
        }
    }

    // Restrict social-profile detection to chrome zones (nav / header /
    // footer / [role='navigation'|'banner'|'contentinfo']). User-submitted
    // content in body text (e.g. Hacker News posts) shouldn't pollute the
    // brand's social profile list.
    let mut socials: Vec<SocialLink> = Vec::new();
    let mut seen_social: HashSet<(String, String)> = HashSet::new();
    let mut chrome_links: Vec<String> = Vec::new();
    for nav_el in doc.select(&SEL_NAV) {
        for el in nav_el.select(&SEL_LINK) {
            if let Some(h) = el.value().attr("href") {
                chrome_links.push(h.to_string());
            }
        }
    }
    for footer_el in doc.select(&SEL_FOOTER) {
        for el in footer_el.select(&SEL_LINK) {
            if let Some(h) = el.value().attr("href") {
                chrome_links.push(h.to_string());
            }
        }
    }
    for href in &chrome_links {
        if is_social_share_url(href) {
            continue;
        }
        let Ok(abs) = base.join(href) else { continue };
        let Some(host) = abs.host_str() else { continue };
        let host_lc = host.to_lowercase();
        for (platform, domain) in SOCIAL_DOMAINS {
            if host_lc == *domain || host_lc.ends_with(&format!(".{domain}")) {
                let url = abs.to_string();
                if seen_social.insert(((*platform).to_string(), url.clone())) {
                    socials.push(SocialLink {
                        platform: (*platform).to_string(),
                        url,
                    });
                }
                break;
            }
        }
    }

    let mut addresses = Vec::new();
    let mut organization: Option<JsonValue> = None;
    fn walk_addr(value: &JsonValue, out_addrs: &mut Vec<String>, out_org: &mut Option<JsonValue>) {
        if let Some(obj) = value.as_object() {
            if let Some(t) = obj.get("@type").and_then(|v| v.as_str()) {
                if t.eq_ignore_ascii_case("PostalAddress") {
                    let parts: Vec<String> = [
                        "streetAddress",
                        "postOfficeBoxNumber",
                        "addressLocality",
                        "postalCode",
                        "addressRegion",
                        "addressCountry",
                    ]
                    .iter()
                    .filter_map(|k| obj.get(*k).and_then(|v| v.as_str()).map(|s| s.to_string()))
                    .collect();
                    if !parts.is_empty() {
                        out_addrs.push(parts.join(", "));
                    }
                }
                if (t.eq_ignore_ascii_case("Organization")
                    || t.eq_ignore_ascii_case("LocalBusiness"))
                    && out_org.is_none()
                {
                    *out_org = Some(value.clone());
                }
            }
            for v in obj.values() {
                walk_addr(v, out_addrs, out_org);
            }
        }
        if let Some(arr) = value.as_array() {
            for v in arr {
                walk_addr(v, out_addrs, out_org);
            }
        }
    }
    for v in structured {
        walk_addr(v, &mut addresses, &mut organization);
    }

    // Case-insensitive email dedup: Mejuri leaked both `PRESS@MEJURI.COM`
    // and `press@mejuri.com` as separate entries. Group by lowercase,
    // keep the first-seen original case for display.
    let emails = dedup_emails(emails.into_iter().collect());
    let phones = dedup_phones(phones.into_iter().collect());
    addresses.sort();
    addresses.dedup();

    ContactInfo {
        emails,
        phones,
        social_links: socials,
        addresses,
        organization,
        // Per-page extraction doesn't track form endpoints — they're
        // collected at the aggregate level in `output::aggregate_contact`
        // where we have access to `page.content_blocks`.
        contact_form_endpoints: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_validator_accepts_common_shapes() {
        assert!(looks_like_phone("+420 604 550 936"));
        assert!(looks_like_phone("+1 (415) 555 0123"));
        assert!(looks_like_phone("+44 20 7946 0958"));
        assert!(looks_like_phone("604-550-936"));
        // Regression: Stumptown / Death & Co's real US 10-digit number.
        // Was being dropped after Round F — Round H regex change ensures it
        // survives single-line extraction.
        assert!(looks_like_phone("503-808-9080"));
        assert!(looks_like_phone("(720) 330-2660"));
    }

    #[test]
    fn phone_validator_rejects_too_many_groups() {
        // kotn.com leaked "29 30 31 32 34 36" (store-hours calendar, 6 digit groups).
        assert!(!looks_like_phone("29 30 31 32 34 36"));
        assert!(!looks_like_phone("1 2 3 4 5 6 7"));
    }

    #[test]
    fn balance_phone_parens_repairs_orphan_close() {
        // Brooklyn Brewery / Kotn extracted phones missing the leading `(`.
        assert_eq!(balance_phone_parens("718) 486-7422"), "(718) 486-7422");
        assert_eq!(balance_phone_parens("800) 952-5210"), "(800) 952-5210");
        assert_eq!(balance_phone_parens("  916) 445-1254  "), "(916) 445-1254");
        // Already balanced → unchanged.
        assert_eq!(balance_phone_parens("(415) 555-0123"), "(415) 555-0123");
        assert_eq!(balance_phone_parens("+420 604 550 936"), "+420 604 550 936");
        // Orphan trailing `(`.
        assert_eq!(balance_phone_parens("415-555-0123 ("), "415-555-0123");
    }

    #[test]
    fn phone_validator_rejects_non_phones() {
        assert!(!looks_like_phone("2025-05-13"));
        assert!(!looks_like_phone("2 2 0 0 0 .586 1.414"));
        assert!(!looks_like_phone("0123456789"));
        assert!(!looks_like_phone("123 456"));
        assert!(!looks_like_phone("1 2 3 4 5 6 7 8 9"));
        // Regression: Hacker News thread IDs like 2026-45185 (year + thread id)
        // used to match the phone regex. Year-prefix filter rejects them.
        assert!(!looks_like_phone("2026-45185"));
        assert!(!looks_like_phone("1999-12345"));
        assert!(!looks_like_phone("2024 567890"));
        // Regression: Plausible bug — multi-space + dot mixed separators
        // (technical version-like string) matched as a phone.
        assert!(!looks_like_phone("059   301   71.58"));
        // Real French dot-separated phone still works (consistent separator).
        assert!(looks_like_phone("06.12.34.56.78"));
        // Stumptown Shopify product card: timestamp + newline + day number.
        assert!(!looks_like_phone("1762296503 \n       3"));
        assert!(!looks_like_phone("1771349142\n12"));
        // Unix timestamp alone (10 digits starting 16/17/18 — years 2020-2030).
        assert!(!looks_like_phone("1762296503"));
        assert!(!looks_like_phone("1700000000"));
        // Buckmason regression: 8-digit + 1-digit shape (product SKU).
        assert!(!looks_like_phone("11301667 3"));
        assert!(!looks_like_phone("123456789 1"));
        assert!(!looks_like_phone("+1 4155550123 9")); // trailing single
        // Developer placeholder pattern: 9-char window of 3 consecutive
        // mono-digit triples. Catches both formatted and compact forms
        // (Elementor / page-builder demo sites use both).
        assert!(!looks_like_phone("+420 111 222 333"));
        assert!(!looks_like_phone("+420111222333"));
        assert!(!looks_like_phone("123 111 222 333"));
        assert!(!looks_like_phone("+1 555 555 5555"));
        assert!(!looks_like_phone("+1 (555) 555-5555"));
        // Real phones with one mono-digit group are still fine.
        assert!(looks_like_phone("+420 602 308 333")); // one mono group, others varied
        assert!(looks_like_phone("+1 800 222 7890")); // 800 group plus varied
    }

    #[test]
    fn percent_decode_email_handles_mailto_escapes() {
        // knihy.cz regression: `mailto:info%40knihy.cz` → `info@knihy.cz`
        assert_eq!(percent_decode_email("info%40knihy.cz"), "info@knihy.cz");
        assert_eq!(percent_decode_email("info@knihy.cz"), "info@knihy.cz");
        assert_eq!(
            percent_decode_email("foo%2Bbar%40example.com"),
            "foo+bar@example.com"
        );
        assert_eq!(percent_decode_email("Hello%2Eworld%40x.com"), "Hello.world@x.com");
        // Spaces in emails are invalid — strip them rather than encode.
        assert_eq!(percent_decode_email("foo%20bar%40x.com"), "foobar@x.com");
        // Unknown escapes pass through.
        assert_eq!(percent_decode_email("a%99b@x.com"), "a%99b@x.com");
        // Plain string unchanged.
        assert_eq!(percent_decode_email("hello@world.com"), "hello@world.com");
    }

    #[test]
    fn email_matches_site_host_general_rule() {
        let base = Url::parse("https://www.brand.com").unwrap();
        // Same-domain emails match.
        assert!(email_matches_site_host("info@brand.com", &base));
        assert!(email_matches_site_host("INFO@BRAND.COM", &base));
        assert!(email_matches_site_host("support@help.brand.com", &base));
        // Different-domain emails (regulator / parent / partner) do NOT.
        assert!(!email_matches_site_host("info@uoou.cz", &base));
        assert!(!email_matches_site_host("dpo@parent.com", &base));
        assert!(!email_matches_site_host("info@partner.com", &base));
        // Fail-open on weird inputs (no @ etc).
        assert!(email_matches_site_host("not-an-email", &base));
    }

    #[test]
    fn dedup_emails_case_insensitive() {
        // Mejuri regression: PRESS@MEJURI.COM + press@mejuri.com were
        // emitted as two separate emails. Round K folds them to one.
        let input: HashSet<String> = [
            "PRESS@MEJURI.COM",
            "press@mejuri.com",
            "Contact@Mejuri.com",
            "support@buttondown.com",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let result = dedup_emails(input);
        assert_eq!(result.len(), 3);
        // Lowercase variant should win when both exist.
        assert!(result.contains(&"press@mejuri.com".to_string()));
        // Mixed-case kept when no lowercase variant exists.
        assert!(result.iter().any(|e| e.to_lowercase() == "contact@mejuri.com"));
        assert!(result.contains(&"support@buttondown.com".to_string()));
    }

    #[test]
    fn dedup_phones_keeps_best_formatted() {
        let input = vec![
            "+420604550936".to_string(),
            "+420 604 550 936".to_string(),
            "604550936".to_string(),
        ];
        let result = dedup_phones(input);
        // Round L: country-code-aware dedup folds all three to ONE entry
        // (same domestic number `604550936`). The variant with `+` country
        // code wins because it's internationally unambiguous; among the
        // two `+` variants the better-formatted one wins.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "+420 604 550 936");
    }

    #[test]
    fn dedup_phones_folds_country_code_variants() {
        // Round L regression: Rohlik emitted both `+420771231771` and
        // `771 231 771` as separate phones.
        let input = vec![
            "+420771231771".to_string(),
            "771 231 771".to_string(),
        ];
        let result = dedup_phones(input);
        assert_eq!(result.len(), 1, "got {result:?}");
        // The `+`-prefixed variant should win (more useful for the agent).
        assert_eq!(result[0], "+420771231771");

        // US: `+1 (415) 555 0123` and `(415) 555-0123`.
        let us = vec![
            "+1 (415) 555 0123".to_string(),
            "(415) 555-0123".to_string(),
        ];
        let us_result = dedup_phones(us);
        assert_eq!(us_result.len(), 1, "got {us_result:?}");
        assert_eq!(us_result[0], "+1 (415) 555 0123");
    }

    #[test]
    fn is_social_share_url_filters_share_endpoints() {
        assert!(is_social_share_url(
            "https://twitter.com/intent/tweet?url=https%3A%2F%2Fexample.com"
        ));
        assert!(is_social_share_url(
            "https://www.facebook.com/sharer/sharer.php?u=https%3A%2F%2Fexample.com"
        ));
        assert!(is_social_share_url(
            "https://www.linkedin.com/sharing/share-offsite/?url=..."
        ));
        assert!(!is_social_share_url("https://twitter.com/anthropicai"));
        assert!(!is_social_share_url(
            "https://www.linkedin.com/company/anthropic"
        ));
    }
}
