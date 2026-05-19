# dump-it

High-performance website scraper built with Rust. Intelligently scrapes websites using sitemaps or automatic crawling, then emits a structured multi-file bundle (JSON + Markdown + images + optional screenshots) you can hand straight to a coding agent to rebuild or migrate the site.

## Purpose

`dump-it` is primarily a **content-capture tool for redesign and migration workflows**. The output JSON is designed to be passed directly to an LLM coding agent (Claude Code, Cursor, etc.) so the agent has everything it needs — copy, images, forms, metadata — to rebuild the site on a new stack.

## Features

- **JavaScript rendering** — Headless Chrome captures SPA / React / Vue / Angular / Elementor / Webflow content
- **Sitemap + crawler fallback** — Recursive sitemap-index with cycle guard; falls back to a domain-bounded crawler
- **Multi-file output bundle** (every run):
  - `scraped.json` — master file, every page with full content blocks
  - `site.json` — aggregated chrome: nav, footer, brand, contact, sitemap, **frameworks**, **assets**, **link graph**
  - `contact.json` — emails, phones, socials, addresses, Organization schema
  - `brand.json` — favicon, logo, color palette, fonts, **CSS variables**, **webfont URLs**
  - `index.md` — human-readable entry point — start here when handing the folder to an agent
  - `compact.json` — stripped-down version (~40 KB on a 40-page site) that fits in any LLM context window
  - Optional `pages/<slug>.json` (`--split-pages`), `markdown/<slug>.md` (`--markdown`), `screenshots/<slug>.{desktop,mobile}.png` (`--screenshots`)
- **Navigation + footer extraction** — emitted as dedicated top-level fields so an agent doesn't have to guess the chrome
- **Structured data capture** — All `application/ld+json` blobs plus canonical URL, `<html lang>`, favicon, **`og:image`**, **`twitter:card`**
- **Framework detection** — Auto-identifies Next.js, Astro, Hugo, Gatsby, Nuxt/Vue, SvelteKit, React, **Vite**, **Remix**, **Solid.js**, **Qwik**, **Phoenix LiveView**, **Rails (Hotwire)**, **Django**, **Laravel**, **Jekyll**, **Eleventy**, WordPress, Elementor, Webflow, Squarespace, **Shopify** (CDN + `Shopify.shop` global + monorail signals), Tailwind CSS, plus a `<meta name="generator">` catch-all. WordPress detection is multi-signal-corroborated (requires `/wp-content/` AND ≥1 of `/wp-json/`, wp-admin, wp-emoji, generator meta) so 3rd-party widgets don't false-positive.
- **Section inference per page** — Heuristic groups `content_blocks` into `hero` / `features` / `team` / `cta` / `embed` / `content` spans so the agent rebuilds with the right components instead of a flat block list
- **Template-page grouping** — Pages with the same block-pattern signature (e.g. 9 team-member profile pages all matching `[img, h1]`) collapse into one `PageTemplate` entry in `site.json:templates`. The agent rebuilds one component + binds N records, not N near-identical pages.
- **Per-page SEO / accessibility quality flags** — `no_h1`, `multiple_h1`, `no_meta_description`, `meta_description_too_long/short`, `title_too_long`, `no_canonical`, `images_missing_alt:N`, `thin_content` — rolled up in `index.md`, detailed per page in `scraped.json`
- **Hreflang alternates** — `<link rel="alternate" hreflang>` captured per page for multilingual sites
- **Open Graph image downloaded** — alongside favicon and logo, with `og_image_local_path` per page
- **Content-Type sniffing** — favicon/logo extension determined from the response header, not just the URL (fixes `_next/image?url=...` and similar proxy URLs)
- **`--js-wait-selector <css>`** — wait for a meaningful element instead of a fixed wall-clock sleep; falls back to `--js-wait` if the selector never appears
- **`--no-js` static fast path** — skip Chrome entirely and use plain reqwest. Roughly **50× faster** on static sites that don't need JS rendering (Hugo, Jekyll, Astro static output, plain HTML)
- **`robots.txt` respected by default** — `/robots.txt` is fetched at start and Disallow rules for `*` and `DumpIt` are honoured. Opt out with `--ignore-robots`.
- **`--capture-404`** — probes a synthetic non-existent URL and stores the site's 404 template under `site.json:error_pages`
- **Retry-with-backoff** on transient HTTP failures (5xx + connect/timeout) for image / favicon / logo / og:image / external-CSS fetches. 200ms → 600ms → 1800ms backoff.
- **Parallel screenshots** — when `--screenshots` is set, capture runs at `--concurrency` instead of sequentially
- **Brand palette + fonts + CSS variables** — Mines inline `<style>` blocks **and external stylesheets** for hex/rgb/hsl colors, `font-family` declarations, and `--custom-property` definitions; ranked by frequency. External-CSS fetch is on by default — disable with `--no-fetch-css`
- **Webfont CDN detection** — Picks up Google Fonts / Adobe Fonts / Bunny Fonts URLs and parses out the loaded families
- **Logo + favicon download** — Detected via header / `[class*=logo]` / `Organization` JSON-LD; downloaded with content-type aware extension detection
- **Inline SVG capture** — `<svg>` markup saved as standalone `.svg` files for agent re-use as icons
- **Screenshots (`--screenshots`)** — Full-page captures at 1280×800 (desktop) + 390×844 (mobile) for visual ground-truth
- **Markdown export (`--markdown`)** — Per-page Markdown rendering, ideal for LLM ingestion
- **Asset manifest** — Flat list of every file produced with size and kind, in `site.json:assets`
- **Link graph** — Each page exposes `internal_links_out`; site summary records `internal_links_in` so the agent reconstructs the IA
- **Contact extraction** — Emails (text + `mailto:`), phones (text + `tel:`, strict SVG-path filter + date filter + digit-form dedup), social profiles (16 platforms, parses URL host with subdomain awareness so platform suffixes don't false-match unrelated domains)
- **Page categorisation** — `home` / `contact` / `about` / `legal` / `blog-index` / `blog-post` / `service` / `pricing` / `case-study` / `page` via URL + heading heuristics
- **`<picture>` / `<source>` aware** — Picks the highest-resolution `srcset` candidate
- **iframe + embed capture** — YouTube, Vimeo, Maps, Spotify, Soundcloud, Calendly, Typeform, HubSpot
- **Form extraction** — Fields, labels, types, options, submit buttons; resolves `action` to absolute URL
- **Smart filtering** — Skips tracking pixels, tiny images, analytics scripts, JS-slider clones (`aria-hidden`, Swiper, Slick) and built-in URL exclude patterns
- **Concurrent + safe** — Multiple pages render in parallel under a semaphore; Chrome tabs are explicitly closed
- **Bundle quality warnings** (top-of-`index.md`):
  - **SPA loading-shell detection** — when ≥80% of pages share a tiny (<5-block) template (typical of a JS-rendered SPA captured before hydration), a `⚠️ SPA loading shell suspected` banner fires with a `--js-wait-selector` recovery hint
  - **Partial-scrape banner** — when ≥50% of attempted pages were bot-protected or render-failed, the bundle prepends `⚠️ Partial scrape — N/M pages blocked`
  - **Cross-domain sitemap warning** — when ≥50% of sitemap URLs point at a foreign host (typical of merger / acquisition redirects), prepends `⚠️ Cross-domain sitemap` naming the foreign host
  - **Empty-bundle failure banner** — when `total_pages == 0`, prepends a `❌ Scrape failed` banner with the 5 likely causes (WAF, DNS, JS-timeout, robots-Disallow-all, XML/TXT-only sitemap)
  - **Crash-survival placeholder** — `index.md` is written immediately after argument parsing as a "scrape did not complete" placeholder. The real index overwrites on success; if Chrome crashes mid-flight, the placeholder stays so the user never sees a missing/empty folder
- **`parked_domain` quality flag** — Single-iframe body to a parked-domain provider (rapidresultsearch, sedoparking, afternic, bodis, dan.com) is detected and flagged so the agent doesn't waste effort rebuilding a dead site
- **Brand confidence** (`site.json:brand.confidence`) — `low` / `medium` / `high` derived from top-color and top-font sample counts. Surfaced in `index.md` with a "verify against screenshots" hint when low/medium — Next.js / Tailwind CSS-in-JS sites typically land at `low` because runtime-generated styles don't show up in static scans
- **Mega-menu nav split** — `<a>` anchors that wrap a heading + paragraph (common mega-menu pattern) are split into `text` (the heading) + `description` (the paragraph) + `role` (`header` / `mega_menu` / `utility` / `social` / `footer`). Agent rebuilds proper menu structure instead of run-on label text.
- **General same-host email rule** — All emails (both `mailto:` anchors and body-text matches) must have a domain matching the site's own host or a subdomain. Filters out regulator contacts (CCPA / GDPR notices citing CA AG, FTC, EU DPAs etc.), parent-company / partner / vendor emails universally without any hardcoded denylist. The rule applies the same way to every site.
- **Body-text phones suppressed on legal pages** — `tel:` anchors still honored. Avoids regulatory phone numbers commonly cited in CCPA / GDPR notices appearing in the brand's `contact.phones` list.
- **Country-code-aware phone dedup** — `+420771231771` and `771 231 771` (E.164 + national) fold to a single entry. 30+ country codes recognized. `+`-prefixed variant wins.
- **Case-insensitive email dedup** — `PRESS@MEJURI.COM` and `press@mejuri.com` collapse to one entry; lowercase variant preferred.
- **Contact-form endpoint extraction** — When a site has no `mailto:` / `tel:` (form-only contact UX, common in EU / Czech sites), the `action` URL of every `<form>` classified as `contact` is surfaced in `contact.json:contact_form_endpoints` and noted in `index.md`. Agent's rebuild can POST to the same URL.
- **Skipped-page log** (`site.json:skipped_pages`) — Per-URL list of pages that failed to render (`bot_protected` / `render_failed`) so the agent knows what wasn't captured.

## Prerequisites

- **Rust** 1.75+ (`rustup install stable`)
- **Google Chrome / Chromium** installed locally — required at runtime for the JavaScript-rendering pipeline (`headless_chrome` crate launches a real browser via the DevTools Protocol). If you can run `google-chrome`, `chrome.exe`, or `chromium` from your shell, you're good.
  - Windows: install Google Chrome from [google.com/chrome](https://www.google.com/chrome/)
  - macOS: `brew install --cask google-chrome`
  - Linux: `apt install chromium-browser` or distro equivalent

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/lordvojta/dump-it.git
cd dump-it

# Build in release mode
cargo build --release

# Binary location: ./target/release/dump-it
```

### Using Cargo

```bash
cargo install --path .
# Or from crates.io (once published)
cargo install dump-it
```

### Pre-built Binaries

Download pre-built binaries from the [Releases](https://github.com/lordvojta/dump-it/releases) page.

## Quick Start

```bash
# Simple - just provide a URL (auto-detects sitemap or crawls)
./target/release/dump-it --url https://vytvorit-web.cz

# Output: output/scraped.json
```

## Usage Examples

```bash
# Scrape from explicit sitemap URL
./target/release/dump-it --url https://example.com/sitemap.xml

# Auto-detect sitemap or crawl (works on ANY website)
./target/release/dump-it --url https://example.com

# Shallow crawl - only homepage + direct links (depth=1)
./target/release/dump-it --url https://example.com --max-depth 1 --max-pages 50

# Deep crawl for large sites
./target/release/dump-it --url https://example.com --max-depth 5 --max-pages 2000

# High concurrency for faster scraping
./target/release/dump-it --url https://example.com --concurrency 20

# Custom output location
./target/release/dump-it --url https://example.com --output output/mysite.json

# Increase timeout for slow websites
./target/release/dump-it --url https://example.com --timeout 60

# Add extra URL filters (skip preview / staging paths)
./target/release/dump-it --url https://example.com --exclude /preview --exclude /staging

# Keep all URLs (disable built-in WordPress / Elementor filters)
./target/release/dump-it --url https://example.com --no-default-excludes

# Give heavy SPAs more time to render before snapshotting
./target/release/dump-it --url https://example.com --js-wait 5000

# Emit one JSON file per page (in addition to the master scraped.json)
./target/release/dump-it --url https://example.com --split-pages

# Skip the brand-palette extraction (default is ON)
./target/release/dump-it --url https://example.com --no-extract-brand

# Static-site fast path (skip Chrome — ~50x faster)
./target/release/dump-it --url https://example.com --no-js

# Wait for a meaningful element instead of a fixed sleep
./target/release/dump-it --url https://example.com --js-wait-selector "main[data-ready]"

# Also capture the 404 template
./target/release/dump-it --url https://example.com --capture-404

# Route output to `test_runs/<host>/` instead of `output/` (for local dev runs)
./target/release/dump-it --url https://example.com --test-run
```

## Options

- `-u, --url <URL>` — Target website or sitemap URL (required)
- `-c, --concurrency <N>` — Max concurrent requests / Chrome tabs (default: 10)
- `-t, --timeout <SECS>` — Request timeout in seconds (default: 30)
- `-o, --output <FILE>` — Master JSON output file (default: `output/scraped.json`). Auxiliary files (`site.json`, `contact.json`, `brand.json`, `index.md`, `compact.json`, `schema.json`) are always written alongside it in the same directory.
- `-d, --max-depth <N>` — Max crawl depth when no sitemap (default: 3)
- `-m, --max-pages <N>` — Max pages to scrape during crawl (default: 1000)
- `--js-wait <MS>` — Milliseconds to wait after page load for JS to render (default: 2000)
- `--js-wait-selector <CSS>` — CSS selector to wait for instead of a wall-clock sleep; falls back to `--js-wait` if absent
- `--delay <MS>` — Politeness throttle between page requests. `0` = no throttle (default). If unset, `Crawl-delay:` from robots.txt is honoured automatically.
- `--exclude <PATTERN>` — Extra substring pattern to exclude from URLs (repeatable)
- `--no-default-excludes` — Disable built-in URL filters (see below)
- `--split-pages` — Also write each page as its own JSON file under `output/pages/<slug>.json`
- `--markdown` — Also emit a Markdown version of each page under `output/markdown/<slug>.md`
- `--screenshots` — Capture desktop (1280×800) + mobile (390×844) screenshots per page under `output/screenshots/`. Capture runs in parallel under `--concurrency`.
- `--jsonl` — Also write `scraped.jsonl` (newline-delimited `PageData`) alongside `scraped.json`. Useful for streaming consumers.
- `--max-images-per-page <N>` — Cap content images per page (default: 100; `0` disables the cap).
- `--user-agent <UA>` — Override the default User-Agent header.
- `--header "Name: Value"` (repeatable) — Extra HTTP headers (cookies, auth) on every request.
- `--include <PATTERN>` (repeatable) — Whitelist URL substrings; stacks with `--exclude`.
- `--no-extract-brand` — Skip the brand palette + fonts extraction (brand extraction is on by default)
- `--no-fetch-css` — Skip the external stylesheet fetch for brand mining (external-CSS fetch is on by default)
- `--no-js` — Skip launching Chrome and use plain reqwest. Recommended for static sites — much faster (≈ 50×).
- `--crawl-with-http` — Use plain HTTP (not Chrome) for the link-discovery crawl phase. Per-page scrape still uses Chrome unless `--no-js` is also set.
- `--ignore-robots` — Don't fetch or respect `/robots.txt` (default behaviour fetches it and filters Disallowed URLs)
- `--capture-404` — Probe a synthetic non-existent URL and capture the site's 404 template into `site.json:error_pages`
- `-q, --quiet` — Tracing level `warn` (suppresses info-level logs)
- `-v, --verbose` — Tracing level `debug` (full logging). Honours `RUST_LOG` if set.
- `--test-run` — Route output to `test_runs/<host>/` instead of `output/`. Useful for local development scrapes you don't want to mix with the canonical `output/` directory. Ignored if `--output` is explicitly set to a non-default path. The `test_runs/` directory is gitignored.

### Built-in URL exclude patterns

By default the scraper skips URLs containing any of these substrings, because they are almost never useful in a redesign export:

```
/wp-admin/   /wp-login    /wp-json/    /jkit-       /elementor-  elementor_library=
?elementor   /author/     /category/   /tag/        /feed/       /feed
/cart/       /checkout/   /my-account/ ?p=
```

Pass `--no-default-excludes` to keep them, or add your own with `--exclude /staging/ --exclude /preview`.

> **Windows / Git Bash users**: Git Bash transparently rewrites leading-slash CLI arguments to Windows paths (so `--exclude /home` becomes `C:/Program Files/Git/home` by the time it reaches the binary). dump-it detects and reverses this MSYS translation for `--exclude` and `--include` automatically, so `--exclude /home --exclude /contact` works the same in Git Bash as it does in PowerShell or a POSIX shell.

## Output Structure

A single run produces a folder like this:

```
output/
├── scraped.json     # master file — every page with full content blocks
├── site.json        # site-wide aggregate — nav, footer, brand, contact, templates, frameworks, sitemap, assets, error_pages
├── contact.json     # emails, phones, socials, addresses, organization schema
├── brand.json       # favicon, logo, color palette, fonts, CSS variables, webfont URLs
├── compact.json     # stripped-down view for tight LLM context windows
├── index.md         # human-readable entry point — start here when handing the folder to an agent
├── images/          # all downloaded binary assets
│   ├── favicon.<ext>
│   ├── logo.<ext>
│   ├── <hash>.<ext>      # content images + og:images
│   └── svg-<hash>.svg    # captured inline SVGs
├── pages/                # only with --split-pages: one JSON file per page
│   ├── home.json
│   └── about.json
├── markdown/             # only with --markdown: per-page Markdown rendering
│   ├── home.md
│   └── about.md
└── screenshots/          # only with --screenshots: desktop + mobile PNG per page
    ├── home.desktop.png
    └── home.mobile.png
```

The recommended workflow when handing this to a coding agent:

1. Have the agent read `index.md` first — it lists every page, what's in it, and where the supporting data lives
2. Have it consume `site.json` for the chrome (nav menu, footer, brand colors, fonts, favicon, logo, contact info)
3. Have it consume `scraped.json` (or the `pages/` directory if `--split-pages`) for per-page content
4. All binary assets are in `images/` referenced by relative path

## Output Format

`scraped.json` preserves the page layout and content hierarchy. A simplified shape (one page; many fields are omitted when null/empty):

```json
{
  "total_pages": 4,
  "pages": [
    {
      "url": "https://example.com/page",
      "title": "Page Title",
      "meta_title": "SEO Optimized Title",
      "meta_description": "Page description for search engines",
      "canonical_url": "https://example.com/page",
      "language": "en",
      "favicon_url": "https://example.com/favicon.ico",
      "logo_url": "https://example.com/logo.svg",
      "og_image_url": "https://example.com/og.png",
      "og_image_local_path": "output/images/12ab34cd56ef.png",
      "twitter_card": "summary_large_image",
      "hreflang_alternates": [
        { "lang": "en", "url": "https://example.com/page" },
        { "lang": "cs", "url": "https://example.com/cs/page" }
      ],
      "nav_links": [
        { "text": "Home",    "href": "https://example.com/" },
        { "text": "About",   "href": "https://example.com/about" }
      ],
      "footer_blocks": [
        { "type": "heading", "level": 4, "text": "Company" },
        { "type": "list", "items": ["About us", "Careers", "Press"] },
        { "type": "paragraph", "text": "© 2026 Example. All rights reserved." }
      ],
      "structured_data": [
        { "@context": "https://schema.org", "@type": "Organization", "name": "Example" }
      ],
      "content_blocks": [
        { "type": "heading", "level": 1, "text": "Main Heading" },
        { "type": "paragraph", "text": "This is a paragraph with clean text content..." },
        { "type": "list", "items": ["Item 1", "Item 2", "Item 3"] },
        {
          "type": "image",
          "original_url": "https://example.com/photo.jpg",
          "local_path": "output/images/abc123def456.jpg",
          "alt_text": "Photo description"
        }
      ],
      "sections": [
        { "section_type": "hero",     "block_start": 0, "block_end": 2, "summary": "Main Heading" },
        { "section_type": "features", "block_start": 4, "block_end": 12, "summary": "4 feature items" }
      ],
      "quality_flags": ["no_canonical", "images_missing_alt:2"],
      "total_words": 450,
      "page_contact": {
        "emails": ["hello@example.com"],
        "phones": ["+1 415 555 0123"],
        "social_links": [{ "platform": "github", "url": "https://github.com/example" }],
        "addresses": [],
        "organization": null
      },
      "internal_links_out": [
        "https://example.com/about",
        "https://example.com/contact"
      ]
    }
  ]
}
```

### Block Types

Content is structured into blocks that preserve the page layout:

**Heading Block**
```json
{
  "type": "heading",
  "level": 1,      // 1-6 for h1-h6
  "text": "Heading text"
}
```

**Paragraph Block**
```json
{
  "type": "paragraph",
  "text": "Clean paragraph text with whitespace normalized"
}
```

**List Block**
```json
{
  "type": "list",
  "items": ["Item 1", "Item 2", "Item 3"]
}
```

**Image Block** (appears in context where image was on page)
```json
{
  "type": "image",
  "original_url": "https://example.com/image.jpg",
  "local_path": "output/images/hash.jpg",
  "alt_text": "Image description"
}
```

**Code Block** (`<pre>` / `<pre><code>` with best-effort language detection)
```json
{ "type": "code", "language": "rust", "text": "fn main() { println!(\"hi\"); }" }
```

**Quote Block** (`<blockquote>`)
```json
{ "type": "quote", "text": "The only way to go fast is to go well.", "cite": "https://example.com/source" }
```

**Media Block** (`<video>` / `<audio>`)
```json
{ "type": "media", "kind": "video", "src": "https://example.com/demo.mp4", "poster": "https://example.com/cover.jpg", "title": "Demo" }
```

**Definition List Block** (`<dl>` / `<dt>` / `<dd>`)
```json
{ "type": "definitionlist", "items": [ { "term": "HTML", "description": "HyperText Markup Language" } ] }
```

**Table Block** (`<table>` with optional caption + headers)
```json
{ "type": "table", "caption": "Plan comparison", "headers": ["Plan", "Price"], "rows": [["Free", "$0"], ["Pro", "$29"]] }
```

**Embed Block** (captures iframes — YouTube, Vimeo, Maps, Spotify, Calendly, Typeform, HubSpot, …)
```json
{
  "type": "embed",
  "provider": "youtube",
  "src": "https://www.youtube.com/embed/abc123",
  "title": "Product demo"
}
```

**Form Block** (captures contact forms, search forms, etc.)
```json
{
  "type": "form",
  "action": "/submit",
  "method": "POST",
  "fields": [
    {
      "field_type": "text",
      "name": "name",
      "label": "Your Name",
      "placeholder": "Enter your name",
      "required": true,
      "options": []
    },
    {
      "field_type": "email",
      "name": "email",
      "label": "Email Address",
      "placeholder": "you@example.com",
      "required": true,
      "options": []
    },
    {
      "field_type": "select",
      "name": "subject",
      "label": "Subject",
      "placeholder": "",
      "required": false,
      "options": ["General Inquiry", "Support", "Sales"]
    },
    {
      "field_type": "textarea",
      "name": "message",
      "label": "Message",
      "placeholder": "Your message here...",
      "required": true,
      "options": []
    }
  ],
  "submit_text": "Send Message"
}
```

### Site-Level Templates (`site.json:templates`)

Pages that share the same block-pattern signature are grouped so the agent can rebuild a single component instead of N near-identical pages.

```json
{
  "templates": [
    {
      "template_id": "tpl_7e75cc0b",
      "block_pattern": ["img", "h1"],
      "page_count": 9,
      "sample_page": "https://example.com/team/alice",
      "pages": ["https://example.com/team/alice", "..."]
    }
  ]
}
```

### Per-Page Sections (`PageData.sections`)

Heuristic section inference over `content_blocks`. Section types:

| Type | What it means |
|------|---------------|
| `hero` | First heading + supporting paragraphs (and optional hero image) |
| `features` | 3+ consecutive (heading + paragraph) pairs at the same heading level |
| `team` | 3+ consecutive (image + short-heading) pairs |
| `cta` | A form block |
| `embed` | An iframe/video embed |
| `content` | Default fallback |

```json
{
  "sections": [
    { "section_type": "hero",     "block_start": 0,  "block_end": 2,  "summary": "Welcome to Acme" },
    { "section_type": "features", "block_start": 4,  "block_end": 12, "summary": "4 feature items" },
    { "section_type": "cta",      "block_start": 18, "block_end": 19, "summary": "form" }
  ]
}
```

### Quality Flags (`PageData.quality_flags`)

Per-page SEO / accessibility issues. Possible values:

- `no_h1`, `multiple_h1:<count>`
- `no_meta_description`, `meta_description_too_short`, `meta_description_too_long`
- `no_title`, `title_too_long`
- `no_canonical`
- `images_missing_alt:<count>`, `images_low_quality_alt:<count>` (placeholder alts like `"image"`, `"photo"`, raw filenames)
- `thin_content`
- `meta_robots_noindex`, `meta_robots_nofollow` (from `<meta name="robots">`)
- **`parked_domain`** — Body is only an iframe to a parked-domain provider (rapidresultsearch, sedoparking, afternic, bodis, dan.com). The site is dead — agent should not rebuild it.

A flag rollup table is shown at the top of `index.md`.

### Bundle-Level Quality Warnings (`site.json:quality_warnings`)

Bundle-wide diagnostics that surface as top-of-`index.md` banners:

- **`spa_loading_shell:<N>_of_<M>_pages_share_<K>_block_template`** — ≥80% of pages share a tiny (<5-block) template, meaning headless Chrome captured the loading skeleton before JS hydrated. The agent should NOT trust the content blocks.
- **`partial_scrape:<X>%_pages_skipped`** — ≥50% of attempted pages were bot-protected, render-failed, or unreachable. Bundle is incomplete; see `skipped_pages` for the per-URL list.
- **`cross_domain_sitemap:<X>%_urls_at_<foreign_host>`** — Sitemap points mostly at a different host (acquisition / merger redirect). Bundle is named after the input URL but content is from a different domain.

### Page-Level Fields (`PageData`)

- `url` - The page URL
- `title` - Page title from `<title>` tag
- `meta_title` - SEO title from meta tags (fallback to `<title>`)
- `meta_description` - SEO description from meta tags
- `canonical_url` - From `<link rel="canonical">` if present, else `null`
- `language` - From `<html lang>` if present, else `null`
- `favicon_url` - Best favicon URL — prefers `apple-touch-icon`, then `icon`, then `shortcut icon`. `null` if none.
- `logo_url` - Best-guess logo URL for the page (header img, `[class*='logo']`, or `Organization.logo` from JSON-LD). `null` if none.
- `og_image_url` - From `og:image` or `twitter:image` meta. `null` if none.
- `og_image_local_path` - Downloaded copy of the OG image (if reachable). Shared across pages that reference the same URL.
- `twitter_card` - From `twitter:card` meta. `null` if not set.
- `hreflang_alternates[]` - `{lang, url}` pairs extracted from `<link rel="alternate" hreflang>` (multilingual sites only)
- `sections[]` - Heuristic-inferred sections over `content_blocks` (see "Per-Page Sections" above)
- `quality_flags[]` - SEO / accessibility issues (see "Quality Flags" above)
- `nav_links[]` - Deduplicated list of `{text, href}` extracted from all `<nav>`, `<header>`, `[role='navigation']`, and `[role='banner']` regions. Lets a coding agent rebuild the primary navigation without inferring it.
- `footer_blocks[]` - Ordered `ContentBlock` array extracted from `<footer>` and `[role='contentinfo']` regions (headings, paragraphs, lists). Use this to rebuild the footer with the original copy / link groupings.
- `structured_data[]` - Raw `application/ld+json` (schema.org) blobs as JSON, one per `<script type="application/ld+json">` tag. Often contains breadcrumbs, organization info, articles, publisher logo URLs, etc.
- `content_blocks[]` - Ordered array of content blocks preserving layout. Nav/header/footer/aria-hidden/slider-clone elements are excluded.
- `page_contact` - Per-page contact info (emails, phones, socials, addresses) — omitted when empty
- `total_words` - Total word count across heading/paragraph/list text blocks
- `meta_robots` - Raw `<meta name="robots">` value, e.g. `"noindex,nofollow"` — feeds quality flags
- `plain_text` - Concatenated text of every heading/paragraph/list-item block. Useful for full-text search and cheap LLM context.
- `content_hash` - SHA-256 hex (first 16 chars) of `plain_text`. Lets the agent dedupe boilerplate across pages and detect changes between runs.
- `token_estimate` - Rough LLM token count (`chars / 4`) so the agent can budget its context window.
- `summary` - Auto-built one-liner: meta_description → first paragraph → first heading. Appears in `index.md` and `compact.json`.
- `page_assets[]` - Local paths of every asset this page references (content images, inline SVGs, og:image). Lets the agent rebuild a single page without scanning the whole bundle.
- `internal_links_out[]` - Internal anchor hrefs (resolved to absolute) on this page. Site-level rollup is `internal_links_in`.
- `screenshot_desktop`, `screenshot_mobile` - Relative paths to PNG captures when `--screenshots` is set.

### Site-Level Fields (`site.json`)

- `base_url` - The starting URL passed via `--url`
- `language` - First non-null page language
- `frameworks[]` - Detected stacks with `confidence` (high/medium/low) and `evidence[]` listing the specific HTML markers
- `primary_nav[]` - Aggregated `NavLink[]` with `text`, `href`, optional `description` (mega-menu blurb) and `role` (`header` / `mega_menu` / `utility` / `social` / `footer`)
- `footer_blocks[]` - Aggregated footer (first non-empty across pages; falls back to `[class*="footer" i]` / `[id*="footer" i]` when no semantic `<footer>` element exists)
- `contact` - Aggregated `ContactInfo` (emails / phones / socials / addresses / Organization JSON-LD / `contact_form_endpoints`)
- `brand` - Aggregated `BrandPalette` (favicon, logo, colors, fonts, CSS variables, webfont URLs, `confidence`)
- `templates[]` - Same-shape page groups (block-pattern signatures) — rebuild as one component
- `hreflang_groups[]` - Locale clusters from `<link rel="alternate" hreflang>` across pages
- `sitemap[]` - Per-page summaries — URL, title, category, word count, has-form flag, image count, primary heading, `internal_links_in/out`, screenshot paths
- `total_pages` - Successful page count
- `assets[]` - Flat manifest of every file produced (path, size_bytes, kind)
- `error_pages[]` - Synthetic-probe pages (currently the 404 template when `--capture-404` is set)
- `output_files[]` - The list of files this run produced
- `quality_warnings[]` - Bundle-level warnings (see "Bundle-Level Quality Warnings" above)
- `skipped_pages[]` - Per-URL `{url, reason}` list of pages that failed to render (`render_failed` / `bot_protected`). Pairs with the `partial_scrape` quality warning.

### Brand Fields (`brand.json` + `site.json:brand`)

- `favicon_url`, `favicon_local_path`
- `logo_url`, `logo_local_path`
- `colors[]` - Top-12 colors by frequency. Values can be hex (`#513289`), `rgb(…)`, or `hsl(…)` literals. Bootstrap 3/4 utility palettes + known syntax-highlight themes (Atom One Dark, Dracula, Monokai, Tomorrow Night, Solarized Dark, Nord, Catppuccin) are filtered when 3+ of their colors appear. Fully-transparent values (`rgba(*,*,*,0)`, `#xxxxxx00`) are dropped.
- `fonts[]` - Top-12 `font-family` values by frequency. Filtered out: generic families (`sans-serif`, etc.), `var(...)` references, `*-fallback` / ` Fallback` / `_Fallback` suffixes, `__Inter_e8ce0c` mangled names, `!important` leakage, URL-encoded fragments (`%2C`, `%3A`), Adobe Typekit weight-encoded aliases (`tk-*-n[0-9]+`), icon-font names (FontAwesome, Material Icons, ETmodules, swiper-icons, slick-icons, ionicons, octicons, pagebuilder-font, anything ending `-icons`/`-icon` or containing `iconic`/`glyph`), carousel-library CSS-class names (slick, swiper, splide, flickity, owl), and anything outside the alphanumeric+space+hyphen character class.
- `css_variables[]` - `--<name>: <value>` definitions ranked by frequency
- `webfont_urls[]` - Detected `<link rel="stylesheet">` URLs to Google Fonts / Adobe Fonts / Bunny Fonts / cdnfonts, with parsed `families[]`. Family names propagate into the `fonts[]` list with a small boost so sites using `font-family: var(--font-sans)` exclusively still surface their real fonts.
- `confidence` - `low` (top sample count <10), `medium` (10–29), `high` (≥30). Surfaced in `index.md` when not `high`. Helps the agent treat Next.js / Tailwind sites' palettes skeptically.

### Contact Fields (`contact.json` + `site.json:contact`)

- `emails[]` - Case-insensitive-deduped (uppercase + lowercase variants collapse to one), sorted. Lowercase variant preferred when both seen. `mailto:` URL-encoded addresses are decoded (`info%40brand.com` → `info@brand.com`). **General same-host rule:** all emails — both `mailto:` anchors AND body-text matches — must have a domain matching the site's own host or a subdomain. This single universal rule filters out regulator contacts cited in privacy notices, parent-company / partner / vendor emails, and partner law-firm contacts — without any hardcoded denylist. Trade-off: cross-brand partnership emails get filtered too; the bias is toward "what's clearly the brand's own contact". Body-text scanning is otherwise restricted to chrome zones (nav / header / footer) except on `/contact` / `/about` / legal pages where body emails are also scanned. Personal addresses mentioned in blog post bodies still cannot leak as brand contacts.
- `phones[]` - Country-code-aware deduped (E.164 + national variants fold to one entry; 30+ country codes recognized; NANP `1` prefix also stripped for non-plus 11-digit numbers; `+` variant preferred). Validated by digit count (9–15), group shape (1–4 groups for standard format, 5 allowed for French dot-style; no single-digit non-first group, no multi-space, no cross-line `\n`/`\r`, no Unix-timestamp shape `1[5-9]XXXXXXXX`). Body-text phones are NOT scanned on legal pages (`/privacy`, `/terms`, `/legal`, `/ccpa`, `/gdpr`, etc.) to avoid regulator contacts cited in compliance notices — `tel:` anchors are still honored everywhere.
- `social_links[]` - `{platform, url}` pairs across Facebook, Instagram, Twitter/X, LinkedIn, YouTube, TikTok, Pinterest, Snapchat, GitHub, Vimeo, Threads, Bluesky, Mastodon, Medium, Dribbble, Behance. Restricted to chrome zones (nav / header / footer) so user-submitted body links don't pollute the brand's social profile list.
- `addresses[]` - Comma-joined postal addresses extracted from JSON-LD `PostalAddress` records
- `organization` - Raw schema.org `Organization`/`LocalBusiness` blob if found
- `contact_form_endpoints[]` - `action` URLs of forms classified as `contact`. Surfaced in `index.md` when the site has no `mailto:` / `tel:` (common on sites that prefer forms for anti-spam). Agent's rebuilt form should POST to the same URL.

## How It Works

### 1. Sitemap Detection
When you provide a URL, the scraper first looks for `sitemap.xml` at the domain root. If found, it extracts all URLs and scrapes them directly. Nested sitemap indexes (`.xml` referenced from inside another sitemap) are resolved recursively.

### 2. Intelligent Fallback
If no sitemap exists (or contains only 1 URL), the scraper automatically starts crawling mode.

### 3. Web Crawler
The crawler discovers pages by:
- Starting at your provided URL
- Loading the page in headless Chrome (so JS-injected links are also visible)
- Extracting all `<a href>` links from the rendered DOM
- Following those links to discover more pages
- Only following links on the **same domain** (ignores external links)
- Tracking visited URLs to **avoid duplicates**
- Respecting **depth** and **max pages** limits

### 4. Headless Chrome Rendering (default)
By default, every page is loaded in a real Chrome instance via `headless_chrome`. The scraper waits for `<body>` to appear plus a fixed `--js-wait` delay (default 2 s) so JS frameworks (React, Vue, Elementor, etc.) have time to populate the DOM. Alternatively, pass `--js-wait-selector <css>` to wait for a specific element instead of a wall-clock sleep.

### 5. Static-site fast path (`--no-js`)
For sites that don't need JS execution (Hugo, Jekyll, Astro static output, plain HTML), pass `--no-js` to bypass Chrome entirely. Pages are fetched with plain `reqwest`. Roughly **50× faster** because there's no browser launch or render delay.

### 6. Concurrent Scraping
After discovering URLs (via sitemap or crawling), pages are scraped in parallel using a semaphore to cap simultaneous Chrome tabs (or HTTP requests in `--no-js` mode).

## Understanding Crawler Depth

The `--max-depth` parameter controls how many "link hops" away from the starting URL the crawler will go.

### Depth Examples:

**Depth 0** - Only scrapes the starting URL
```
example.com (starting URL)
```

**Depth 1** - Starting URL + all pages directly linked from it
```
example.com (starting URL)
├── example.com/about (linked from homepage)
├── example.com/contact (linked from homepage)
└── example.com/services (linked from homepage)
```

**Depth 2** - Goes one level deeper
```
example.com (starting URL)
├── example.com/about (linked from homepage)
│   ├── example.com/team (linked from /about)
│   └── example.com/history (linked from /about)
├── example.com/contact (linked from homepage)
└── example.com/services (linked from homepage)
    ├── example.com/services/web (linked from /services)
    └── example.com/services/mobile (linked from /services)
```

**Depth 3** (default) - Three levels of links
- Good balance for most websites
- Captures main content without going too deep
- Avoids getting lost in deep navigation structures

**Depth 5+** - Very deep crawl
- For large sites with deep hierarchies
- May discover hundreds/thousands of pages
- Use with `--max-pages` limit to control size

### Recommended Settings:

| Website Type | Depth | Max Pages | Use Case |
|-------------|-------|-----------|----------|
| Small business site | 1-2 | 50 | Quick scrape of main pages |
| Medium website | 3 | 500 | Default - good balance |
| Large website | 4-5 | 1000-2000 | Comprehensive scrape |
| Documentation site | 5+ | 5000 | Deep technical docs |

## Performance

Optimized release build with:
- LTO (Link Time Optimization)
- Stripped binary
- Async I/O with Tokio
- Concurrent fetching with semaphore-based rate limiting
- Efficient URL deduplication

**Trade-off note:** In default (Chrome) mode, throughput is dominated by browser startup and the JS-settle delay rather than network latency — expect roughly 2–4 seconds per page on top of the page's own load time. For purely server-rendered sites, pass `--no-js` to bypass Chrome and use plain reqwest (≈ 50× faster). Use `--js-wait-selector <css>` to replace the fixed sleep with a "wait for this element" check when JS rendering is still required.

## Real-World Example

```bash
$ ./target/release/dump-it --url https://www.prag-travel.de/
🚀 Starting scraper...
Target: https://www.prag-travel.de/
Concurrency: 10
🔍 Looking for sitemap at: https://www.prag-travel.de/sitemap.xml
✓ Found sitemap with 36 URLs
📊 Found 36 URLs to scrape
✓ Scraped: https://www.prag-travel.de/blog-bootsfahrt-in-prag/ (5 blocks, 10 words, 1 images)
✓ Scraped: https://www.prag-travel.de/referenz/ (7 blocks, 17 words, 2 images)
✓ Scraped: https://www.prag-travel.de/anfrage-formular/ (7 blocks, 23 words, 1 images)
...
✓ Scraped: https://www.prag-travel.de/ (24 blocks, 179 words, 8 images)
✅ Done! Scraped 36/36 pages
💾 Output saved to: output/scraped.json
```

Result:
- JSON file with structured content blocks preserving page layout
- 152 images downloaded to `output/images/` folder
- Content organized into headings, paragraphs, lists, images, and forms
- Images and forms appear inline where they occurred on the page

## Tips & Best Practices

### Choosing the Right Depth
- **Start shallow** (depth 1-2) to see what you get
- **Increase gradually** if you need more coverage
- **Monitor output size** - each depth level can exponentially increase pages

### Preventing Runaway Crawls
- Always set `--max-pages` limit for safety
- Default 1000 pages prevents excessive scraping
- Increase only when you know the site structure

### Performance Tuning
- **Fast sites**: Increase `--concurrency` to 20-50
- **Slow sites**: Decrease to 5 and increase `--timeout`
- **Large sites**: Use higher depth with high max-pages

### Handling Different Sites
- **Static sites**: Usually have sitemaps, very fast
- **Dynamic sites**: May need crawler mode, slower
- **E-commerce**: Set high max-pages (products = many URLs)
- **Blogs**: Depth 2-3 usually captures all posts

## Troubleshooting

### "Failed to launch browser" / process panics on startup
The `headless_chrome` crate could not locate a Chrome/Chromium binary. Install Chrome (see [Prerequisites](#prerequisites)) and make sure it is on `PATH`, or set the `CHROME` environment variable to the absolute path of the executable before running.

### "No sitemap found, starting crawler"
This is normal. The tool will automatically discover pages by following links from the start URL.

### Crawler finds fewer pages than expected
- Increase `--max-depth` (site may have deep navigation)
- Increase `--max-pages` limit
- Some pages may sit behind interactive elements (e.g. require a button click) which the scraper does not exercise

### Timeout errors
- Increase `--timeout` to 60 or higher
- Decrease `--concurrency` to reduce load
- Website may be blocking automated requests

### Getting blocked by rate limiting
- Decrease `--concurrency` to 5 or lower
- Adding deliberate per-request delays is on the roadmap (not currently supported)
- Website may require custom headers/authentication

### Duplicate content from sliders/carousels
JS sliders (Swiper, Slick) clone their slides for infinite scrolling. dump-it now filters these in two ways: it skips elements with `aria-hidden="true"`, `.swiper-slide-duplicate`, `.swiper-slide-duplicate-active`, or `.slick-cloned`, and it runs an adjacent-duplicate filter on long text blocks. If a slider plugin uses non-standard markers and clones still leak through, raise an issue.

## Limitations

- **Authentication**: Cannot scrape pages behind login walls
- **Interactive content**: Tabs, accordions, "load more" buttons, and modals are not clicked
- **Rate limiting**: Some sites may block high-frequency requests. Built-in retry/backoff (200 ms → 600 ms → 1.8 s) handles transient 5xx for asset fetches, but page-render failures aren't retried.
- **Brand mining is style-source bound**: Colors / fonts / CSS variables are mined from inline `<style>` blocks **plus** every linked external stylesheet (`--no-fetch-css` to disable). Compiled/obfuscated CSS (some Tailwind-JIT bundles) may still produce a thin fonts list — mitigation: hand the agent `images/logo.<ext>` or a hero screenshot and let it eyeball the typeface.
- **Cloudflare / bot-protected sites**: Heavily-protected sites may serve a challenge page or hang. Try `--concurrency 1` and/or a larger `--js-wait`. If it persists, the site is blocking automation.
- **Image formats**: Downloads images as-is (no format conversion or resizing)
- **`--max-pages` only caps crawler mode**: Sitemap mode scrapes every URL the sitemap returns (after applying `--exclude` patterns and robots.txt rules)
- **Page-render retries**: A single render timeout drops a page from the export (logged to stderr). Retry/backoff currently covers only asset fetches (images, CSS, favicon, logo, og:image).
- **No per-request delay**: There is no `--delay` flag yet — `--concurrency 1` is the only throttle if you need to be very polite.

## Tested on

dump-it has been smoke-tested against a variety of site shapes, including:
- Single-page static sites (Hugo / Jekyll / Astro static output / plain HTML)
- Next.js / React / Vue / Svelte marketing sites with JSON-LD and mega-menu navigation
- WordPress + Elementor / Divi / Webflow / Squarespace portfolio sites with sliders and image-heavy content
- Shopify e-commerce sites with sub-sitemaps and product catalogs
- Multi-locale sites with `hreflang` alternates
- Sites behind Cloudflare / WAF challenges (correctly detected and handled gracefully)

## Module Layout

For contributors, the source is split as follows:

```
src/
├── main.rs       — entry point: parses CLI, orchestrates the scrape, emits all output files
├── cli.rs        — clap Args definition
├── model.rs      — all data types (PageData, ContentBlock, SiteData, BrandPalette, …)
├── selectors.rs  — cached CSS selectors (LazyLock<Selector>) + regex statics + constants
├── util.rs       — small helpers: element_text, url_to_slug, image_extension_from_url, body_text_only, dedup_adjacent_long_text, count_words, embed_provider_from_src
├── chrome.rs     — headless-Chrome render + screenshot capture (tab cleanup baked in)
├── extract.rs    — DOM extraction (meta, canonical, lang, favicon, logo, structured data, nav, footer, content blocks, stylesheets, internal links, image download)
├── scrape.rs     — Scraper struct: HTTP client + Browser, sitemap, crawler, scrape_page orchestration
├── contact.rs    — phone validator, social-share filter, dedup_phones, extract_contact
├── brand.rs      — color/font/CSS-var aggregation, webfont URL parsing, favicon/logo download, external CSS fetcher
└── output.rs     — categorize_page, build_site_data, build_index_md, page_to_markdown, build_compact, build_asset_manifest, detect_frameworks_from_html
```

## Feeding the Output to a Coding Agent

The recommended workflow for redesign/migration:

1. Run `dump-it --url https://oldsite.example` (add `--screenshots` if you want visual reference, `--markdown` for LLM-friendly per-page MD)
2. Open a new project in your coding agent (Claude Code, Cursor, etc.) and tell it the target stack (Next.js, Astro, plain HTML, etc.)
3. Hand the agent the whole `output/` folder
4. Have it read **`index.md` first** (it's the orientation map), then `site.json` for the chrome, then drill into `scraped.json` / `pages/*.json` / `markdown/*.md` for per-page content
5. Spot-check the rebuilt pages against the originals — the bundle preserves copy, structure, metadata, brand assets, contact info, sitemap, and link graph

Tips for getting better results from the agent:
- **Use `site.json:templates`** — pages with the same shape collapse to one template, so the agent rebuilds one component (e.g. a `<TeamMember />` card) and binds N records rather than N near-identical pages
- **Use `site.json:primary_nav` and `site.json:footer_blocks`** — the chrome is already extracted; the agent doesn't need to re-infer it from raw content
- **Use `brand.json`** for the visual system: colors, fonts, CSS variables, and webfont URLs are ranked by frequency, so the top entries are the brand's actual design tokens
- **Use `compact.json`** when the agent's context window is tight — it has all the structure (sitemap, sections, brand summary) without the full per-page bodies
- If `quality_flags` flags `multiple_h1` or `no_canonical` on the source site, decide whether to inherit or fix those issues in the rebuild

## Roadmap

The Round D push shipped all 18 items from the previous list (see `CHANGELOG.md`). Open candidates for the next round:

### Quick wins
- **Persistent caching of downloaded images** by content hash across runs (currently we re-download on each invocation if `output/images/` was wiped).
- **`--user-agent <ua>`** flag to override the default `Mozilla/5.0 (compatible; DumpIt/0.1)`.
- **Cookie/header passthrough** (e.g. `--header "Cookie: ..."`) for behind-paywall scraping you have legitimate access to.
- **Sitemap.xml `lastmod` + `priority` capture** — surface in `site.json:sitemap` so the agent knows which content is freshest.
- **Per-page screenshot thumbnail in `index.md`** — Markdown image tags so the human reviewer can glance at the bundle.

### Medium effort
- **Auto-respond to cookie banners** before snapshotting (heuristic click of "Accept" / "Souhlasím"). High value on EU sites.
- **`<picture>` / `<source>` `media` query awareness** — pick the desktop variant deliberately rather than the largest srcset.
- **JSON-LD aggregation** in `site.json` — merge per-page `Organization` / `WebSite` blobs to one canonical record.
- **Visual change-detection** between two runs (`dump-it diff old/ new/`) — surface which pages, sections, or assets changed.
- **`.har` / network log export** alongside screenshots — gives the agent a sense of API endpoints.

### Larger initiatives
- **`thiserror` boundary types** for the public-facing functions (currently we use `anyhow` everywhere; if `dump-it` becomes a library we'll want stable typed errors).
- **Plugin architecture** — let users extend section detectors / form classifiers without forking.
- **WASM build** for in-browser scraping of single pages (no Chrome).
- **Distributed crawl** — split URL list across N workers via a queue (Redis / SQS) for very large sites.
- **TypeScript types generation** alongside `schema.json` for agents using TS.

### Recently shipped

dump-it has gone through 13 rounds of hardening (A through M) driven by smoketests on diverse public sites across multiple verticals (food / hospitality, retail / e-commerce, services, publishers, SaaS) and several countries. The full round-by-round changelog is in [`CHANGELOG.md`](./CHANGELOG.md). Headline categories of work, in rough order:

- **Module split** — single-file monolith → 11 focused modules (`cli`, `model`, `selectors`, `util`, `chrome`, `extract`, `scrape`, `contact`, `brand`, `output`)
- **Static fast path** (`--no-js`) for sites that don't need Chrome (≈50× faster)
- **Robots.txt respected by default** with `Crawl-delay` honoured automatically
- **Sitemap-aware scraping** with cycle guard, sub-sitemap recursion, and crawler fallback
- **Multi-file output bundle** — `scraped.json`, `site.json`, `contact.json`, `brand.json`, `index.md`, `compact.json`, `schema.json`, optional `pages/`, `markdown/`, `screenshots/`
- **Content extraction expanded** — Heading / Paragraph / Image / List / Form / Embed / Table / Code / Quote / Media / DefinitionList block types with figcaption fallback for empty alts, double-counting guard on emitted containers
- **Brand mining** — colours, fonts, CSS variables, webfont URLs; with filters for known syntax themes, Bootstrap utility palettes, icon fonts, carousel-library CSS classes, weight-encoded suffixes, URL-encoded font fragments, transparent values, and `confidence` rating
- **Contact extraction** — emails (case-insensitive dedup, `mailto:` URL-decoding, same-host filtering), phones (country-code-aware dedup, NANP / French / Czech format handling, regulator-leak prevention via legal-page body suppression), social profiles (chrome zones only)
- **Quality flags + bundle warnings** — per-page (`no_h1`, `no_canonical`, `thin_content`, `parked_domain`, …) and bundle-level (`spa_loading_shell`, `partial_scrape`, `cross_domain_sitemap`)
- **Failure-resilient output** — crash-survival placeholder, partial-scrape banner, empty-bundle banner, cross-domain sitemap banner
- **Framework detection** — many stacks identified with `confidence` + `evidence[]`
- **Page-level metadata** — `og_image`, `twitter_card`, `meta_robots`, `hreflang_alternates`, `plain_text`, `content_hash`, `token_estimate`, `summary`, `page_assets`, `internal_links_out`, `screenshot_desktop`/`mobile`
- **Section inference + template grouping** — pages with the same block-pattern signature collapse to one `PageTemplate`
- **CLI flags** — `--delay`, `--js-wait`, `--js-wait-selector`, `--no-js`, `--max-images-per-page`, `--user-agent`, `--header`, `--include`, `--screenshots`, `--markdown`, `--jsonl`, `--split-pages`, `--capture-404`, `--ignore-robots`, `--no-extract-brand`, `--no-fetch-css`, `--crawl-with-http`, `--quiet`, `--verbose`, `--test-run`
- **Engineering** — `anyhow::Result` throughout, `tracing` + `tracing-subscriber`, 29 unit tests, GitHub Actions CI (`fmt`, `clippy -D warnings`, `test`, build), Windows / Git Bash MSYS path translation handling, comprehensive `.gitignore`

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. For major changes, please open an issue first to discuss what you would like to change.

### Development

```bash
# Run in development mode
cargo run -- --url https://example.com

# Run tests (when added)
cargo test

# Format code
cargo fmt

# Lint
cargo clippy
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

Vojtech Kotrc - [GitHub](https://github.com/lordvojta)

## Acknowledgments

Built with:
- [Tokio](https://tokio.rs/) - Async runtime
- [Reqwest](https://github.com/seanmonstar/reqwest) - HTTP client
- [Scraper](https://github.com/causal-agent/scraper) - HTML parsing
- [Clap](https://github.com/clap-rs/clap) - CLI argument parsing
