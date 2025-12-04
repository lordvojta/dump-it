use clap::Parser;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

type SitemapResult<'a> = Pin<Box<dyn std::future::Future<Output = Result<Vec<String>, Box<dyn std::error::Error>>> + 'a>>;

#[derive(Parser)]
#[command(name = "dump-it")]
#[command(about = "High-performance website scraper with sitemap support", long_about = None)]
struct Args {
    /// Target website URL or sitemap URL
    #[arg(short, long)]
    url: String,

    /// Maximum concurrent requests
    #[arg(short, long, default_value = "10")]
    concurrency: usize,

    /// Request timeout in seconds
    #[arg(short, long, default_value = "30")]
    timeout: u64,

    /// Output JSON file path
    #[arg(short, long, default_value = "output/scraped.json")]
    output: String,

    /// Maximum crawl depth when no sitemap exists (0 = single page, default: 3)
    #[arg(short = 'd', long, default_value = "3")]
    max_depth: usize,

    /// Maximum pages to scrape (prevents runaway crawling)
    #[arg(short = 'm', long, default_value = "1000")]
    max_pages: usize,
}

#[derive(Serialize, Deserialize)]
struct PageData {
    url: String,
    title: String,
    content: String,
    word_count: usize,
}

#[derive(Serialize)]
struct ScrapedData {
    total_pages: usize,
    pages: Vec<PageData>,
}

struct Scraper {
    client: Client,
    semaphore: Arc<Semaphore>,
}

impl Scraper {
    fn new(concurrency: usize, timeout: u64) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout))
            .user_agent("Mozilla/5.0 (compatible; DumpIt/0.1)")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            semaphore: Arc::new(Semaphore::new(concurrency)),
        }
    }

    fn fetch_sitemap<'a>(&'a self, url: &'a str) -> SitemapResult<'a> {
        Box::pin(async move {
            let response = self.client.get(url).send().await?;
            let body = response.text().await?;

            let mut urls = Vec::new();
            let doc = Html::parse_document(&body);

            // Try XML sitemap first
            if body.contains("<urlset") || body.contains("<sitemapindex") {
                let loc_selector = Selector::parse("loc").unwrap();
                for element in doc.select(&loc_selector) {
                    let url = element.text().collect::<String>().trim().to_string();
                    if url.ends_with(".xml") {
                        // Recursive sitemap
                        if let Ok(sub_urls) = self.fetch_sitemap(&url).await {
                            urls.extend(sub_urls);
                        }
                    } else {
                        urls.push(url);
                    }
                }
            } else {
                // Fallback: just scrape the given URL
                urls.push(url.to_string());
            }

            Ok(urls)
        })
    }

    async fn scrape_page(&self, url: String) -> Option<PageData> {
        let _permit = self.semaphore.acquire().await.ok()?;

        let response = self.client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            eprintln!("Failed to fetch {}: {}", url, response.status());
            return None;
        }

        let body = response.text().await.ok()?;
        let doc = Html::parse_document(&body);

        // Extract title
        let title_selector = Selector::parse("title").unwrap();
        let title = doc
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "No title".to_string());

        // Extract text content (remove script, style, etc.)
        let body_selector = Selector::parse("body").unwrap();
        let script_selector = Selector::parse("script, style, noscript").unwrap();

        let mut content = String::new();
        if let Some(body) = doc.select(&body_selector).next() {
            let mut body_html = body.html();

            // Remove scripts and styles
            let temp_doc = Html::parse_fragment(&body_html);
            for element in temp_doc.select(&script_selector) {
                body_html = body_html.replace(&element.html(), "");
            }

            let cleaned_doc = Html::parse_fragment(&body_html);
            content = cleaned_doc
                .root_element()
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
        }

        let word_count = content.split_whitespace().count();

        println!("✓ Scraped: {} ({} words)", url, word_count);

        Some(PageData {
            url,
            title,
            content,
            word_count,
        })
    }

    fn extract_links(&self, html: &str, base_url: &Url) -> Vec<String> {
        let doc = Html::parse_document(html);
        let link_selector = Selector::parse("a[href]").unwrap();
        let mut links = Vec::new();

        for element in doc.select(&link_selector) {
            if let Some(href) = element.value().attr("href") {
                // Resolve relative URLs
                if let Ok(absolute_url) = base_url.join(href) {
                    let url_str = absolute_url.to_string();
                    // Filter out anchors, mailto, tel, javascript, etc.
                    if url_str.starts_with("http://") || url_str.starts_with("https://") {
                        // Remove fragments
                        let clean_url = url_str.split('#').next().unwrap_or(&url_str).to_string();
                        if !clean_url.is_empty() {
                            links.push(clean_url);
                        }
                    }
                }
            }
        }

        links
    }

    async fn crawl(&self, start_url: &str, max_depth: usize, max_pages: usize) -> Vec<String> {
        let base_url = match Url::parse(start_url) {
            Ok(url) => url,
            Err(_) => return vec![start_url.to_string()],
        };

        let base_domain = match base_url.host_str() {
            Some(host) => host,
            None => return vec![start_url.to_string()],
        };

        let visited = Arc::new(Mutex::new(HashSet::new()));
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((start_url.to_string(), 0));

        let mut discovered_urls = Vec::new();
        visited.lock().await.insert(start_url.to_string());

        println!(
            "🕷️  Crawling website (max depth: {}, max pages: {})...",
            max_depth, max_pages
        );

        while let Some((url, depth)) = queue.pop_front() {
            if discovered_urls.len() >= max_pages {
                println!("⚠️  Reached max pages limit ({})", max_pages);
                break;
            }

            discovered_urls.push(url.clone());

            if depth >= max_depth {
                continue;
            }

            // Fetch page and extract links
            let _permit = self.semaphore.acquire().await.ok();
            if let Ok(response) = self.client.get(&url).send().await {
                if response.status().is_success() {
                    if let Ok(body) = response.text().await {
                        let current_url = Url::parse(&url).unwrap();
                        let links = self.extract_links(&body, &current_url);

                        for link in links {
                            // Only follow links on the same domain
                            if let Ok(link_url) = Url::parse(&link) {
                                if let Some(link_domain) = link_url.host_str() {
                                    if link_domain == base_domain {
                                        let mut visited_lock = visited.lock().await;
                                        if !visited_lock.contains(&link) {
                                            visited_lock.insert(link.clone());
                                            queue.push_back((link, depth + 1));
                                        }
                                    }
                                }
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

    async fn scrape_all(&self, urls: Vec<String>) -> Vec<PageData> {
        stream::iter(urls)
            .map(|url| self.scrape_page(url))
            .buffer_unordered(self.semaphore.available_permits())
            .filter_map(|x| async { x })
            .collect()
            .await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    println!("🚀 Starting scraper...");
    println!("Target: {}", args.url);
    println!("Concurrency: {}", args.concurrency);

    let scraper = Scraper::new(args.concurrency, args.timeout);

    // Determine if URL is a sitemap
    let urls = if args.url.contains("sitemap") || args.url.ends_with(".xml") {
        println!("📋 Parsing sitemap...");
        scraper.fetch_sitemap(&args.url).await?
    } else {
        // Try to find sitemap automatically
        let base_url = Url::parse(&args.url)?;
        let sitemap_url = format!(
            "{}://{}/sitemap.xml",
            base_url.scheme(),
            base_url.host_str().unwrap()
        );

        println!("🔍 Looking for sitemap at: {}", sitemap_url);
        match scraper.fetch_sitemap(&sitemap_url).await {
            Ok(urls) if !urls.is_empty() && urls.len() > 1 => {
                println!("✓ Found sitemap with {} URLs", urls.len());
                urls
            }
            _ => {
                println!("⚠️  No sitemap found, starting crawler...");
                scraper
                    .crawl(&args.url, args.max_depth, args.max_pages)
                    .await
            }
        }
    };

    let total = urls.len();
    println!("📊 Found {} URLs to scrape", total);

    let pages = scraper.scrape_all(urls).await;

    let result = ScrapedData {
        total_pages: pages.len(),
        pages,
    };

    // Write to file
    let json = serde_json::to_string_pretty(&result)?;

    // Ensure output directory exists
    if let Some(parent) = std::path::Path::new(&args.output).parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&args.output, json)?;

    println!("✅ Done! Scraped {}/{} pages", result.total_pages, total);
    println!("💾 Output saved to: {}", args.output);

    Ok(())
}
