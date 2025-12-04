# dump-it

High-performance website scraper built with Rust. Intelligently scrapes websites using sitemaps or automatic crawling.

## Features

- **Sitemap auto-detection** - Finds and parses sitemap.xml automatically
- **Intelligent crawler** - Discovers pages by following links when no sitemap exists
- **Domain-bounded** - Stays within the target domain, won't follow external links
- **Duplicate prevention** - Tracks visited URLs to avoid re-scraping
- **Clean text extraction** - Removes HTML, scripts, styles, extracts pure content
- **Concurrent scraping** - Fetches multiple pages in parallel for speed
- **Configurable limits** - Control depth, max pages, concurrency, timeouts
- **JSON output** - Structured data with URLs, titles, content, word counts

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/vojtakotrc/dump-it.git
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

Download pre-built binaries from the [Releases](https://github.com/vojtakotrc/dump-it/releases) page.

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
```

## Options

- `-u, --url <URL>` - Target website or sitemap URL (required)
- `-c, --concurrency <N>` - Max concurrent requests (default: 10)
- `-t, --timeout <SECS>` - Request timeout in seconds (default: 30)
- `-o, --output <FILE>` - Output JSON file (default: output/scraped.json)
- `-d, --max-depth <N>` - Max crawl depth when no sitemap (default: 3)
- `-m, --max-pages <N>` - Max pages to scrape (default: 1000)

## Output Format

```json
{
  "total_pages": 42,
  "pages": [
    {
      "url": "https://example.com/page",
      "title": "Page Title",
      "content": "Full text content...",
      "word_count": 1234
    }
  ]
}
```

## How It Works

### 1. Sitemap Detection
When you provide a URL, the scraper first looks for `sitemap.xml` at the domain root. If found, it extracts all URLs and scrapes them directly.

### 2. Intelligent Fallback
If no sitemap exists (or contains only 1 URL), the scraper automatically starts crawling mode.

### 3. Web Crawler
The crawler discovers pages by:
- Starting at your provided URL
- Extracting all `<a href>` links from the page
- Following those links to discover more pages
- Only following links on the **same domain** (ignores external links)
- Tracking visited URLs to **avoid duplicates**
- Respecting **depth** and **max pages** limits

### 4. Concurrent Scraping
After discovering URLs (via sitemap or crawling), all pages are scraped concurrently for maximum performance.

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
- Stripped binary (~3MB)
- Async I/O with Tokio
- Concurrent fetching with semaphore-based rate limiting
- Efficient URL deduplication

## Output Location

All JSON outputs are saved to the `output/` folder (gitignored by default).

## Real-World Example

```bash
$ ./target/release/dump-it --url https://vytvorit-web.cz
🚀 Starting scraper...
Target: https://vytvorit-web.cz
Concurrency: 10
🔍 Looking for sitemap at: https://vytvorit-web.cz/sitemap.xml
✓ Found sitemap with 4 URLs
📊 Found 4 URLs to scrape
✓ Scraped: https://vytvorit-web.cz/blog (109 words)
✓ Scraped: https://vytvorit-web.cz (858 words)
✓ Scraped: https://vytvorit-web.cz/ochrana-osobnich-udaju (836 words)
✓ Scraped: https://vytvorit-web.cz/obchodni-podminky (1183 words)
✅ Done! Scraped 4/4 pages
💾 Output saved to: output/scraped.json
```

Result: 24KB JSON file with complete text content from all pages.

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

### "No sitemap found, starting crawler"
This is normal. The tool will automatically discover pages by following links.

### Crawler finds fewer pages than expected
- Increase `--max-depth` (site may have deep navigation)
- Increase `--max-pages` limit
- Check if pages are JavaScript-rendered (this tool only works with static HTML)

### Timeout errors
- Increase `--timeout` to 60 or higher
- Decrease `--concurrency` to reduce load
- Website may be blocking automated requests

### Getting blocked by rate limiting
- Decrease `--concurrency` to 5 or lower
- Add delays between requests (not currently supported)
- Website may require custom headers/authentication

## Limitations

- **JavaScript-rendered content**: Only captures server-side HTML, not client-side JS content
- **Authentication**: Cannot scrape pages behind login walls
- **Rate limiting**: Some sites may block high-frequency requests
- **Dynamic content**: AJAX-loaded content is not captured

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

Vojtech Kotrc - [GitHub](https://github.com/vojtakotrc)

## Acknowledgments

Built with:
- [Tokio](https://tokio.rs/) - Async runtime
- [Reqwest](https://github.com/seanmonstar/reqwest) - HTTP client
- [Scraper](https://github.com/causal-agent/scraper) - HTML parsing
- [Clap](https://github.com/clap-rs/clap) - CLI argument parsing
