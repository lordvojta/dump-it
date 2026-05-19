# Changelog

All notable changes to **dump-it** are documented here. Format adapted from [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project will follow [SemVer](https://semver.org/) once 1.0.0 is tagged.

## [Unreleased]

### Round M — Continuous Czech-cohort iteration (4 fixes)

User asked for continuous loop validation on Czech small-biz sites. Three iterations surfaced regulatory contacts leaking from privacy pages, font weight-suffix variants, URL-encoded mailto, and crystallized the principle that all rules must be GENERAL (no hardcoded lists of specific values per site).

#### Fixed

- **Font weight-suffix variants merged.** Mahabis emitted `AktivGrotest`, `AktivGrotestBold`, `AktivGrotestHairline`, `AktivGrotesk` as 4 separate "fonts" — same family with weight aliases. `normalize_font_family()` strips trailing weight words (Thin/Light/Book/Regular/Medium/Bold/Black/Heavy/Hairline/UltraLight/ExtraLight/SemiBold/DemiBold/ExtraBold/Italic/Oblique) when separated by space / hyphen / camelCase boundary. AktivGrotest now ranks at the combined count (1100×) instead of fragmenting across 4 entries.
- **`mailto:` URL-decoding.** knihy.cz emitted both `info%40knihy.cz` AND `info@knihy.cz` as separate emails. `percent_decode_email()` translates `%40 → @`, `%2B → +`, `%2E → .`, `%2D → -`, `%5F → _`, `%2F → /` (spaces dropped — invalid in emails). After decode, `dedup_emails` collapses the variants.

#### Changed

- **Regulatory contact filtering rewritten as a general structural rule** (per user guidance: "do not make conditions related to individual websites — rules need to be general"). Removed the previous CA-DOJ phone number denylist and the EU DPA email domain denylist (both were hardcoded lists of specific values). Replaced with:
  - **All emails (mailto: and body-text alike) must match the site's own host or a subdomain.** This single universal rule subsumes the deleted denylists. CCPA / GDPR notices universally cite regulator (`@uoou.cz`, `@cnil.fr`) and parent-company (`@heineken.com`, `@studentagency.cz`) addresses, and form-only sites often have web-vendor support emails (`@orsys.cz`). None of these are the brand's primary contact. Round-M validation: Starobrno 8 → 3 emails; Letenky 7 → 1 email; Knihy 2 → 1 email.
  - On legal pages: body-text phones are not scanned at all (`tel:` anchors still honored).
  - The trade-off: brands that ONLY publish their customer-service phone in privacy-page body text (e.g. Buck Mason) will lose it; cross-brand partnerships announced via `mailto:` get filtered. The structural rule is preferred because (a) the user will not scrape the same site twice — generalization matters more than per-site optimization, (b) the agent can still find such contacts manually if needed.
- **NANP phone dedup extended to non-plus form** — `+18554961110` and `1-855-496-1110` are the same number. `canonical_digits()` now strips a leading `1` when the digit-only form is exactly 11 digits, aligning the non-plus NANP variant with the `+`-form canonical.
- **Bare icon-font names filtered** — Added exact-match denylist for `icons`, `icon`, `icomoon`, `iconmoon`, `icofont`, `themify`, `lineicons`, `dripicons`, `linearicons`, `eicons`, `elementskit`, `essential-addons`, `ekiticons`, `uicons` — universal icon-font naming conventions (vanilla, generator-suffixed, and Elementor / ElementsKit / Essential Addons WordPress libraries), never brand typography.
- **SPA loading-shell detector refined with content-density gate.** Previously fired on any cluster where ≥80% of pages share a tiny (<5-block) template — false-positive on page-builder WordPress sites (Elementor / Divi / Webflow) where pages legitimately share template shapes with substantial content. Now also requires `mean total_words per page < 50` — true loading shells have ~0–30 words/page, real templates have 100+.
- **Placeholder-phone pattern detection.** Demo phones like `+420 111 222 333` or `+1 555 555 5555` slipped through previous validation because each group passes individual checks. New general rule: reject when the digit-only form contains any 9-character window of three consecutive mono-digit triples (`AAABBBCCC`). Catches both formatted and compact placeholder forms without any site-specific values. Real numbers with a single mono group (`+1 800 222 7890`) still pass.

### Round L — Czech cohort fixes (4 fixes)

Driven by smoketest on 7 Czech sites (Rohlik, Kosik, Divadlo na Vinohradech, Footshop, Bohemia Bagel, Martinus, Alza). Findings revealed total silent failures when Chrome crashes early, undeduped E.164/national phone variants, Adobe Typekit weight-encoded font names, and form-only contact UX leaving emails/phones empty.

#### Fixed

- **No more silent total failures.** Martinus.cz crashed during sitemap fetch (Chrome transport timeout) before any output directory existed, leaving the user with zero files. Output directory + a placeholder `index.md` failure banner are now written **immediately** after argument parsing. If the run crashes mid-flight, the placeholder survives so the user sees "❌ Scrape crashed — no bundle produced" with recovery suggestions. On success the placeholder gets overwritten with the real index.
- **Country-code-aware phone dedup.** Rohlik leaked `+420771231771` AND `771 231 771` as separate phones (same number, E.164 vs national format). `dedup_phones()` now strips known country-code prefixes (`+420`, `+1`, `+44`, 30+ codes) before keying. Variants with `+` are preferred when collapsing (internationally unambiguous).
- **Adobe Typekit weight-encoded font names rejected.** Footshop leaked `tk-neue-haas-unica-n4` and `…-n5` (Typekit's weight-suffix aliases where `n4` = weight 400, `n5` = 500). The real `neue-haas-unica` is captured separately. Pattern: `^tk-.*-n[0-9]+$`.

#### Added

- **`ContactInfo.contact_form_endpoints`**: list of `action` URLs of forms classified as `contact`. Footshop and Bohemia Bagel have 0 emails/phones because they use contact forms (no `mailto:` / `tel:`) for anti-spam reasons. The endpoint(s) now surface in `index.md` with a note suggesting the agent's rebuilt form POST to the same URL.

### Round K — Cohort #4 fixes (8 fixes)

Driven by smoketest on Buttondown, Smashing.media, Partake.foods, Buckmason, Railway, Damejidlo, Prusa3D, Mejuri. Findings revealed CA-DOJ regulatory phone leaks from privacy pages, cross-domain sitemap redirects (mergers), case-sensitive email dups, multi-locale auto-detection, fallback/carousel-library font leaks, single-digit phone group SKUs, and a framework-detection coverage gap.

#### Fixed

- **CA-DOJ regulatory phones no longer leak from privacy pages.** Mejuri and Brooklyn Brewery emitted `(800) 952-5210` and `(916) 445-1254` as brand phones — these are the California Attorney General's office, referenced in CCPA notices. Body-text phone extraction is now suppressed when `URL slug` indicates a legal/privacy/terms page (`/privacy`, `/terms`, `/legal`, `/impressum`, `/ccpa`, `/gdpr`, `/cookies`, `/zasady`). `tel:` anchors are still honored even on legal pages.
- **Case-insensitive email dedup.** Mejuri had `PRESS@MEJURI.COM` AND `press@mejuri.com` as separate entries. `dedup_emails()` groups by lowercase and prefers the lowercase original when both variants are seen.
- **Default Accept-Language `en-US,en;q=0.9`.** Prusa3D auto-redirected to French on Czech-locale machines. The reqwest client and Chrome (`--lang=en-US`) now both pin English as the default. User can override via `--header`.
- **`-fallback` font filter expanded to space + underscore variants.** Prusa3D's `AtlasGrotesk Fallback` (capital F, space-separated) and `AtlasGrotesk_Fallback` (underscore) now rejected alongside the kebab-case form.
- **Carousel-library font names filtered.** Mejuri's `slick (6×)` (Slick.js carousel pseudo-element font) used to rank in the brand fonts list. Added `slick`, `swiper`, `splide`, `flickity`, `owl`, `owl-carousel` to the icon-font denylist.
- **Phone single-digit non-first group rejected.** Buckmason emitted `11301667 3` (8-digit + 1-digit product SKU shape) as a phone. Real phones have 2-4 digit groups after the leading country code; rejecting `<2` after the first group kills the SKU leak.
- **Cross-domain sitemap warning.** Damejidlo's sitemap redirects 100% of URLs to `foodora.cz` (post-merger). When ≥50% of sitemap URLs point at a different host than the input URL, `index.md` prepends a `⚠️ Cross-domain sitemap` banner naming the foreign host. Agent learns the bundle's content is from a different domain than its name suggests.
- **Framework detection extended.** Added Vite (hash-suffixed `/assets/<name>-<hash>.{js,css}` bundles — catches Railway.app), Remix (`__remix_run_`), SvelteKit `data-sveltekit-*`, Solid.js `_$HY` global, Qwik `q:container`. Closes the Round I/J gap where Railway showed no stack.

### Round J — Chrome reliability + confidence tuning (3 fixes)

#### Changed

- **Default `--concurrency` lowered from 10 → 5.** Brooklyn Brewery and Catbird repeatedly hit `headless_chrome` transport-loop crashes at 10 concurrent tabs ("Unable to make method calls because underlying connection is closed", "Got a timeout while listening for browser events"). Empirically 5 is the stable ceiling on SPA-heavy / WordPress sites with 10+ external stylesheets. For `--no-js` runs you can safely pass `-c 16` or higher.
- **Brand confidence thresholds tightened.** Schoolhouse (top color count = 10) used to register `medium` but the data is genuinely thin. New cuts: `< 10 = low`, `10–29 = medium`, `≥ 30 = high`.
- **Chrome render retries 2 → 3 with exponential backoff (400 ms → 1.5 s → 4 s).** Gives the browser time to recover between tab-open failures. Total worst-case extra latency is ~5.5 s on a fully-failing page, which is preferable to a silently-empty bundle.

#### Added

- `detect_quality_warnings_flags_spa_loading_shell` unit test pinning the SPA-shell detector — 10 fake pages sharing a 4-block template trigger the warning; 1-page bundles and varied-template bundles do not.

### Round I — small-biz cohort #2 retest (8 fixes)

Driven by a fresh smoketest on Strand Books, Brooklyn Brewery, McSweeneys, Kotn, Schoolhouse, Catbird, Glasswing, Lokál (lokal.ambi.cz). Findings revealed SPA loading-shell scrapes, bot-protected silent failures, paren-unbalanced phones, icon-font palette leakage, URL-encoded font names, and wasted page-cap slots on auth/account pages.

#### Added

- **`SiteData.quality_warnings`** + **`SiteData.skipped_pages`** — bundle-level diagnostics. `index.md` now prepends two new banners: **⚠️ SPA loading shell suspected** when ≥80% of pages share a tiny (<5 block) template (Brooklyn Brewery scraped 10 identical `img,img,h1,p` shells), and **⚠️ Partial scrape — N/M pages blocked** when ≥50% of attempted pages failed to render (Catbird had 9 of 10 pages bot-protected).
- **`BrandPalette.confidence`** — "high" / "medium" / "low" derived from top color/font sample counts. Surfaced in `index.md` with a verify-against-screenshots note when low/medium. Next.js / Tailwind CSS-in-JS sites typically land at "low" because their styling lives in runtime-generated stylesheets.
- **`balance_phone_parens()`** — repairs `718) 486-7422` → `(718) 486-7422` (orphan close paren common in body-text tokenization) and strips orphan opens. Unit tests pin the behavior.
- Utility-page deprioritization in `url_priority()` — `/account/`, `/cart`, `/checkout`, `/search`, `/login`, `/signup`, `/password` etc. now sort to priority 200 (lowest) so they only get scraped when the page cap is generous. Schoolhouse used to waste 4/10 page slots on these.

#### Fixed

- **Phone group-count limit** — max 4 groups for standard format, 5 for French dot-style (`06.12.34.56.78`). Rejects `29 30 31 32 34 36` (kotn store-hours calendar, 6 groups).
- **Icon-font names filtered from brand fonts** — Catbird's `catbird-icons (77×)` and Schoolhouse's `swiper-icons` no longer rank in the brand palette. Filter matches `-icons` / `-icon` / `iconic` / `glyph` suffixes plus exact names (FontAwesome, Material Icons, ETmodules, swiper-icons, slick-icons, ionicons, octicons, pagebuilder-font).
- **URL-encoded font names rejected** — Brooklyn Brewery's `Libre Franklin%3A300%2C300i...` (URL-encoded font-weight metadata from CSS-in-JS) used to surface as a "font family". Now dropped.

#### Engineering

- `scrape_all()` signature changed from `Vec<PageData>` to `(Vec<PageData>, Vec<SkippedPage>)` so the caller can surface skipped URLs in the bundle.

### Round H — HIGH-severity polish from Round F retest (4 fixes)

#### Fixed

- **Phone regex now anchored to a single line.** The `\s` character class allowed matches to cross newlines, combining unrelated digit groups (timestamps + adjacent product IDs on Shopify cards). Replaced with a literal space. Stumptown now returns `1-800-352-5267` and `1-855-711-3385`; Levain returns `+18443117893` and `877-932-2161`.
- **Priority-sort URLs before `--max-pages` truncation.** Home → contact → about → legal/privacy/terms now survive the cap; Shopify-style e-commerce sitemaps (products-first ordering) used to drop the contact page that the agent needs most. Levain went from 0 to 2 real corporate emails (`media@levainbakery.com`, `shianne.smalling@levainbakery.com`).
- **Added `product` page category.** Detects URL paths `/products/`, `/product/`, `/shop/`, `/store/`, `/collections/`, `/produkty/`, plus JSON-LD `@type: "Product"`. Levain's cookie pages are now correctly tagged `product` instead of `service`.
- **Bootstrap utility palette filtered from brand colors.** Death & Co was reporting `#d9534f` (btn-danger), `#5cb85c` (btn-success), `#f0ad4e` (btn-warning) as brand because Bootstrap declares them hundreds of times. Same approach as the syntax-theme filter: drop the palette when 3+ Bootstrap colors match. Death & Co's real `#b32614` burgundy now ranks #1. Tailwind/Material default neutrals deliberately NOT filtered — real brands often use those exact hexes.

#### Added

- `url_priority()` helper in `src/util.rs` with regression test.
- Phone regression tests pinning `503-808-9080`, `(720) 330-2660` accept; `1762296503\n3` and Unix timestamps reject.
- Product-category regression tests for `/products/` and `/collections/` paths.

### Round F — small-biz cohort hardening (8 fixes)

Round F was driven by a smoketest on a real small-business cohort (Levain Bakery, Stumptown Coffee, Ace Hotel, Maitrea, French Laundry, Death & Co). Findings revealed Shopify sitemap pollution, catastrophic phone false-positives on e-commerce, silent failures on bot-protected sites, and false-positive WordPress detection.

#### Fixed

- **Shopify sitemap pollution** — sitemap.xml entries ending in `.xml` / `.txt` / `.md` / `.json` are now skipped during ingestion. Previously Shopify's `llms.txt`, `llms-full.txt`, `agents.md`, and `sitemap_products_1.xml?from=…&to=…` sub-sitemap URLs were scraped as content pages, producing 0-word noise pages.
- **Phone regex catastrophic Shopify failure** — `"1762296503 \n 3"` (Unix timestamp + Shopify product day-id) used to match as a phone. Now rejects: any candidate containing `\n` / `\r` (cross-element matches), AND any 10-digit pure-int candidate starting with `1[5-9]` (Unix timestamps 2017-2033).
- **Silent empty bundles** — when `total_pages == 0`, `index.md` now prepends a prominent `❌ Scrape failed — empty bundle` banner with the five likely root causes (WAF block, DNS failure, JS-render timeout, robots Disallow-all, sitemap-only XML/TXT). Previously the agent received "Pages: 0" with no explanation.
- **Email under-detection on Shopify contact pages** — chrome-only restriction (round E) missed legitimate addresses on Shopify-style sites where contact forms replace `mailto:`. Body emails are now extracted on pages whose URL slug indicates a contact zone (`/contact`, `/kontakt`, `/about`, `/imprint`, `/impressum`, `/legal`, `/contact-us`, `/get-in-touch`).
- **WordPress false-positive on Ace Hotel** — detector required only a single `/wp-content/` reference, which third-party widgets often satisfy. Now requires `/wp-content/` AND ≥1 of (`/wp-json/` REST, `wp-admin` link, `meta generator=WordPress`, `wp-emoji`). Drops single-signal matches to `low` confidence.
- **Shopify framework detection** — added signals for `cdn.shop`, `shopifycdn.com`, `monorail-edge.shopifysvc.com`, `Shopify.shop` JS global, `meta name="shopify-*"`. Levain Bakery and Stumptown Coffee are now correctly tagged.
- **Font-family CSS-property leakage** — `"object-fit:cover"` (a CSS property value, not a font) used to appear in the brand palette via Maitrea's CSS. New character-class whitelist: alphanumeric + space + hyphen only, length 2-40. Drops `object-fit:cover`, calc() values, URL fragments, etc.
- **Fully-transparent colors in brand palette** — `rgba(0,0,0,0)` and 8-digit hex with `00` alpha now filtered. Maitrea's palette no longer includes `rgba(0,0,0,0) (×7)`.

### Test-run output routing

#### Added

- **`--test-run` flag** — routes output to `test_runs/<host-slug>/` instead of the default `output/`. `<host-slug>` is derived from the target URL (e.g. `https://www.example.com/` → `example_com`, `https://sportujusa.cz/` → `sportujusa_cz`). The default output target stays as `output/scraped.json` so production / agent-handoff runs are unaffected; `--test-run` is purely a local-development convenience. Ignored if the user passes a custom `--output` path. `test_runs/` is already gitignored.
- Unit test `url_to_host_slug_strips_www_and_normalises` pinning the slug normalisation contract.

### Windows / Git Bash quality-of-life fix

#### Fixed

- **MSYS path translation on Windows Git Bash** — Git Bash silently rewrites leading-slash CLI arguments to Windows paths before they reach the binary (so `--exclude /home` becomes `C:/Program Files/Git/home`). `--exclude` and `--include` patterns starting with `/` were therefore failing to match anything on Windows. Both flags now run user-provided patterns through `unmsys_pattern()` which detects the known MSYS roots (`C:/Program Files/Git/`, `C:/Program Files (x86)/Git/`, `C:/msys64/`, `C:/cygwin64/`) and recovers the original `/foo` form. Generic drive-letter fallback recovers `/last-segment` for anything else. Regression test added.

#### Validated

- **sportujusa.cz Czech-only scrape** — 9/9 Czech pages captured, 0 English mutations leaked through `--exclude /services --exclude /home --exclude /how-to-get-a-scholarship --exclude /contact` on Windows Git Bash. WordPress (Divi) detected with high confidence. Brand `#2ea3f2` (150 occurrences). Fonts: Poppins (588), Open Sans (288), Arizonia (33). Contact: 1 email + 1 phone + 4 socials. Quality flags surfaced 8 pages missing `meta_description`, 5 with multiple `h1`, 7 with images missing alt — real WordPress SEO debt the agent should fix during the rebuild.

### Agent-quality polish — content types, agent UX, CLI flags, URL canonicalisation

#### Added — new content block types

- **`ContentBlock::Code { language, text }`** — `<pre>` / `<pre><code>` blocks with best-effort language detection from `class="language-rust"`, `class="hljs rust"`, etc. Critical for docs and engineering blogs (htmx.org went from 0 to 423 captured code blocks on a 3-page scrape).
- **`ContentBlock::Quote { text, cite }`** — `<blockquote>` block-level quotations (testimonials, pull quotes, callouts).
- **`ContentBlock::Media { kind, src, poster, title }`** — `<video>` and `<audio>` elements. Picks first `<source>` or direct `src=`. Hero videos and demo media no longer dropped.
- **`ContentBlock::DefinitionList { items: [{term, description}] }`** — `<dl>` / `<dt>` / `<dd>` pairs.
- **Figcaption fallback** — `<img>` inside `<figure>` with empty `alt` now inherits the `<figcaption>` text as alt.

#### Added — per-page agent UX

- **`content_hash`** — first 16 hex chars of SHA-256(plain_text). Lets the agent dedupe boilerplate across pages and detect change between runs.
- **`token_estimate`** — rough LLM token count (chars / 4) so the agent can budget calls.
- **`summary`** — auto-built one-liner: meta_description → first paragraph → first heading. Lives in `PageData` and `index.md`.

#### Added — CLI

- **`--user-agent <ua>`** — override the default UA when sites block it.
- **`--header "Name: Value"`** (repeatable) — extra HTTP headers for auth / cookie passthrough.
- **`--include <pattern>`** (repeatable) — whitelist patterns (stacks with `--exclude`).

#### Changed

- **URL canonicalisation before scraping** — strip fragment, drop tracking params (`utm_*`, `fbclid`, `gclid`, `mc_*`, `ref`, `ref_src`), collapse trailing slash, lowercase host. `/page` + `/page/` + `/page?utm_source=x` no longer scrape three times as separate pages.
- **Success/fail tally on completion** — `✅ Done! Scraped X/Y pages` now annotates `✗ N failed` when X < Y.
- **Double-counting guard** — descendants of `<blockquote>`, `<pre>`, `<dl>`, `<table>`, `<video>`, `<audio>` are no longer re-emitted as separate paragraphs/headings.

#### .gitignore

Added: `test_runs/`, `.cursor/`, `.fleet/`, `.aider/`, `*.orig`, `*.rej`, `coverage/`, `tarpaulin-report.html`, `*.profraw`, `*.profdata`, `node_modules/`, `dist/`, `.vercel/`, `.fly/`, `.netlify/`, `*.crash`, `*.dmp`.

### QA pass — second round of fixes

#### Added

- **`<table>` content extraction** as a new `ContentBlock::Table { caption, headers, rows }` variant. HN, Wikipedia, classic news sites, pricing tables, and comparison grids now produce real content_blocks instead of empty pages. Tables nested inside other tables are skipped to avoid double-counting layout chrome.
- **`--max-images-per-page <N>`** flag (default 100) — caps content-image downloads per page so image-heavy marketing sites don't dominate run time. `0` disables the cap.
- **Cloudflare / bot-protection interstitial detection** — Chrome render now bails early when it sees `cf-browser-verification`, `Just a moment...`, `Verifying you are human`, `challenge-platform`, PerimeterX markers, etc.
- **20 s hard timeout on `wait_for_element` and `wait_for_element_with_custom_timeout`** in render + screenshot paths. Previously, hung pages (gov.uk, heroku.com, bun.sh) blocked forever.
- **Known syntax-highlighting theme filter** for brand palette — drops Atom One Dark, Dracula, Monokai, Tomorrow Night, Solarized Dark, Nord, Catppuccin colours when 3+ from a single theme appear in the scanned palette. Dev-docs sites (htmx.org etc.) now surface their actual brand colour instead of `.hljs-string`.
- **Pygments-shaped selector filter** — short single/double/triple-letter classes (`.k`, `.s1`, `.cm`) are recognised as syntax-highlighting rules even without a `.hljs` / `.token` wrapper.

#### Fixed

- htmx.org top palette `#e06c75` (Atom One Dark red) → `rgb(52, 101, 164)` (htmx's actual blue brand colour)
- HN content_blocks were empty (everything was in `<table>`); now contain the news list with title / points / author / comments per row
- Silent hangs on Cloudflare-protected pages — they now fail fast with a `tracing::warn` and the run continues

### QA pass — first round of fixes (HN, htmx, daringfireball, rust-lang, solovino, anthropic)

#### Fixed

- **Phone false-positive on year-prefix strings** — `"2026-45185"` (HN thread IDs), `"1999-12345"`, `"2024 567890"` no longer match the phone regex. Year-prefix rejection covers the gap the ISO-date filter doesn't.
- **Testimonials over-detection** — detector removed entirely after producing 332 false hits on htmx.org and 178 on daringfireball.net. The signal (3+ consecutive paragraphs of similar length) is too weak; the agent can infer testimonials from context. We'll reintroduce only with a higher-precision signal (e.g. `<blockquote>` or schema.org `Review` markup).
- **Pricing-grid over-detection** — pricing detection now requires the currency-shaped string to be in a Heading, not buried in paragraph body text. Previously triggered on any blog post that mentioned money.
- **Social_links pollution** — social-platform URL detection is restricted to chrome zones (`<nav>` / `<header>` / `<footer>`). User-submitted body content (Hacker News posts pointing at GitHub / YouTube / Medium) no longer pollutes the site's social profile list.
- **HN-style brand colour missed** — `extract_style_text` now also harvests HTML4 `bgcolor=` / `color=` attributes (wrapped as `background: #...;`) so legacy table-based sites like Hacker News surface their brand colour (`#ff6600`) correctly.

#### Added

- Unit tests pinning the year-prefix rejection (`2026-45185`, `1999-12345`, `2024 567890`) in `contact::tests`.

## Round D — roadmap items 1-18

### Added

- `--delay <ms>` — politeness throttle enforced across concurrent tasks via a shared `RateLimiter`. Auto-honours `Crawl-delay:` from robots.txt when `--delay` isn't passed.
- `--js-wait-selector <css>` — wait for a meaningful element instead of a fixed wall-clock sleep (falls back to `--js-wait` if the selector never appears).
- `--no-js` — bypass headless Chrome entirely for static / server-rendered sites. Roughly 50× faster.
- `--crawl-with-http` — link-discovery crawl uses reqwest while per-page scrapes still use Chrome (unless `--no-js`).
- `--capture-404` — probe a synthetic non-existent URL and capture the site's 404 template into `site.json:error_pages`.
- `--screenshots` — full-page desktop (1280×800) + mobile (390×844) PNGs per page. Capture now runs in parallel.
- `--markdown` — emit per-page Markdown renderings under `output/markdown/`.
- `--split-pages` — write each page as its own JSON file under `output/pages/`.
- `--jsonl` — also write `scraped.jsonl` (newline-delimited PageData) for streaming consumers.
- `--ignore-robots` — opt out of the new robots.txt default-respect.
- `--no-extract-brand` — skip the brand palette extraction (default ON).
- `--no-fetch-css` — skip external stylesheet fetch for brand mining (default ON).
- `--quiet` / `--verbose` — control tracing log level.

#### Output bundle

- `scraped.json` — master file (every PageData)
- `site.json` — site-wide aggregate: nav, footer, contact, brand, **templates**, **hreflang_groups**, **frameworks**, sitemap, **assets**, **error_pages**
- `contact.json` — emails, phones, socials, addresses, organization
- `brand.json` — favicon, logo, colors, fonts, **CSS variables**, **webfont URLs**
- `index.md` — human-readable entry point with templates rollup + quality-flag rollup
- `compact.json` — stripped-down view for tight LLM context windows
- `schema.json` — JSON Schema describing the bundle shape
- `images/`, optional `pages/`, optional `markdown/`, optional `screenshots/`

#### Per-page fields

- `og_image_url`, `og_image_local_path`, `twitter_card`, `meta_robots`
- `hreflang_alternates`, `internal_links_out`, `page_assets`
- `plain_text` (concatenated headings + paragraphs + list items)
- `sections` (heuristic spans: `hero` / `features` / `team` / `gallery` / `testimonials` / `faq` / `pricing-grid` / `cta` / `embed` / `content`)
- `quality_flags` (`no_h1`, `multiple_h1:N`, `no_meta_description`, `meta_description_too_long`, `no_title`, `title_too_long`, `no_canonical`, `images_missing_alt:N`, `images_low_quality_alt:N`, `thin_content`, `meta_robots_noindex`, `meta_robots_nofollow`)
- Form blocks now carry `purpose`: `contact` / `newsletter` / `search` / `login` / `signup` / `payment` / `comment` / `generic`

#### Engineering

- Module split from a 770 LOC monolith into 11 focused modules.
- Migrated from `Box<dyn Error>` to `anyhow::Result` throughout.
- `tracing` + `tracing-subscriber` for structured logging; user-facing emoji status lines remain on stdout.
- 15 unit tests covering robots parsing, image-extension detection, phone validation, form classification, page categorisation, section detection, quality flags, and template grouping.
- GitHub Actions CI: `fmt --check`, `clippy -D warnings`, `test`, release build.
- Retry-with-backoff on Chrome page renders (1 retry) + asset fetches (2 retries, exponential 200 ms → 600 ms → 1.8 s).
- Content-Type sniffing for favicon/logo extension detection.
- Dominant-color fallback from logo/favicon image bytes when the CSS palette scan returns < 3 colours.
- Section detection expanded with `gallery` (3+ images), `testimonials` (3+ long paragraphs), `faq` (h3/h4 + p pairs), and `pricing-grid` (currency-shaped strings).
- Alt-text quality flag (`images_low_quality_alt:N`) catches placeholders like "image", "photo", filenames.

### Changed

- Brand extraction is now ON by default (opt out with `--no-extract-brand`).
- External CSS fetch is ON by default (opt out with `--no-fetch-css`).
- Robots.txt is fetched and respected by default (opt out with `--ignore-robots`).

### Fixed

- Phones no longer match SVG `<path d="...">` data (regex now runs on rendered text only, not raw HTML).
- Social-link detection uses parsed URL host with subdomain check, so `gtmetrix.com` no longer false-matches `x.com`.
- `image_extension_from_url` handles Next.js Image proxy URLs (`_next/image?url=...`).
- `local_path` is forward-slash-normalised on all platforms.
- Headless Chrome tabs are explicitly closed after each render to prevent leaks on large crawls.
- Sitemap-index recursion has a visited-set guard against circular references.
- Phone dedup keeps the best-formatted variant per unique digit sequence.

## [0.1.0] — initial public release (Dec 2025)

Initial WordPress / SPA scraper with sitemap auto-detection, JS rendering via headless Chrome, image download, form extraction.
