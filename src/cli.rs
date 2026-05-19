use clap::Parser;

#[derive(Parser)]
#[command(name = "dump-it")]
#[command(
    about = "Website scraper for redesign/migration — emits a JSON consumable by coding agents",
    long_about = None
)]
pub(crate) struct Args {
    /// Target website URL or sitemap URL
    #[arg(short, long)]
    pub url: String,

    /// Maximum concurrent requests / Chrome tabs. Default 5 — empirically
    /// headless_chrome's transport loop becomes unstable above ~6 tabs on
    /// SPA-heavy or WordPress sites with 10+ external stylesheets (Brooklyn
    /// Brewery / Catbird regression). For `--no-js` runs with plain HTTP
    /// you can safely pass `-c 16` or higher.
    #[arg(short, long, default_value = "5")]
    pub concurrency: usize,

    /// Request timeout in seconds
    #[arg(short, long, default_value = "30")]
    pub timeout: u64,

    /// Output JSON file path
    #[arg(short, long, default_value = "output/scraped.json")]
    pub output: String,

    /// Maximum crawl depth when no sitemap exists
    #[arg(short = 'd', long, default_value = "3")]
    pub max_depth: usize,

    /// Maximum pages to scrape
    #[arg(short = 'm', long, default_value = "1000")]
    pub max_pages: usize,

    /// Milliseconds to wait after page load for JS to render
    #[arg(long, default_value = "2000")]
    pub js_wait: u64,

    /// CSS selector to wait for after navigation. Faster than --js-wait on
    /// well-known content (e.g. `main` or `[data-loaded]`). Falls back to
    /// --js-wait if the selector never appears.
    #[arg(long)]
    pub js_wait_selector: Option<String>,

    /// Disable the built-in URL exclude patterns (WP archives, Elementor templates, etc.)
    #[arg(long)]
    pub no_default_excludes: bool,

    /// Extra substring patterns to exclude from URLs (repeatable)
    #[arg(long = "exclude")]
    pub excludes: Vec<String>,

    /// Also write each page as its own JSON file under output/pages/<slug>.json
    #[arg(long)]
    pub split_pages: bool,

    /// Skip the brand palette + fonts extraction (otherwise on by default).
    #[arg(long)]
    pub no_extract_brand: bool,

    /// Capture a desktop + mobile screenshot of every page
    /// (saved to output/screenshots/<slug>.{desktop,mobile}.png).
    #[arg(long)]
    pub screenshots: bool,

    /// Download every linked external stylesheet so brand colors / fonts /
    /// CSS variables defined outside inline <style> blocks can be mined.
    /// On by default — disable with --no-fetch-css.
    #[arg(long)]
    pub no_fetch_css: bool,

    /// Emit a Markdown rendering of each page's content blocks under
    /// output/markdown/<slug>.md. Useful when the agent prefers MD over JSON.
    #[arg(long)]
    pub markdown: bool,

    /// Emit a compact.json that drops long text and binary fields so the
    /// whole bundle fits in a constrained LLM context window. Always on.
    #[arg(long, hide = true, default_value = "true")]
    pub _compact: bool,

    /// Skip launching headless Chrome and use plain reqwest instead. Much
    /// faster, but only captures server-rendered HTML. Recommended for
    /// static sites (Hugo, Jekyll, Astro static-output, plain HTML).
    #[arg(long)]
    pub no_js: bool,

    /// Don't fetch and respect /robots.txt. By default the scraper reads
    /// robots.txt and skips URLs disallowed for our user-agent.
    #[arg(long)]
    pub ignore_robots: bool,

    /// Probe a synthetic non-existent URL and capture the site's 404
    /// template. Emitted under site.json:error_pages.
    #[arg(long)]
    pub capture_404: bool,

    /// Politeness throttle: minimum milliseconds between consecutive page
    /// requests across all concurrent tasks. 0 = no throttle. If unset, the
    /// `Crawl-delay:` from robots.txt (if any) is honoured automatically.
    #[arg(long, default_value = "0")]
    pub delay: u64,

    /// Use reqwest (plain HTTP) instead of Chrome during the link-discovery
    /// crawl phase. The per-page scrape still uses Chrome unless --no-js is
    /// also set. Speeds up the crawl substantially when JS isn't needed to
    /// discover `<a href>` links.
    #[arg(long)]
    pub crawl_with_http: bool,

    /// Suppress non-error log output. Implies tracing level `warn`.
    #[arg(short = 'q', long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Increase log verbosity. Implies tracing level `debug`.
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Also write each page as a newline-delimited JSON record to
    /// `output/scraped.jsonl`. Useful for streaming consumers that want one
    /// PageData per line rather than parsing the full scraped.json array.
    #[arg(long)]
    pub jsonl: bool,

    /// Cap the number of content images downloaded per page. Pages with
    /// more candidates are truncated to the first N. Default 100; set to 0
    /// to disable the cap. Helps on image-heavy marketing sites.
    #[arg(long, default_value = "100")]
    pub max_images_per_page: usize,

    /// Override the default User-Agent header. Some sites block our default
    /// `Mozilla/5.0 (compatible; DumpIt/0.1)` UA.
    #[arg(long)]
    pub user_agent: Option<String>,

    /// Extra HTTP header `Name: Value` to send on every request. Repeatable.
    /// Use for cookies / auth tokens on members-only content.
    #[arg(long = "header")]
    pub headers: Vec<String>,

    /// Substring patterns URLs must contain to be kept. If any pattern is
    /// set, only matching URLs are scraped. Stacks with `--exclude` (exclude
    /// wins).
    #[arg(long = "include")]
    pub includes: Vec<String>,

    /// Route output to `test_runs/<host>/` instead of the default `output/`.
    /// Useful for keeping local development scrapes isolated from the
    /// canonical `output/` directory. Ignored if `--output` is explicitly set
    /// to a non-default path.
    #[arg(long)]
    pub test_run: bool,
}
