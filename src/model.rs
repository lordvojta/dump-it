use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct FormField {
    pub field_type: String,
    pub name: String,
    pub label: String,
    pub placeholder: String,
    pub required: bool,
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum ContentBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    Image {
        original_url: String,
        local_path: String,
        alt_text: String,
    },
    List {
        items: Vec<String>,
    },
    Form {
        action: String,
        method: String,
        fields: Vec<FormField>,
        submit_text: String,
        /// Heuristic classification: contact / newsletter / search / login /
        /// signup / payment / comment / generic. Inferred from field names,
        /// types, placeholders.
        #[serde(default)]
        purpose: String,
    },
    /// `<iframe>` + common video embeds (YouTube, Vimeo, Maps).
    /// `provider` is the recognised platform or `iframe` fallback.
    Embed {
        provider: String,
        src: String,
        title: String,
    },
    /// HTML `<table>` with structured rows + optional column headers.
    /// Captures classic table-based layouts (Hacker News, Wikipedia,
    /// pricing tables, comparison grids) that would otherwise produce
    /// empty content_blocks.
    Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Code block (`<pre>` or `<pre><code>`). `language` is best-effort from
    /// `class="language-rust"` / `class="hljs rust"` etc. Common on docs +
    /// engineering blogs; previously dropped as opaque text.
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        text: String,
    },
    /// Block-level quotation (`<blockquote>`). Used for testimonials,
    /// callouts, pull quotes in articles, "important" admonitions on docs.
    Quote {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cite: Option<String>,
    },
    /// `<video>` or `<audio>` element. `src` is the best-resolution `<source>`
    /// or direct `src` attribute. `kind` is `"video"` or `"audio"`.
    Media {
        kind: String,
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poster: Option<String>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
    },
    /// `<dl>` definition list — key/value pairs that aren't a table and
    /// aren't a heading/paragraph sequence.
    DefinitionList {
        items: Vec<DefinitionItem>,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct DefinitionItem {
    pub term: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct NavLink {
    pub text: String,
    pub href: String,
    /// Sub-menu blurb when the anchor wraps a heading + paragraph
    /// (mega-menus). Lets the agent rebuild the menu with proper title vs
    /// description structure instead of a run-on `text` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// "header" — primary nav inside <header>; "mega_menu" — nested
    /// drop-down item with a description; "utility" — login/pricing/search
    /// type links; "social" — points at a known social-profile domain.
    /// Lets the agent reconstruct chrome instead of dumping everything as
    /// a flat menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct HreflangAlternate {
    pub lang: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct PageSection {
    pub section_type: String, // "hero" | "features" | "cta" | "testimonials" | "team" | "faq" | "content"
    pub block_start: usize,
    pub block_end: usize, // exclusive
    pub summary: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct PageTemplate {
    pub template_id: String,
    pub block_pattern: Vec<String>,
    pub page_count: usize,
    pub pages: Vec<String>,
    pub sample_page: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct PageData {
    pub url: String,
    pub title: String,
    pub meta_title: String,
    pub meta_description: String,
    pub canonical_url: Option<String>,
    pub language: Option<String>,
    pub favicon_url: Option<String>,
    pub logo_url: Option<String>,
    #[serde(default)]
    pub og_image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub og_image_local_path: Option<String>,
    #[serde(default)]
    pub twitter_card: Option<String>,
    /// Raw `<meta name="robots" content="...">` value. e.g. "noindex,nofollow".
    /// Detected at extraction time; rolled up into the quality_flag
    /// `meta_robots_noindex` when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_robots: Option<String>,
    #[serde(default)]
    pub hreflang_alternates: Vec<HreflangAlternate>,
    pub nav_links: Vec<NavLink>,
    pub footer_blocks: Vec<ContentBlock>,
    pub structured_data: Vec<JsonValue>,
    pub content_blocks: Vec<ContentBlock>,
    /// Concatenated text of every heading/paragraph/list-item block.
    /// Useful for full-text search and cheap LLM context.
    #[serde(default)]
    pub plain_text: String,
    /// SHA-256 hex (first 16 chars) of the page's plain_text — lets the
    /// agent spot boilerplate / duplicate content across pages and detect
    /// changes between runs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
    /// Rough LLM-token estimate (plain_text chars / 4) so an agent can
    /// budget its context window.
    #[serde(default)]
    pub token_estimate: usize,
    /// One-line auto-summary built from title + meta_description + first
    /// h1 — agent-friendly preview that lives in `index.md` and
    /// `compact.json` without opening the full page.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    /// Local paths of every asset this page references — content images,
    /// inline SVGs, og:image. Lets the agent rebuild this single page
    /// without scanning the whole bundle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_assets: Vec<String>,
    /// Heuristic-inferred sections over `content_blocks` — gives the agent
    /// "this is a hero, that's a features grid, that's a CTA" hints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<PageSection>,
    /// SEO / accessibility flags ("no_h1", "no_meta_description",
    /// "images_missing_alt:3", "thin_content", "title_too_long", …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_flags: Vec<String>,
    pub total_words: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_contact: Option<ContactInfo>,
    /// Internal anchor hrefs (resolved to absolute) pointing at pages on the
    /// same site. Used to build the link graph aggregate.
    #[serde(default)]
    pub internal_links_out: Vec<String>,
    /// Concatenated <style> block text — used post-scrape to mine colors
    /// and fonts. Not serialised (skipped) to keep page JSON readable.
    #[serde(skip)]
    pub style_text: String,
    /// URLs of <link rel="stylesheet"> elements (skipped from JSON;
    /// fetched separately during brand aggregation).
    #[serde(skip)]
    pub stylesheet_urls: Vec<String>,
    /// Screenshot relative paths if --screenshots was enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_desktop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_mobile: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ScrapedData {
    pub total_pages: usize,
    pub pages: Vec<PageData>,
}

#[derive(Serialize, Clone)]
pub(crate) struct PageSummary {
    pub url: String,
    pub title: String,
    pub meta_description: String,
    pub category: String,
    pub word_count: usize,
    pub block_count: usize,
    pub image_count: usize,
    pub has_form: bool,
    pub primary_heading: Option<String>,
    pub file: Option<String>,
    pub markdown_file: Option<String>,
    pub screenshot_desktop: Option<String>,
    pub screenshot_mobile: Option<String>,
    pub internal_links_out: usize,
    pub internal_links_in: usize,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct SocialLink {
    pub platform: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub(crate) struct ContactInfo {
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub social_links: Vec<SocialLink>,
    pub addresses: Vec<String>,
    pub organization: Option<JsonValue>,
    /// Contact-form endpoints (the `action` URLs of forms classified as
    /// `contact`). Surfaced when no `mailto:` / `tel:` links exist on the
    /// site — common on EU / Czech sites that prefer forms to email for
    /// anti-spam reasons. Agent can wire its rebuilt contact form to POST
    /// at the same endpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contact_form_endpoints: Vec<String>,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct ColorUse {
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct FontUse {
    pub family: String,
    pub count: usize,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct CssVariable {
    pub name: String,
    pub value: String,
    pub count: usize,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct WebfontUrl {
    pub provider: String,
    pub families: Vec<String>,
    pub url: String,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct BrandPalette {
    pub colors: Vec<ColorUse>,
    pub fonts: Vec<FontUse>,
    pub css_variables: Vec<CssVariable>,
    pub webfont_urls: Vec<WebfontUrl>,
    pub logo_url: Option<String>,
    pub logo_local_path: Option<String>,
    pub favicon_url: Option<String>,
    pub favicon_local_path: Option<String>,
    /// Confidence in the brand palette: "high" (top color+font counts >=
    /// 20), "medium" (>= 5), "low" (< 5). Next.js / Tailwind CSS-in-JS
    /// sites typically end up at "low" because most styling lives in
    /// runtime-generated stylesheets that our static scan can't see —
    /// the agent should treat top colors / fonts as a hint, not gospel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

#[derive(Serialize, Default, Clone)]
pub(crate) struct FrameworkHint {
    pub framework: String,
    pub confidence: String, // "high" | "medium" | "low"
    pub evidence: Vec<String>,
}

#[derive(Serialize, Clone)]
pub(crate) struct AssetEntry {
    pub path: String,
    pub size_bytes: u64,
    pub kind: String, // "image" | "favicon" | "logo" | "svg" | "screenshot" | "stylesheet"
}

#[derive(Serialize, Clone)]
pub(crate) struct HreflangGroup {
    pub lang: String,
    pub urls: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct SiteData {
    pub base_url: String,
    pub language: Option<String>,
    pub frameworks: Vec<FrameworkHint>,
    pub primary_nav: Vec<NavLink>,
    pub footer_blocks: Vec<ContentBlock>,
    pub contact: ContactInfo,
    pub brand: BrandPalette,
    pub templates: Vec<PageTemplate>,
    pub hreflang_groups: Vec<HreflangGroup>,
    pub sitemap: Vec<PageSummary>,
    pub total_pages: usize,
    pub assets: Vec<AssetEntry>,
    /// Synthetic-probe pages — currently just the 404 template if
    /// `--capture-404` was set. Kept separate from `sitemap` so the agent
    /// doesn't mistake them for real site pages.
    pub error_pages: Vec<PageData>,
    pub output_files: Vec<String>,
    /// Bundle-level quality warnings — `spa_loading_shell` (JS-rendered
    /// site captured before hydration), `partial_scrape_bot_protected`
    /// (most pages blocked by WAF), etc. Rendered at the top of
    /// `index.md` so the agent doesn't trust the bundle blindly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quality_warnings: Vec<String>,
    /// Counts of skipped pages by reason — e.g. `("bot_protected", 9)`.
    /// Pairs with `partial_scrape_bot_protected` warning above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_pages: Vec<SkippedPage>,
}

#[derive(Serialize, Clone)]
pub(crate) struct SkippedPage {
    pub url: String,
    /// "bot_protected" | "render_failed" | "http_error" | "robots_disallow".
    pub reason: String,
}
