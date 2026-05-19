use regex::Regex;
use scraper::Selector;
use std::sync::LazyLock;

macro_rules! sel {
    ($name:ident, $css:expr) => {
        pub(crate) static $name: LazyLock<Selector> = LazyLock::new(|| {
            Selector::parse($css).expect(concat!("invalid built-in selector: ", $css))
        });
    };
}

sel!(SEL_LOC, "loc");
sel!(SEL_BODY, "body");
sel!(SEL_TITLE, "title");
sel!(SEL_META, "meta");
sel!(SEL_LI, "li");
sel!(SEL_INPUT, "input, textarea, select");
sel!(SEL_OPTION, "option");
sel!(
    SEL_SUBMIT,
    "button[type='submit'], input[type='submit'], button:not([type])"
);
sel!(SEL_LINK, "a[href]");
sel!(SEL_HTML, "html");
sel!(SEL_MAIN, "main, article, [role='main']");
sel!(SEL_NAV, "nav, header, [role='navigation'], [role='banner']");
sel!(SEL_FOOTER, "footer, [role='contentinfo']");
sel!(SEL_CANONICAL, "link[rel='canonical']");
sel!(
    SEL_FAVICON,
    "link[rel~='icon'], link[rel='apple-touch-icon']"
);
sel!(SEL_JSONLD, r#"script[type="application/ld+json"]"#);
sel!(
    SEL_HEADER_IMG,
    "header img, header svg, [class*='logo'] img, [class*='logo'] svg, [id*='logo'] img, [id*='logo'] svg, a[aria-label*='home' i] img, a[href='/'] img"
);
sel!(SEL_STYLE_BLOCK, "style");
sel!(SEL_STYLESHEET, "link[rel='stylesheet']");
sel!(SEL_HREFLANG, "link[rel='alternate'][hreflang]");
sel!(SEL_TR, "tr");
sel!(SEL_TH, "th");
sel!(SEL_TD, "td");
sel!(SEL_CAPTION, "caption");
sel!(SEL_FIGCAPTION, "figcaption");
sel!(SEL_CODE_INSIDE_PRE, "code");
sel!(SEL_VIDEO_SOURCE, "source");
sel!(SEL_DT, "dt");
sel!(SEL_DD, "dd");
sel!(
    SEL_SKIP,
    "nav, header, footer, [role='navigation'], [role='banner'], [role='contentinfo'], \
     script, style, noscript, [aria-hidden='true'], \
     .swiper-slide-duplicate, .swiper-slide-duplicate-active, .slick-cloned"
);

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (compatible; DumpIt/0.1)";

pub(crate) static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b").expect("invalid email regex")
});
// Phone regex — uses a LITERAL space inside the char class instead of `\s`
// so matches don't extend across newlines / tabs. Real phone numbers always
// live on a single rendered line; allowing `\s` previously combined a real
// phone with adjacent timestamps and product IDs (e.g. Shopify product
// cards: "1762296503\n3" eats the line break and produces an 11-digit
// "phone").
pub(crate) static RE_PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+?\d[\d ().\-]{7,}\d").expect("invalid phone regex"));
pub(crate) static RE_COLOR_HEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#([0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3})\b").expect("hex re")
});
pub(crate) static RE_COLOR_RGB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)rgba?\([0-9]+(?:\.[0-9]+)?(?:\s*,\s*[0-9]+(?:\.[0-9]+)?){2,3}\s*\)")
        .expect("rgb re")
});
pub(crate) static RE_COLOR_HSL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)hsla?\([0-9]+(?:\.[0-9]+)?(?:\s*,?\s*[0-9]+(?:\.[0-9]+)?%?){2,3}\s*\)")
        .expect("hsl re")
});
pub(crate) static RE_FONT_FAMILY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)font-family\s*:\s*([^;}\n"]+|"[^"]+"|'[^']+')"#).expect("font-family re")
});
pub(crate) static RE_CSS_VAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"--([a-zA-Z][a-zA-Z0-9_\-]*)\s*:\s*([^;}\n]+?)\s*[;}]").expect("css var re")
});
pub(crate) static RE_LOOKS_LIKE_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}[-./]\d{1,2}[-./]\d{1,2}").expect("date re"));

pub(crate) const SOCIAL_DOMAINS: &[(&str, &str)] = &[
    ("facebook", "facebook.com"),
    ("instagram", "instagram.com"),
    ("twitter", "twitter.com"),
    ("x", "x.com"),
    ("linkedin", "linkedin.com"),
    ("youtube", "youtube.com"),
    ("tiktok", "tiktok.com"),
    ("pinterest", "pinterest.com"),
    ("snapchat", "snapchat.com"),
    ("github", "github.com"),
    ("vimeo", "vimeo.com"),
    ("threads", "threads.net"),
    ("bluesky", "bsky.app"),
    ("mastodon", "mastodon.social"),
    ("medium", "medium.com"),
    ("dribbble", "dribbble.com"),
    ("behance", "behance.net"),
];

pub(crate) const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
    "/wp-admin/",
    "/wp-login",
    "/wp-json/",
    "/jkit-",
    "/elementor-",
    "elementor_library=",
    "?elementor",
    "/author/",
    "/category/",
    "/tag/",
    "/feed/",
    "/feed",
    "/cart/",
    "/checkout/",
    "/my-account/",
    "?p=",
];
