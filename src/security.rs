pub const FINANCIAL_DOMAINS: &[&str] = &[
    "*bank*",
    "americanexpress.com",
    "ally.com",
    "allybank.com",
    "bankofamerica.com",
    "bofa.com",
    "capitalone.com",
    "chase.com",
    "citi.com",
    "citibank.com",
    "discover.com",
    "navyfederal.org",
    "pnc.com",
    "regions.com",
    "synchrony.com",
    "truist.com",
    "usbank.com",
    "wellsfargo.com",
    "zionsbank.com",
    "paypal.com",
    "venmo.com",
    "zelle.com",
    "stripe.com",
    "checkout.stripe.com",
    "squareup.com",
    "cash.app",
    "wise.com",
    "remitly.com",
    "fidelity.com",
    "schwab.com",
    "robinhood.com",
    "tradestation.com",
    "etrade.com",
    "tdameritrade.com",
    "webull.com",
    "interactivebrokers.com",
    "ibkr.com",
    "vanguard.com",
    "coinbase.com",
    "binance.com",
    "kraken.com",
    "gemini.com",
    "metamask.io",
    "blockchain.com",
    "checkout",
    "payment",
    "/pay/",
];

const READ_ONLY_ACTIONS: &[&str] = &["screenshot", "extract_content", "get_text", "get_html"];
const WRITE_ACTIONS: &[&str] = &[
    "click",
    "type",
    "type_text",
    "fill",
    "fill_form",
    "inject_script",
    "submit",
    "submit_form",
    "eval",
    "evaluate",
];

fn normalize_url(url: &str) -> String {
    url.trim().to_ascii_lowercase()
}

fn split_host_and_path(url: &str) -> (&str, &str) {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let without_credentials = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host_port, path) = without_credentials
        .split_once('/')
        .map(|(host, rest)| (host, rest))
        .unwrap_or((without_credentials, ""));
    let host = host_port
        .split_once(':')
        .map(|(host, _)| host)
        .unwrap_or(host_port)
        .trim_start_matches("www.");

    (host, path)
}

fn matches_domain(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn matches_pattern(host: &str, normalized_url: &str, pattern: &str) -> bool {
    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 2 {
        let needle = &pattern[1..pattern.len() - 1];
        return host.contains(needle) || normalized_url.contains(needle);
    }

    if pattern.contains('/') || !pattern.contains('.') {
        return normalized_url.contains(pattern);
    }

    matches_domain(host, pattern)
}

pub fn is_financial_site(url: &str) -> bool {
    let normalized_url = normalize_url(url);
    if normalized_url.is_empty() {
        return false;
    }

    let (host, path) = split_host_and_path(&normalized_url);
    let normalized_target = if path.is_empty() {
        host.to_string()
    } else {
        format!("{host}/{path}")
    };

    FINANCIAL_DOMAINS
        .iter()
        .any(|pattern| matches_pattern(host, &normalized_target, pattern))
}

pub fn check_browser_write_action(url: &str, action: &str) -> Result<(), String> {
    let normalized_action = action.trim().to_ascii_lowercase();

    if READ_ONLY_ACTIONS.contains(&normalized_action.as_str()) {
        return Ok(());
    }

    if !WRITE_ACTIONS.contains(&normalized_action.as_str()) {
        return Ok(());
    }

    if !is_financial_site(url) {
        return Ok(());
    }

    Err(format!(
        "Confirmation required: blocked browser write action '{normalized_action}' on financial site '{url}'. Ask the user to confirm before proceeding."
    ))
}

#[cfg(test)]
mod tests {
    use super::{check_browser_write_action, is_financial_site};

    #[test]
    fn detects_financial_domains_and_paths() {
        assert!(is_financial_site("https://www.chase.com/personal"));
        assert!(is_financial_site("https://checkout.example.com/session"));
        assert!(is_financial_site("https://example.com/payment/confirm"));
        assert!(is_financial_site("https://example.com/pay/invoice"));
    }

    #[test]
    fn ignores_non_financial_urls() {
        assert!(!is_financial_site("https://example.com/docs"));
        assert!(!is_financial_site("https://openai.com/research"));
    }

    #[test]
    fn blocks_write_actions_on_financial_sites() {
        let result = check_browser_write_action("https://www.paypal.com/checkout", "click");
        assert!(result.is_err());
    }

    #[test]
    fn allows_read_only_actions_on_financial_sites() {
        let result = check_browser_write_action("https://www.paypal.com/checkout", "get_html");
        assert!(result.is_ok());
    }
}
