use futures::stream::{self, StreamExt};
use headless_chrome::{Browser, LaunchOptions};
use reqwest::Client;
use scraper::Html;
use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

use crate::contact::extract_contact;
use crate::extract::{
    extract_canonical, extract_content_blocks, extract_favicon, extract_footer_blocks,
    extract_hreflang, extract_internal_links, extract_language, extract_logo_url, extract_meta,
    extract_nav_links, extract_structured_data, extract_style_text, extract_stylesheet_urls,
};
use crate::model::{ContentBlock, PageData};
use crate::selectors::{SEL_LINK, SEL_LOC, USER_AGENT};
use crate::util::{element_text, parse_robots, url_matches_excludes, RateLimiter, RobotsRules};

type SitemapFut<'a> = Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<String>>> + 'a>>;

pub(crate) struct Scraper {
    pub client: Client,
    /// `None` when `--no-js` is active (HTTP-only path).
    pub browser: Option<Arc<Browser>>,
    pub semaphore: Arc<Semaphore>,
    pub js_wait_ms: u64,
    pub js_wait_selector: Option<String>,
    pub extract_brand: bool,
    /// Inter-request throttle (politeness). `None` = no throttle.
    pub rate_limiter: Option<Arc<RateLimiter>>,
    /// Cap on content images per page. `0` = no cap.
    pub max_images_per_page: usize,
}

impl Scraper {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        concurrency: usize,
        timeout: u64,
        js_wait_ms: u64,
        js_wait_selector: Option<String>,
        extract_brand: bool,
        no_js: bool,
        delay_ms: u64,
        max_images_per_page: usize,
        user_agent: Option<&str>,
        extra_headers: &[String],
    ) -> anyhow::Result<Self> {
        use anyhow::Context;
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT_LANGUAGE};
        let mut header_map = HeaderMap::new();
        // Default Accept-Language `en-US,en;q=0.9` so multi-locale sites
        // (Prusa3D, IKEA, etc.) don't auto-redirect to the user's
        // browser-detected locale. Without this Chrome inherits its
        // system locale, which on a Czech machine yields French / Czech
        // content on those sites. User can override via --header.
        header_map.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        for h in extra_headers {
            if let Some((name, value)) = h.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                match (HeaderName::try_from(name), HeaderValue::try_from(value)) {
                    (Ok(n), Ok(v)) => {
                        header_map.insert(n, v);
                    }
                    _ => tracing::warn!("ignored malformed --header value: {h}"),
                }
            } else {
                tracing::warn!("ignored --header without `Name: Value` form: {h}");
            }
        }
        let ua = user_agent.unwrap_or(USER_AGENT);
        // Always include Accept-Language (which is at minimum the en-US
        // default we set above) — `default_headers` is the only way to
        // apply it across every request.
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent(ua)
            .default_headers(header_map)
            .build()
            .context("failed to build HTTP client")?;

        let browser = if no_js {
            None
        } else {
            // `--lang=en-US` pins Chrome's UI / Accept-Language so multi-
            // locale sites (Prusa3D, IKEA, etc.) don't auto-redirect to
            // the system locale. Mirrors the reqwest Accept-Language
            // header set above. The user can still override by passing
            // a custom Accept-Language via `--header`.
            let lang_arg = std::ffi::OsStr::new("--lang=en-US");
            let launch_options = LaunchOptions::default_builder()
                .headless(true)
                .args(vec![lang_arg])
                .build()
                .map_err(|e| anyhow::anyhow!("failed to build Chrome launch options: {e}"))?;

            let browser = Browser::new(launch_options).map_err(|e| {
                anyhow::anyhow!(
                    "failed to launch headless Chrome: {e}\n\
                     Make sure Google Chrome or Chromium is installed and reachable via PATH \
                     (or set the CHROME env var to the executable path).\n\
                     If you don't need JS rendering, pass --no-js."
                )
            })?;
            Some(Arc::new(browser))
        };

        Ok(Self {
            client,
            browser,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            js_wait_ms,
            js_wait_selector,
            extract_brand,
            rate_limiter: RateLimiter::new(delay_ms),
            max_images_per_page,
        })
    }

    /// Render a single URL — Chrome if available, otherwise reqwest.
    /// Retries page-level failures once (Chrome path only); HTTP path
    /// already retries inside `fetch_with_retry`.
    async fn render(&self, url: &str) -> Option<String> {
        if let Some(limiter) = &self.rate_limiter {
            limiter.wait().await;
        }
        if let Some(browser) = &self.browser {
            // Three attempts with exponential backoff (400ms → 1.5s → 4s).
            // Brooklyn Brewery regression: headless_chrome's transport loop
            // crashes intermittently under load; a longer pause lets the
            // browser stabilize before the next tab-open attempt.
            const MAX_ATTEMPTS: u32 = 3;
            let backoffs_ms = [400u64, 1500, 4000];
            for attempt in 0..MAX_ATTEMPTS {
                let browser = Arc::clone(browser);
                let url_for_render = url.to_string();
                let js_wait_ms = self.js_wait_ms;
                let wait_sel = self.js_wait_selector.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::chrome::render_in_chrome(
                        &browser,
                        &url_for_render,
                        js_wait_ms,
                        wait_sel.as_deref(),
                    )
                })
                .await;

                match result {
                    Ok(Some(body)) => return Some(body),
                    Ok(None) => {
                        if attempt + 1 < MAX_ATTEMPTS {
                            tracing::warn!(
                                "Render retry {}/{} for {url}",
                                attempt + 1,
                                MAX_ATTEMPTS - 1
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(
                                backoffs_ms[attempt as usize],
                            ))
                            .await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("spawn_blocking error for {url}: {e}");
                        return None;
                    }
                }
            }
            None
        } else {
            match crate::util::fetch_with_retry(&self.client, url, 2).await {
                Some(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(text) => Some(text),
                    Err(e) => {
                        tracing::error!("Failed to read body for {url}: {e}");
                        None
                    }
                },
                Some(resp) => {
                    tracing::error!("HTTP {} for {url}", resp.status());
                    None
                }
                None => None,
            }
        }
    }

    pub fn fetch_sitemap_inner<'a>(
        &'a self,
        url: &'a str,
        visited: &'a Mutex<HashSet<String>>,
    ) -> SitemapFut<'a> {
        Box::pin(async move {
            {
                let mut v = visited.lock().await;
                if !v.insert(url.to_string()) {
                    return Ok(Vec::new());
                }
            }

            let response = self.client.get(url).send().await?;
            let body = response.text().await?;

            let mut urls = Vec::new();
            let doc = Html::parse_document(&body);

            if body.contains("<urlset") || body.contains("<sitemapindex") {
                for element in doc.select(&SEL_LOC) {
                    let loc = element_text(&element);
                    if loc.is_empty() {
                        continue;
                    }
                    // Path-only test (ignoring query/fragment) — Shopify
                    // sub-sitemaps look like `sitemap_products_1.xml?from=…`.
                    let path_only = loc
                        .split(['?', '#'])
                        .next()
                        .unwrap_or(&loc)
                        .to_ascii_lowercase();
                    if path_only.ends_with(".xml") {
                        match self.fetch_sitemap_inner(&loc, visited).await {
                            Ok(sub_urls) => urls.extend(sub_urls),
                            Err(e) => tracing::warn!("Failed to fetch sub-sitemap {loc}: {e}"),
                        }
                        continue;
                    }
                    // Skip non-page assets that Shopify (and others) expose
                    // via sitemap.xml — llms.txt, llms-full.txt, agents.md,
                    // robots.txt, raw .json endpoints. These aren't agent-
                    // rebuild content; the scraper produces empty-page noise
                    // when they get included.
                    if path_only.ends_with(".txt")
                        || path_only.ends_with(".md")
                        || path_only.ends_with(".json")
                    {
                        tracing::debug!("Skipping non-HTML sitemap entry: {loc}");
                        continue;
                    }
                    urls.push(loc);
                }
            } else {
                urls.push(url.to_string());
            }

            Ok(urls)
        })
    }

    pub async fn fetch_sitemap(&self, url: &str) -> anyhow::Result<Vec<String>> {
        let visited = Mutex::new(HashSet::new());
        self.fetch_sitemap_inner(url, &visited).await
    }

    /// Fetch and parse `/robots.txt`. Returns Disallow paths + Crawl-delay
    /// for `*` and `DumpIt`. Empty defaults if robots.txt is missing.
    pub async fn fetch_robots_rules(&self, base_url: &Url) -> RobotsRules {
        let robots_url = format!(
            "{}://{}/robots.txt",
            base_url.scheme(),
            base_url.host_str().unwrap_or("")
        );
        let body = match self.client.get(&robots_url).send().await {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(t) => t,
                Err(_) => {
                    return RobotsRules {
                        disallow: Vec::new(),
                        crawl_delay_ms: None,
                    }
                }
            },
            _ => {
                return RobotsRules {
                    disallow: Vec::new(),
                    crawl_delay_ms: None,
                }
            }
        };
        parse_robots(&body)
    }

    pub async fn scrape_page(&self, url: String, output_dir: &str) -> Option<PageData> {
        let _permit = self.semaphore.acquire().await.ok()?;

        let body = match self.render(&url).await {
            Some(b) => b,
            None => {
                tracing::error!("Failed to render: {url}");
                return None;
            }
        };

        let doc = Html::parse_document(&body);
        let page_url = Url::parse(&url).ok()?;

        let (title, meta_title, meta_description, og_image_url, twitter_card, meta_robots) =
            extract_meta(&doc);
        let canonical_url = extract_canonical(&doc, &page_url);
        let language = extract_language(&doc);
        let favicon_url = extract_favicon(&doc, &page_url);
        let nav_links = extract_nav_links(&doc, &page_url);
        let footer_blocks = extract_footer_blocks(&doc);
        let structured_data = extract_structured_data(&doc);
        let logo_url = extract_logo_url(&doc, &page_url, &structured_data);
        let hreflang_alternates = extract_hreflang(&doc, &page_url);
        let internal_links_out = extract_internal_links(&doc, &page_url);
        let page_contact = extract_contact(&doc, &page_url, &structured_data);
        let style_text = if self.extract_brand {
            extract_style_text(&doc)
        } else {
            String::new()
        };
        let stylesheet_urls = if self.extract_brand {
            extract_stylesheet_urls(&doc, &page_url)
        } else {
            Vec::new()
        };
        let content_blocks = extract_content_blocks(
            &self.client,
            &doc,
            &page_url,
            output_dir,
            self.max_images_per_page,
        )
        .await;

        let total_words = crate::util::count_words(&content_blocks);
        let plain_text = crate::util::blocks_to_plain_text(&content_blocks);
        let image_count = content_blocks
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image { .. }))
            .count();
        let form_count = content_blocks
            .iter()
            .filter(|b| matches!(b, ContentBlock::Form { .. }))
            .count();

        let stats = if form_count > 0 {
            format!(
                "{} blocks, {} words, {} images, {} forms",
                content_blocks.len(),
                total_words,
                image_count,
                form_count
            )
        } else {
            format!(
                "{} blocks, {} words, {} images",
                content_blocks.len(),
                total_words,
                image_count
            )
        };
        println!("✓ Scraped: {url} ({stats})");

        let page_contact = if page_contact.emails.is_empty()
            && page_contact.phones.is_empty()
            && page_contact.social_links.is_empty()
            && page_contact.addresses.is_empty()
            && page_contact.organization.is_none()
        {
            None
        } else {
            Some(page_contact)
        };

        Some(PageData {
            url,
            title,
            meta_title,
            meta_description,
            canonical_url,
            language,
            favicon_url,
            logo_url,
            og_image_url,
            og_image_local_path: None,
            twitter_card,
            meta_robots,
            hreflang_alternates,
            nav_links,
            footer_blocks,
            structured_data,
            content_blocks,
            plain_text,
            content_hash: String::new(),
            token_estimate: 0,
            summary: String::new(),
            page_assets: Vec::new(),
            sections: Vec::new(),
            quality_flags: Vec::new(),
            total_words,
            page_contact,
            internal_links_out,
            style_text,
            stylesheet_urls,
            screenshot_desktop: None,
            screenshot_mobile: None,
        })
    }

    pub fn extract_links(&self, html: &str, base_url: &Url) -> Vec<String> {
        let doc = Html::parse_document(html);
        let mut links = Vec::new();
        for element in doc.select(&SEL_LINK) {
            let Some(href) = element.value().attr("href") else {
                continue;
            };
            if href.starts_with("javascript:")
                || href.starts_with('#')
                || href.starts_with("mailto:")
                || href.starts_with("tel:")
            {
                continue;
            }
            if let Ok(absolute_url) = base_url.join(href) {
                let url_str = absolute_url.to_string();
                if url_str.starts_with("http://") || url_str.starts_with("https://") {
                    let clean = url_str.split('#').next().unwrap_or(&url_str).to_string();
                    if !clean.is_empty() {
                        links.push(clean);
                    }
                }
            }
        }
        links
    }

    /// Fetch a URL's HTML using plain reqwest (no Chrome). Used by the
    /// crawler when --crawl-with-http is set so link discovery is fast.
    async fn fetch_html_plain(&self, url: &str) -> Option<String> {
        if let Some(limiter) = &self.rate_limiter {
            limiter.wait().await;
        }
        match crate::util::fetch_with_retry(&self.client, url, 2).await {
            Some(resp) if resp.status().is_success() => resp.text().await.ok(),
            _ => None,
        }
    }

    pub async fn crawl(
        &self,
        start_url: &str,
        max_depth: usize,
        max_pages: usize,
        excludes: &[String],
        crawl_with_http: bool,
    ) -> Vec<String> {
        let base_url = match Url::parse(start_url) {
            Ok(u) => u,
            Err(_) => return vec![start_url.to_string()],
        };
        let Some(base_domain) = base_url.host_str().map(|s| s.to_string()) else {
            return vec![start_url.to_string()];
        };

        let visited = Arc::new(Mutex::new(HashSet::new()));
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((start_url.to_string(), 0));
        visited.lock().await.insert(start_url.to_string());

        let mut discovered_urls = Vec::new();

        println!("🕷️  Crawling website (max depth: {max_depth}, max pages: {max_pages})...");

        while let Some((url, depth)) = queue.pop_front() {
            if discovered_urls.len() >= max_pages {
                println!("⚠️  Reached max pages limit ({max_pages})");
                break;
            }

            discovered_urls.push(url.clone());

            if depth >= max_depth {
                continue;
            }

            let _permit = self.semaphore.acquire().await.ok();
            let body_opt = if crawl_with_http {
                self.fetch_html_plain(&url).await
            } else {
                self.render(&url).await
            };
            if let Some(body) = body_opt {
                let Ok(current_url) = Url::parse(&url) else {
                    continue;
                };
                let links = self.extract_links(&body, &current_url);
                for link in links {
                    if url_matches_excludes(&link, excludes) {
                        continue;
                    }
                    if let Ok(link_url) = Url::parse(&link) {
                        if link_url.host_str() == Some(base_domain.as_str()) {
                            let mut v = visited.lock().await;
                            if v.insert(link.clone()) {
                                queue.push_back((link, depth + 1));
                            }
                        }
                    }
                }
            }

            if discovered_urls.len() % 10 == 0 && !discovered_urls.is_empty() {
                println!("📍 Discovered {} pages so far...", discovered_urls.len());
            }
        }

        println!(
            "✓ Crawl complete: found {} unique URLs",
            discovered_urls.len()
        );
        discovered_urls
    }

    pub async fn scrape_all(
        &self,
        urls: Vec<String>,
        output_dir: String,
    ) -> (Vec<PageData>, Vec<crate::model::SkippedPage>) {
        let concurrency = self.semaphore.available_permits().max(1);
        let pairs: Vec<(String, Option<PageData>)> = stream::iter(urls)
            .map(|url| {
                let output_dir = output_dir.clone();
                async move {
                    let result = self.scrape_page(url.clone(), &output_dir).await;
                    (url, result)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;
        let mut pages = Vec::with_capacity(pairs.len());
        let mut skipped = Vec::new();
        for (url, opt) in pairs {
            match opt {
                Some(p) => pages.push(p),
                None => skipped.push(crate::model::SkippedPage {
                    url,
                    // Distinguishing bot_protected vs render_failed at
                    // this level isn't possible from scrape_page's return
                    // type alone (None means "didn't yield a page"). The
                    // chrome.rs render path logs a WARN before bailing on
                    // a challenge interstitial, so the log is the source
                    // of truth; here we tag generically as render_failed.
                    reason: "render_failed".to_string(),
                }),
            }
        }
        (pages, skipped)
    }
}
