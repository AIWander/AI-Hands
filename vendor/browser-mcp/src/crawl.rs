//! Polite recursive web crawler — backs the `crawl` and `map` tools.
//!
//! Honors robots.txt, rate-limits per domain, identifies with an honest
//! user-agent, and bounds every run (depth / pages / wall-clock). Reuses the
//! crate's reqwest + scraper stack. Optional egress proxy via CPC_CRAWLER_PROXY
//! — point it at a Tailscale-reachable HTTP proxy to crawl from a non-home IP.

use crate::bot_auth::Signer;
use reqwest::Url;
use scraper::{Html, Selector};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

const DEFAULT_UA: &str = "CPCBot/1.0 (+https://github.com/AIWander; polite crawler)";
const FETCH_TIMEOUT_SECS: u64 = 20;
const HARD_MAX_PAGES: usize = 2000;
const HARD_MAX_DEPTH: usize = 12;
const DEFAULT_PAGE_TEXT_CAP: usize = 4000;
const MAP_SHALLOW_DEPTH: usize = 2;
const MAP_SHALLOW_BUDGET_SECS: u64 = 60;

/// Honest crawler user-agent: `CPC_CRAWLER_UA` env var, else the built-in default.
pub fn default_user_agent() -> String {
    std::env::var("CPC_CRAWLER_UA").unwrap_or_else(|_| DEFAULT_UA.to_string())
}

/// Optional egress proxy from `CPC_CRAWLER_PROXY` (e.g. `http://100.x.x.x:8888`).
pub fn proxy_from_env() -> Option<String> {
    std::env::var("CPC_CRAWLER_PROXY")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

pub struct CrawlOptions {
    pub start_url: String,
    pub max_depth: usize,
    pub max_pages: usize,
    pub same_domain_only: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub user_agent: String,
    pub delay_ms: u64,
    pub wall_clock_secs: u64,
    pub respect_robots: bool,
    pub page_text_cap: usize,
    pub proxy: Option<String>,
}

impl CrawlOptions {
    pub fn new(start_url: impl Into<String>) -> Self {
        CrawlOptions {
            start_url: start_url.into(),
            max_depth: 3,
            max_pages: 50,
            same_domain_only: true,
            include: Vec::new(),
            exclude: Vec::new(),
            user_agent: default_user_agent(),
            delay_ms: 1000,
            wall_clock_secs: 120,
            respect_robots: true,
            page_text_cap: DEFAULT_PAGE_TEXT_CAP,
            proxy: proxy_from_env(),
        }
    }
}

#[derive(Serialize)]
pub struct CrawlPage {
    pub url: String,
    pub depth: usize,
    pub status: u16,
    pub title: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct CrawlReport {
    pub start_url: String,
    pub pages_crawled: usize,
    pub stopped_reason: String,
    pub elapsed_ms: u128,
    pub egress: String,
    pub pages: Vec<CrawlPage>,
}

#[derive(Serialize)]
pub struct MapReport {
    pub start_url: String,
    pub url_count: usize,
    pub source: String,
    pub egress: String,
    pub urls: Vec<String>,
}

// ---------------------------------------------------------------- robots.txt

struct Robots {
    disallow: Vec<String>,
    allow: Vec<String>,
    crawl_delay_ms: Option<u64>,
}

impl Robots {
    fn allow_all() -> Self {
        Robots {
            disallow: Vec::new(),
            allow: Vec::new(),
            crawl_delay_ms: None,
        }
    }
}

/// The token a robots.txt `User-agent:` line is matched against (the bit
/// before any `/`), lower-cased.
fn ua_token(user_agent: &str) -> String {
    user_agent
        .split('/')
        .next()
        .unwrap_or(user_agent)
        .trim()
        .to_ascii_lowercase()
}

/// Literal prefix of a robots path pattern (everything before the first `*`).
fn robots_prefix(pattern: &str) -> String {
    pattern.split('*').next().unwrap_or(pattern).to_string()
}

/// Parse robots.txt, merging every group that applies to `token` or `*`.
/// Merging is intentionally conservative — a polite bot errs toward not crawling.
fn parse_robots(body: &str, token: &str) -> Robots {
    let mut out = Robots::allow_all();
    let mut group_applies = false;
    let mut last_was_directive = false;
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, val) = match line.split_once(':') {
            Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim().to_string()),
            None => continue,
        };
        match key.as_str() {
            "user-agent" => {
                if last_was_directive {
                    group_applies = false;
                    last_was_directive = false;
                }
                let v = val.to_ascii_lowercase();
                if v == "*" || token.contains(&v) || v.contains(token) {
                    group_applies = true;
                }
            }
            "disallow" => {
                last_was_directive = true;
                if group_applies && !val.is_empty() {
                    out.disallow.push(robots_prefix(&val));
                }
            }
            "allow" => {
                last_was_directive = true;
                if group_applies && !val.is_empty() {
                    out.allow.push(robots_prefix(&val));
                }
            }
            "crawl-delay" => {
                last_was_directive = true;
                if group_applies {
                    if let Ok(secs) = val.parse::<f64>() {
                        if secs >= 0.0 {
                            out.crawl_delay_ms = Some((secs * 1000.0) as u64);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Longest-match wins between Allow and Disallow; Allow wins ties; default allow.
fn path_allowed(robots: &Robots, path: &str) -> bool {
    let longest = |rules: &[String]| -> Option<usize> {
        rules
            .iter()
            .filter(|p| path.starts_with(p.as_str()))
            .map(|p| p.len())
            .max()
    };
    match (longest(&robots.disallow), longest(&robots.allow)) {
        (Some(d), Some(a)) => a >= d,
        (Some(_), None) => false,
        (None, _) => true,
    }
}

// ---------------------------------------------------------------------- http

fn build_client(user_agent: &str, proxy: &Option<String>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent.to_string())
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5));
    if let Some(px) = proxy {
        if !px.trim().is_empty() {
            let p = reqwest::Proxy::all(px.trim())
                .map_err(|e| format!("invalid proxy '{}': {}", px, e))?;
            builder = builder.proxy(p);
        }
    }
    builder.build().map_err(|e| e.to_string())
}

/// Apply Web Bot Auth (RFC 9421) signature headers to a request if signing is enabled.
/// No-op when `signer` is None or the URL has no host.
fn signed(
    builder: reqwest::RequestBuilder,
    signer: Option<&Signer>,
    url: &Url,
) -> reqwest::RequestBuilder {
    if let Some(s) = signer {
        if let Some((si, sg)) = s.sign(url) {
            return builder
                .header("Signature-Input", si)
                .header("Signature", sg);
        }
    }
    builder
}

async fn fetch_robots(
    client: &reqwest::Client,
    page: &Url,
    token: &str,
    signer: Option<&Signer>,
) -> Robots {
    let mut robots_url = page.clone();
    robots_url.set_path("/robots.txt");
    robots_url.set_query(None);
    robots_url.set_fragment(None);
    match signed(client.get(robots_url.clone()), signer, &robots_url)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => parse_robots(&body, token),
            Err(_) => Robots::allow_all(),
        },
        // missing / 4xx / 5xx / network error => no robots.txt => crawling allowed
        _ => Robots::allow_all(),
    }
}

async fn fetch_page(
    client: &reqwest::Client,
    url: &Url,
    signer: Option<&Signer>,
) -> Result<(u16, String), String> {
    let resp = signed(client.get(url.clone()), signer, url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Skip body parse for non-text payloads (PDFs, images, archives).
    // JSON allowed so API-style targets are readable (application/json,
    // application/ld+json, etc. — text/json already matched by "text").
    if !ctype.is_empty()
        && !(ctype.contains("html")
            || ctype.contains("xml")
            || ctype.contains("text")
            || ctype.contains("json"))
    {
        return Ok((status, String::new()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    Ok((status, body))
}

// ------------------------------------------------------------------- parsing

/// Extract (title, whitespace-collapsed text, outbound http(s) links).
fn parse_page(html: &str, base: &Url) -> (String, String, Vec<Url>) {
    let doc = Html::parse_document(html);

    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let text = doc
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let mut links = Vec::new();
    if let Ok(sel) = Selector::parse("a[href]") {
        for el in doc.select(&sel) {
            if let Some(href) = el.value().attr("href") {
                if let Ok(joined) = base.join(href) {
                    if matches!(joined.scheme(), "http" | "https") {
                        links.push(joined);
                    }
                }
            }
        }
    }
    (title, text, links)
}

/// Dedup key: drop the fragment and any trailing slash.
fn normalize(u: &Url) -> String {
    let mut u = u.clone();
    u.set_fragment(None);
    u.as_str().trim_end_matches('/').to_string()
}

/// Same registrable site, treating `www.` as equivalent.
fn same_site(a: &str, b: &str) -> bool {
    a.trim_start_matches("www.")
        .eq_ignore_ascii_case(b.trim_start_matches("www."))
}

fn passes_filters(url: &str, include: &[String], exclude: &[String]) -> bool {
    if exclude.iter().any(|p| url.contains(p.as_str())) {
        return false;
    }
    if !include.is_empty() && !include.iter().any(|p| url.contains(p.as_str())) {
        return false;
    }
    true
}

fn truncate(s: &str, cap: usize) -> String {
    if cap == 0 || s.chars().count() <= cap {
        return s.to_string();
    }
    let mut out: String = s.chars().take(cap).collect();
    out.push_str(" ...[truncated]");
    out
}

fn describe_egress(proxy: &Option<String>) -> String {
    match proxy {
        Some(p) if !p.trim().is_empty() => format!("proxy:{}", p.trim()),
        _ => "direct".to_string(),
    }
}

// ------------------------------------------------------------------- crawl

pub async fn crawl(opts: CrawlOptions) -> Result<CrawlReport, String> {
    let started = Instant::now();
    let max_depth = opts.max_depth.min(HARD_MAX_DEPTH);
    let max_pages = opts.max_pages.clamp(1, HARD_MAX_PAGES);

    let start = Url::parse(&opts.start_url)
        .map_err(|e| format!("invalid start_url '{}': {}", opts.start_url, e))?;
    if !matches!(start.scheme(), "http" | "https") {
        return Err(format!(
            "unsupported scheme '{}': use http/https",
            start.scheme()
        ));
    }
    let root_host = start.host_str().unwrap_or_default().to_string();
    let token = ua_token(&opts.user_agent);
    let client = build_client(&opts.user_agent, &opts.proxy)?;
    let signer = Signer::from_env();
    let egress = describe_egress(&opts.proxy);

    let mut frontier: VecDeque<(Url, usize)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut robots_cache: HashMap<String, Robots> = HashMap::new();
    let mut last_fetch: HashMap<String, Instant> = HashMap::new();
    let mut pages: Vec<CrawlPage> = Vec::new();

    visited.insert(normalize(&start));
    frontier.push_back((start, 0));
    let mut stopped = "completed — frontier exhausted".to_string();

    while let Some((url, depth)) = frontier.pop_front() {
        if pages.len() >= max_pages {
            stopped = format!("stopped — max_pages ({}) reached", max_pages);
            break;
        }
        if started.elapsed().as_secs() >= opts.wall_clock_secs {
            stopped = format!("stopped — wall-clock timeout ({}s)", opts.wall_clock_secs);
            break;
        }

        let host = url.host_str().unwrap_or_default().to_string();

        // robots.txt gate
        if opts.respect_robots {
            if !robots_cache.contains_key(&host) {
                let r = fetch_robots(&client, &url, &token, signer.as_ref()).await;
                robots_cache.insert(host.clone(), r);
            }
            if let Some(r) = robots_cache.get(&host) {
                if !path_allowed(r, url.path()) {
                    continue; // disallowed — skip without fetching
                }
            }
        }

        // polite per-domain rate limit (max of configured delay and robots Crawl-delay)
        let delay_ms = robots_cache
            .get(&host)
            .and_then(|r| r.crawl_delay_ms)
            .unwrap_or(0)
            .max(opts.delay_ms);
        if delay_ms > 0 {
            if let Some(prev) = last_fetch.get(&host) {
                let elapsed = prev.elapsed();
                let want = Duration::from_millis(delay_ms);
                if elapsed < want {
                    tokio::time::sleep(want - elapsed).await;
                }
            }
        }
        last_fetch.insert(host.clone(), Instant::now());

        match fetch_page(&client, &url, signer.as_ref()).await {
            Ok((status, body)) => {
                let (title, text, links) = parse_page(&body, &url);
                pages.push(CrawlPage {
                    url: url.to_string(),
                    depth,
                    status,
                    title,
                    text: truncate(&text, opts.page_text_cap),
                });
                if depth < max_depth {
                    for link in links {
                        let key = normalize(&link);
                        if visited.contains(&key) {
                            continue;
                        }
                        let link_host = link.host_str().unwrap_or_default();
                        if opts.same_domain_only && !same_site(link_host, &root_host) {
                            continue;
                        }
                        if !passes_filters(link.as_str(), &opts.include, &opts.exclude) {
                            continue;
                        }
                        visited.insert(key);
                        frontier.push_back((link, depth + 1));
                    }
                }
            }
            Err(e) => {
                // Fail loud per page, but keep crawling the rest.
                pages.push(CrawlPage {
                    url: url.to_string(),
                    depth,
                    status: 0,
                    title: String::new(),
                    text: format!("[fetch error: {}]", e),
                });
            }
        }
    }

    Ok(CrawlReport {
        start_url: opts.start_url,
        pages_crawled: pages.len(),
        stopped_reason: stopped,
        elapsed_ms: started.elapsed().as_millis(),
        egress,
        pages,
    })
}

// --------------------------------------------------------------------- map

/// Extract `<loc>` entries from a sitemap (urlset or sitemapindex) XML body.
fn extract_locs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<loc>") {
        rest = &rest[i + 5..];
        match rest.find("</loc>") {
            Some(j) => {
                let loc = rest[..j].trim();
                if !loc.is_empty() {
                    out.push(loc.replace("&amp;", "&"));
                }
                rest = &rest[j + 6..];
            }
            None => break,
        }
    }
    out
}

fn accept_url(url: &str, root_host: &str, same_domain_only: bool) -> bool {
    if !same_domain_only {
        return true;
    }
    match Url::parse(url) {
        Ok(u) => same_site(u.host_str().unwrap_or_default(), root_host),
        Err(_) => false,
    }
}

pub async fn map_site(
    start_url: &str,
    max_urls: usize,
    same_domain_only: bool,
    user_agent: &str,
    proxy: Option<String>,
) -> Result<MapReport, String> {
    let max_urls = max_urls.clamp(1, HARD_MAX_PAGES);
    let start =
        Url::parse(start_url).map_err(|e| format!("invalid start_url '{}': {}", start_url, e))?;
    if !matches!(start.scheme(), "http" | "https") {
        return Err(format!(
            "unsupported scheme '{}': use http/https",
            start.scheme()
        ));
    }
    let root_host = start.host_str().unwrap_or_default().to_string();
    let client = build_client(user_agent, &proxy)?;
    let signer = Signer::from_env();
    let egress = describe_egress(&proxy);

    let mut found: HashSet<String> = HashSet::new();
    let mut from_sitemap = false;

    // 1. sitemap.xml / sitemap_index.xml (+ one level of nested sitemaps)
    for candidate in ["/sitemap.xml", "/sitemap_index.xml"] {
        if found.len() >= max_urls {
            break;
        }
        let mut sm = start.clone();
        sm.set_path(candidate);
        sm.set_query(None);
        sm.set_fragment(None);
        let body = match signed(client.get(sm.clone()), signer.as_ref(), &sm)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
            _ => continue,
        };
        for loc in extract_locs(&body) {
            if found.len() >= max_urls {
                break;
            }
            if loc.ends_with(".xml") {
                // nested sitemap — pull one level deeper
                let loc_url = match Url::parse(&loc) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                if let Ok(r2) = signed(client.get(loc_url.clone()), signer.as_ref(), &loc_url)
                    .send()
                    .await
                {
                    if r2.status().is_success() {
                        if let Ok(b2) = r2.text().await {
                            for l2 in extract_locs(&b2) {
                                if found.len() >= max_urls {
                                    break;
                                }
                                if accept_url(&l2, &root_host, same_domain_only) {
                                    found.insert(l2);
                                    from_sitemap = true;
                                }
                            }
                        }
                    }
                }
            } else if accept_url(&loc, &root_host, same_domain_only) {
                found.insert(loc);
                from_sitemap = true;
            }
        }
    }

    // 2. shallow link crawl to supplement pages missing from the sitemap
    let mut from_crawl = false;
    if found.len() < max_urls {
        let mut frontier: VecDeque<(Url, usize)> = VecDeque::new();
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(normalize(&start));
        frontier.push_back((start.clone(), 0));
        let crawl_started = Instant::now();
        while let Some((url, depth)) = frontier.pop_front() {
            if found.len() >= max_urls
                || crawl_started.elapsed().as_secs() >= MAP_SHALLOW_BUDGET_SECS
            {
                break;
            }
            let (status, body) = match fetch_page(&client, &url, signer.as_ref()).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            if status >= 400 {
                continue;
            }
            let (_title, _text, links) = parse_page(&body, &url);
            for link in links {
                if found.len() >= max_urls {
                    break;
                }
                let key = normalize(&link);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);
                let link_host = link.host_str().unwrap_or_default();
                if same_domain_only && !same_site(link_host, &root_host) {
                    continue;
                }
                found.insert(link.as_str().to_string());
                from_crawl = true;
                if depth + 1 < MAP_SHALLOW_DEPTH {
                    frontier.push_back((link, depth + 1));
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    let source = match (from_sitemap, from_crawl) {
        (true, true) => "sitemap + shallow crawl",
        (true, false) => "sitemap",
        (false, true) => "shallow crawl",
        (false, false) => "none found",
    }
    .to_string();

    let mut urls: Vec<String> = found.into_iter().collect();
    urls.sort();
    urls.truncate(max_urls);

    Ok(MapReport {
        start_url: start_url.to_string(),
        url_count: urls.len(),
        source,
        egress,
        urls,
    })
}
