# dump-it

High-performance website scraper built with Rust. Intelligently scrapes websites using sitemaps or automatic crawling.

## Features

- **Sitemap auto-detection** - Finds and parses sitemap.xml automatically
- **Intelligent crawler** - Discovers pages by following links when no sitemap exists
- **Domain-bounded** - Stays within the target domain, won't follow external links
- **Duplicate prevention** - Tracks visited URLs to avoid re-scraping
- **Structured content** - Preserves page layout with headings, paragraphs, lists, images, and forms
- **Form extraction** - Captures all form fields, labels, types, and submit buttons
- **Image downloading** - Automatically downloads and saves all images from pages
- **Smart filtering** - Skips tracking pixels, tiny images, and analytics scripts
- **Meta tag extraction** - Captures meta title and meta description for SEO context
- **Concurrent scraping** - Fetches multiple pages in parallel for speed
- **Configurable limits** - Control depth, max pages, concurrency, timeouts
- **JSON output** - Structured data preserving page layout and hierarchy

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
```

## Options

- `-u, --url <URL>` - Target website or sitemap URL (required)
- `-c, --concurrency <N>` - Max concurrent requests (default: 10)
- `-t, --timeout <SECS>` - Request timeout in seconds (default: 30)
- `-o, --output <FILE>` - Output JSON file (default: output/scraped.json)
- `-d, --max-depth <N>` - Max crawl depth when no sitemap (default: 3)
- `-m, --max-pages <N>` - Max pages to scrape (default: 1000)

## Output Format

The scraper outputs structured JSON that preserves the page layout and content hierarchy:

```json
{
  "total_pages": 4,
  "pages": [
    {
      "url": "https://example.com/page",
      "title": "Page Title",
      "meta_title": "SEO Optimized Title",
      "meta_description": "Page description for search engines",
      "content_blocks": [
        {
          "type": "heading",
          "level": 1,
          "text": "Main Heading"
        },
        {
          "type": "paragraph",
          "text": "This is a paragraph with clean text content..."
        },
        {
          "type": "list",
          "items": [
            "First list item",
            "Second list item",
            "Third list item"
          ]
        },
        {
          "type": "image",
          "original_url": "https://example.com/photo.jpg",
          "local_path": "output/images/abc123def456.jpg",
          "alt_text": "Photo description"
        },
        {
          "type": "heading",
          "level": 2,
          "text": "Subheading"
        },
        {
          "type": "paragraph",
          "text": "More content here..."
        }
      ],
      "total_words": 450
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

### Page-Level Fields

- `url` - The page URL
- `title` - Page title from `<title>` tag
- `meta_title` - SEO title from meta tags (fallback to title)
- `meta_description` - SEO description from meta tags
- `content_blocks[]` - Ordered array of content blocks preserving layout
- `total_words` - Total word count across all text blocks

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

All outputs are saved to the `output/` folder (gitignored by default):
- `output/scraped.json` - Main JSON file with all scraped data
- `output/images/` - Downloaded images (named with content hash)

Images are saved with SHA-256 hash-based filenames to avoid duplicates and collisions.

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
- **Image formats**: Downloads images as-is (no format conversion)
- **Large images**: No automatic resizing or optimization

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
