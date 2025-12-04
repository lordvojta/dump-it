use clap::Parser;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

type SitemapResult<'a> = Pin<
    Box<dyn std::future::Future<Output = Result<Vec<String>, Box<dyn std::error::Error>>> + 'a>,
>;

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

#[derive(Serialize, Deserialize, Clone)]
struct FormField {
    field_type: String,
    name: String,
    label: String,
    placeholder: String,
    required: bool,
    options: Vec<String>, // for select/radio/checkbox
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ContentBlock {
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
    },
}

#[derive(Serialize, Deserialize)]
struct PageData {
    url: String,
    title: String,
    meta_title: String,
    meta_description: String,
    content_blocks: Vec<ContentBlock>,
    total_words: usize,
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

    async fn download_image(&self, img_url: &str, output_dir: &str) -> Option<String> {
        // Filter out tracking pixels and unwanted images
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

        // Create hash-based filename from URL
        let mut hasher = Sha256::new();
        hasher.update(img_url.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        // Get file extension from URL
        let extension = img_url
            .split('?')
            .next()?
            .split('.')
            .last()
            .unwrap_or("jpg");

        let filename = format!("{}.{}", &hash[..16], extension);
        let filepath = format!("{}/{}", output_dir, filename);

        // Check if already downloaded
        if Path::new(&filepath).exists() {
            return Some(filepath);
        }

        // Download image
        match self.client.get(img_url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Ok(bytes) = response.bytes().await {
                    // Skip very small images (< 1KB, likely tracking pixels)
                    if bytes.len() < 1024 {
                        return None;
                    }
                    if fs::write(&filepath, &bytes).await.is_ok() {
                        return Some(filepath);
                    }
                }
            }
            _ => {}
        }

        None
    }

    async fn extract_content_blocks(
        &self,
        doc: &Html,
        page_url: &Url,
        output_dir: &str,
    ) -> Vec<ContentBlock> {
        let mut blocks = Vec::new();
        let mut seen_image_urls = HashSet::new();

        // Select main content area
        let main_selectors = ["main", "article", "[role='main']", "body"];
        let mut content_root = None;

        for selector_str in &main_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                if let Some(element) = doc.select(&selector).next() {
                    content_root = Some(element);
                    break;
                }
            }
        }

        let content_root = content_root.unwrap_or_else(|| {
            let body_selector = Selector::parse("body").unwrap();
            doc.select(&body_selector).next().unwrap()
        });

        // Skip nav, header, footer
        let skip_selector =
            Selector::parse("nav, header, footer, script, style, noscript").unwrap();

        // Collect image data first (to use later)
        struct ImageInfo {
            src: String,
            data_src: Option<String>,
            srcset: Option<String>,
            alt: String,
        }
        let mut image_infos = Vec::new();
        let mut processed_forms = HashSet::new();

        for element in content_root.descendants() {
            if let Some(elem_ref) = scraper::ElementRef::wrap(element) {
                let tag_name = elem_ref.value().name();

                // Check if element is inside a skip element
                let mut should_skip = false;
                for ancestor in elem_ref.ancestors() {
                    if let Some(anc_elem) = scraper::ElementRef::wrap(ancestor) {
                        if skip_selector.matches(&anc_elem) {
                            should_skip = true;
                            break;
                        }
                    }
                }

                if should_skip {
                    continue;
                }

                if matches!(tag_name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                    let level = tag_name.chars().last().unwrap().to_digit(10).unwrap() as u8;
                    let text: String = elem_ref.text().collect::<Vec<_>>().join(" ");
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Heading { level, text });
                    }
                } else if tag_name == "p" {
                    let text: String = elem_ref.text().collect::<Vec<_>>().join(" ");
                    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !text.is_empty() && text.len() > 20 {
                        blocks.push(ContentBlock::Paragraph { text });
                    }
                } else if tag_name == "img" {
                    let src = elem_ref.value().attr("src").map(|s| s.to_string());
                    let data_src = elem_ref.value().attr("data-src").map(|s| s.to_string());
                    let srcset = elem_ref.value().attr("srcset").map(|s| s.to_string());
                    let alt = elem_ref.value().attr("alt").unwrap_or("").to_string();

                    if let Some(src_val) = src {
                        image_infos.push(ImageInfo {
                            src: src_val,
                            data_src,
                            srcset,
                            alt,
                        });
                    }
                } else if matches!(tag_name, "ul" | "ol") {
                    let li_selector = Selector::parse("li").unwrap();
                    let items: Vec<String> = elem_ref
                        .select(&li_selector)
                        .map(|li| {
                            li.text()
                                .collect::<Vec<_>>()
                                .join(" ")
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .filter(|item| !item.is_empty())
                        .collect();

                    if !items.is_empty() {
                        blocks.push(ContentBlock::List { items });
                    }
                } else if tag_name == "form" {
                    // Create unique form ID to avoid duplicates
                    let form_id = format!("{:p}", elem_ref.value() as *const _);
                    if !processed_forms.contains(&form_id) {
                        processed_forms.insert(form_id);

                        let action = elem_ref.value().attr("action").unwrap_or("").to_string();
                        let method = elem_ref
                            .value()
                            .attr("method")
                            .unwrap_or("get")
                            .to_uppercase();

                        let mut fields = Vec::new();
                        let input_selector = Selector::parse("input, textarea, select").unwrap();

                        for input in elem_ref.select(&input_selector) {
                            let field_type = input
                                .value()
                                .attr("type")
                                .unwrap_or(input.value().name())
                                .to_string();

                            // Skip hidden fields and buttons in field list
                            if matches!(field_type.as_str(), "hidden" | "submit" | "button") {
                                continue;
                            }

                            let name = input.value().attr("name").unwrap_or("").to_string();
                            let placeholder = input.value().attr("placeholder").unwrap_or("").to_string();
                            let required = input.value().attr("required").is_some();

                            // Try to find associated label
                            let mut label = String::new();
                            if let Some(id) = input.value().attr("id") {
                                let label_selector = Selector::parse(&format!("label[for='{}']", id)).unwrap();
                                if let Some(label_elem) = doc.select(&label_selector).next() {
                                    label = label_elem.text().collect::<Vec<_>>().join(" ").trim().to_string();
                                }
                            }
                            // Fallback: check if input is inside a label
                            if label.is_empty() {
                                for ancestor in input.ancestors() {
                                    if let Some(anc_elem) = scraper::ElementRef::wrap(ancestor) {
                                        if anc_elem.value().name() == "label" {
                                            label = anc_elem.text().collect::<Vec<_>>().join(" ").trim().to_string();
                                            break;
                                        }
                                    }
                                }
                            }

                            // Extract select options
                            let mut options = Vec::new();
                            if input.value().name() == "select" {
                                let option_selector = Selector::parse("option").unwrap();
                                for option in input.select(&option_selector) {
                                    let opt_text = option.text().collect::<Vec<_>>().join(" ").trim().to_string();
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

                        // Extract submit button text
                        let mut submit_text = String::from("Submit");
                        let button_selector = Selector::parse("button[type='submit'], input[type='submit'], button:not([type])").unwrap();
                        if let Some(submit_btn) = elem_ref.select(&button_selector).next() {
                            if submit_btn.value().name() == "input" {
                                submit_text = submit_btn.value().attr("value").unwrap_or("Submit").to_string();
                            } else {
                                let text = submit_btn.text().collect::<Vec<_>>().join(" ").trim().to_string();
                                if !text.is_empty() {
                                    submit_text = text;
                                }
                            }
                        }

                        blocks.push(ContentBlock::Form {
                            action,
                            method,
                            fields,
                            submit_text,
                        });
                    }
                }
            }
        }

        // Now process images asynchronously
        for img_info in image_infos {
            let mut img_sources = vec![img_info.src.as_str()];
            if let Some(ref ds) = img_info.data_src {
                img_sources.push(ds.as_str());
            }
            if let Some(ref srcset) = img_info.srcset {
                if let Some(first_src) = srcset.split(',').next() {
                    let url_part = first_src.split_whitespace().next().unwrap_or("");
                    if !url_part.is_empty() {
                        img_sources.push(url_part);
                    }
                }
            }

            for src in img_sources {
                if let Ok(absolute_url) = page_url.join(src) {
                    let img_url = absolute_url.to_string();

                    if img_url.starts_with("data:")
                        || img_url.contains("1x1")
                        || img_url.contains("placeholder")
                        || seen_image_urls.contains(&img_url)
                    {
                        continue;
                    }

                    seen_image_urls.insert(img_url.clone());

                    if let Some(local_path) = self.download_image(&img_url, output_dir).await {
                        blocks.push(ContentBlock::Image {
                            original_url: img_url,
                            local_path,
                            alt_text: img_info.alt.clone(),
                        });
                        break;
                    }
                }
            }
        }

        blocks
    }

    async fn scrape_page(&self, url: String, output_dir: &str) -> Option<PageData> {
        let _permit = self.semaphore.acquire().await.ok()?;

        let response = self.client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            eprintln!("Failed to fetch {}: {}", url, response.status());
            return None;
        }

        let body = response.text().await.ok()?;
        let doc = Html::parse_document(&body);

        let page_url = Url::parse(&url).ok()?;

        // Extract title
        let title_selector = Selector::parse("title").unwrap();
        let title = doc
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "No title".to_string());

        // Extract meta tags
        let meta_selector = Selector::parse("meta").unwrap();
        let mut meta_title = String::new();
        let mut meta_description = String::new();

        for element in doc.select(&meta_selector) {
            if let Some(property) = element.value().attr("property") {
                if property == "og:title" {
                    meta_title = element.value().attr("content").unwrap_or("").to_string();
                } else if property == "og:description" && meta_description.is_empty() {
                    meta_description =
                        element.value().attr("content").unwrap_or("").to_string();
                }
            } else if let Some(name) = element.value().attr("name") {
                if name == "title" && meta_title.is_empty() {
                    meta_title = element.value().attr("content").unwrap_or("").to_string();
                } else if name == "description" && meta_description.is_empty() {
                    meta_description = element.value().attr("content").unwrap_or("").to_string();
                }
            }
        }

        // Fallback to title tag if no meta title
        if meta_title.is_empty() {
            meta_title = title.clone();
        }

        // Extract structured content blocks
        let content_blocks = self
            .extract_content_blocks(&doc, &page_url, output_dir)
            .await;

        // Calculate total word count from all blocks
        let total_words = content_blocks.iter().fold(0, |acc, block| {
            acc + match block {
                ContentBlock::Heading { text, .. } => text.split_whitespace().count(),
                ContentBlock::Paragraph { text } => text.split_whitespace().count(),
                ContentBlock::List { items } => items
                    .iter()
                    .map(|item| item.split_whitespace().count())
                    .sum(),
                ContentBlock::Image { .. } => 0,
                ContentBlock::Form { .. } => 0,
            }
        });

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

        println!("✓ Scraped: {} ({})", url, stats);

        Some(PageData {
            url,
            title,
            meta_title,
            meta_description,
            content_blocks,
            total_words,
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

    async fn scrape_all(&self, urls: Vec<String>, output_dir: String) -> Vec<PageData> {
        stream::iter(urls)
            .map(|url| {
                let output_dir = output_dir.clone();
                async move { self.scrape_page(url, &output_dir).await }
            })
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

    // Create output directories
    let output_path = std::path::Path::new(&args.output);
    let output_dir = output_path.parent().unwrap_or(std::path::Path::new("."));
    let images_dir = output_dir.join("images");

    std::fs::create_dir_all(output_dir)?;
    std::fs::create_dir_all(&images_dir)?;

    let images_dir_str = images_dir.to_string_lossy().to_string();

    let pages = scraper.scrape_all(urls, images_dir_str).await;

    let result = ScrapedData {
        total_pages: pages.len(),
        pages,
    };

    // Write to file
    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(&args.output, json)?;

    println!("✅ Done! Scraped {}/{} pages", result.total_pages, total);
    println!("💾 Output saved to: {}", args.output);

    Ok(())
}
