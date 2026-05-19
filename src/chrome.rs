use headless_chrome::Browser;
use std::sync::Arc;
use std::time::Duration;

use crate::util::normalize_path;

/// Returns `true` if the HTML body looks like a bot-protection / challenge
/// interstitial (Cloudflare "Just a moment...", PerimeterX, Akamai, etc.)
/// rather than the real page. We don't pretend to bypass these — we just
/// flag them so the caller can drop the page and the user knows why.
fn looks_like_challenge_page(title_or_html: &str) -> bool {
    let lc = title_or_html.to_lowercase();
    lc.contains("just a moment...")
        || lc.contains("verifying you are human")
        || lc.contains("cf-browser-verification")
        || lc.contains("cf-challenge-running")
        || lc.contains("challenge-platform")
        || lc.contains("checking your browser before accessing")
        || lc.contains("attention required! | cloudflare")
        || lc.contains("ddos protection by cloudflare")
        || lc.contains("/_px/")
        || lc.contains("perimeterx")
}

/// Render a single page in headless Chrome and return its HTML.
///
/// Always closes the tab before returning so the browser doesn't leak tabs
/// across a large crawl. If `wait_selector` is provided, waits for that
/// element to appear (with a short post-buffer); otherwise falls back to a
/// fixed `js_wait_ms` sleep.
pub(crate) fn render_in_chrome(
    browser: &Arc<Browser>,
    url: &str,
    js_wait_ms: u64,
    wait_selector: Option<&str>,
) -> Option<String> {
    let tab = match browser.new_tab() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to open Chrome tab for {url}: {e}");
            return None;
        }
    };

    let html = (|| -> Option<String> {
        if let Err(e) = tab.navigate_to(url) {
            tracing::warn!("Failed to navigate {url}: {e}");
            return None;
        }
        // Hard cap on how long we'll wait for <body> — otherwise heavy /
        // bot-protected sites can hang forever. 20 s is generous for any
        // reasonable page; Cloudflare challenges typically don't resolve.
        if let Err(e) = tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(20)) {
            tracing::warn!("Body never appeared on {url} (timeout): {e}");
            return None;
        }
        if let Some(sel) = wait_selector {
            match tab.wait_for_element_with_custom_timeout(sel, Duration::from_secs(15)) {
                Ok(_) => std::thread::sleep(Duration::from_millis(200)),
                Err(_) => std::thread::sleep(Duration::from_millis(js_wait_ms)),
            }
        } else {
            std::thread::sleep(Duration::from_millis(js_wait_ms));
        }
        match tab.get_content() {
            Ok(content) => {
                if looks_like_challenge_page(&content) {
                    tracing::warn!(
                        "Bot-protection / challenge interstitial detected on {url} — skipping"
                    );
                    return None;
                }
                Some(content)
            }
            Err(e) => {
                tracing::warn!("Failed to read content from {url}: {e}");
                None
            }
        }
    })();

    let _ = tab.close(true);
    html
}

/// Render at the requested viewport and capture a full-page PNG screenshot.
/// Returns the relative `output/...` path on success.
pub(crate) fn capture_screenshot(
    browser: &Arc<Browser>,
    url: &str,
    js_wait_ms: u64,
    wait_selector: Option<&str>,
    width: u32,
    height: u32,
    out_path: &str,
) -> Option<String> {
    use headless_chrome::protocol::cdp::Emulation;
    use headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption;

    let tab = match browser.new_tab() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to open Chrome tab for screenshot {url}: {e}");
            return None;
        }
    };

    let result = (|| -> Option<()> {
        // Emulate the target viewport via DeviceMetrics.
        if let Err(e) = tab.call_method(Emulation::SetDeviceMetricsOverride {
            width,
            height,
            device_scale_factor: 1.0,
            mobile: width < 600,
            scale: None,
            screen_width: None,
            screen_height: None,
            position_x: None,
            position_y: None,
            dont_set_visible_size: None,
            screen_orientation: None,
            viewport: None,
            display_feature: None,
            device_posture: None,
        }) {
            tracing::warn!("Failed to set viewport for {url}: {e}");
            return None;
        }
        if let Err(e) = tab.navigate_to(url) {
            tracing::warn!("Failed to navigate {url}: {e}");
            return None;
        }
        if let Err(e) = tab.wait_for_element_with_custom_timeout("body", Duration::from_secs(20)) {
            tracing::warn!("Body never appeared on {url} (timeout): {e}");
            return None;
        }
        if let Some(sel) = wait_selector {
            match tab.wait_for_element_with_custom_timeout(sel, Duration::from_secs(15)) {
                Ok(_) => std::thread::sleep(Duration::from_millis(200)),
                Err(_) => std::thread::sleep(Duration::from_millis(js_wait_ms)),
            }
        } else {
            std::thread::sleep(Duration::from_millis(js_wait_ms));
        }
        match tab.capture_screenshot(CaptureScreenshotFormatOption::Png, None, None, true) {
            Ok(bytes) => match std::fs::write(out_path, bytes) {
                Ok(_) => Some(()),
                Err(e) => {
                    tracing::warn!("Failed to write screenshot to {out_path}: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to capture screenshot for {url}: {e}");
                None
            }
        }
    })();

    let _ = tab.close(true);
    result.map(|_| normalize_path(out_path))
}
