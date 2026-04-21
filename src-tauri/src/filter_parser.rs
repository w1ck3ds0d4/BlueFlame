//! Parse a subset of the easylist/ublock filter format into regex patterns.
//!
//! Supported syntax:
//! - `||domain.tld^` -> block any request to that domain (and subdomains)
//! - `||domain.tld/path` -> block requests to that URL prefix
//! - `/path/` -> block requests containing that path
//! - `domain.tld^` -> block a bare domain
//! - Lines beginning with `!`, `[`, or blank are comments/metadata (skipped)
//! - Element-hiding rules (`##`, `#@#`) are skipped (we operate on network, not DOM)
//! - Exception rules (`@@`) are skipped in this first cut
//! - Option suffixes (`$script`, `$third-party`, etc.) are ignored - the rule
//!   still applies, just without the filtering nuance. Good enough as a first pass.

/// Convert one easylist line to a regex pattern, or `None` if the line is a
/// comment, unsupported, or explicitly skipped.
pub fn line_to_pattern(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Comments and metadata
    if trimmed.starts_with('!') || trimmed.starts_with('[') {
        return None;
    }
    // Element-hiding rules - not applicable at network layer
    if trimmed.contains("##") || trimmed.contains("#@#") || trimmed.contains("#?#") {
        return None;
    }
    // Exception rules (allow-list) - skip for now; a future PR can support them
    if trimmed.starts_with("@@") {
        return None;
    }

    // Rules with trailing `$options` are contextual - they apply only when
    // the request is third-party, or on a specific domain, or for a specific
    // resource type, etc. We don't track any of that context at the proxy
    // layer, so applying the rule unconditionally overblocks (classic
    // example: a `$third-party` rule for `?q=foo` ends up blocking the
    // user's first-party DuckDuckGo search). Skip those entirely - we
    // favor false negatives (missed ads) over false positives (breaking
    // the user's searches).
    if let Some(i) = trimmed.find('$') {
        if !preceding_escape(trimmed, i) {
            return None;
        }
    }

    translate(trimmed)
}

fn preceding_escape(s: &str, i: usize) -> bool {
    s.as_bytes().get(i.wrapping_sub(1)) == Some(&b'\\')
}

fn translate(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    // `||example.com` -> scheme-flexible host anchor
    if let Some(rest) = body.strip_prefix("||") {
        // Host runs until a path separator (`/`), separator marker (`^`), or end.
        let host_end = rest.find(['/', '^']).unwrap_or(rest.len());
        let host = &rest[..host_end];
        let tail = &rest[host_end..];

        let mut pattern = String::from(r"^https?://([^/]+\.)?");
        pattern.push_str(&escape_literal(host));
        if tail.is_empty() {
            // Bare domain anchor - require a separator or end of URL after the host.
            pattern.push_str(r"(?:[/:?&#]|$)");
        } else {
            pattern.push_str(&translate_tail(tail));
        }
        return Some(pattern);
    }

    // `|http://...` - exact start anchor
    if let Some(rest) = body.strip_prefix('|') {
        return Some(format!("^{}", translate_tail(rest)));
    }

    // Ends-with `|`
    if let Some(body) = body.strip_suffix('|') {
        return Some(format!("{}$", translate_tail(body)));
    }

    // Otherwise treat as a substring match
    Some(translate_tail(body))
}

/// Translate an easylist body into a regex fragment.
fn translate_tail(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for ch in s.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '^' => out.push_str(r"[/:?&#]"),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '|' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for ch in s.chars() {
        match ch {
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '|' | '$' | '*' | '^' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Parse a whole easylist-formatted document into a list of regex patterns.
/// Invalid and unsupported lines are silently dropped.
pub fn parse_document(body: &str) -> Vec<String> {
    body.lines().filter_map(line_to_pattern).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn compile(line: &str) -> Regex {
        let pat = line_to_pattern(line).unwrap_or_else(|| panic!("line did not parse: {line}"));
        Regex::new(&pat).unwrap_or_else(|e| panic!("regex failed for {line}: {e}\npat={pat}"))
    }

    #[test]
    fn domain_anchor_blocks_host_and_subdomains() {
        let r = compile("||doubleclick.net^");
        assert!(r.is_match("https://doubleclick.net/dc.js"));
        assert!(r.is_match("https://stats.doubleclick.net/track"));
        assert!(r.is_match("http://ad.doubleclick.net/"));
        assert!(!r.is_match("https://example.com/"));
    }

    #[test]
    fn domain_with_path_prefix() {
        let r = compile("||facebook.com/tr");
        assert!(r.is_match("https://www.facebook.com/tr?id=123"));
        assert!(r.is_match("https://connect.facebook.com/tr/"));
        assert!(!r.is_match("https://www.facebook.com/profile"));
    }

    #[test]
    fn bare_substring_match() {
        let r = compile("/banner_");
        assert!(r.is_match("https://cdn.example.com/img/banner_728.png"));
        assert!(!r.is_match("https://cdn.example.com/img/hero.png"));
    }

    #[test]
    fn rules_with_options_are_skipped() {
        // Contextual rules get dropped entirely - we'd overblock without a
        // way to enforce `$third-party`, `$domain=`, `$script`, etc.
        assert!(line_to_pattern("||hotjar.com^$third-party").is_none());
        assert!(line_to_pattern("||ads.example/*$domain=foo.com").is_none());
        assert!(line_to_pattern("/track?$script,xhr").is_none());
    }

    #[test]
    fn exception_rules_are_skipped() {
        assert!(line_to_pattern("@@||example.com^").is_none());
    }

    #[test]
    fn cosmetic_rules_are_skipped() {
        assert!(line_to_pattern("example.com##.ad-banner").is_none());
        assert!(line_to_pattern("example.com#@#.ad-banner").is_none());
    }

    #[test]
    fn comments_and_metadata_are_skipped() {
        assert!(line_to_pattern("! EasyList").is_none());
        assert!(line_to_pattern("[Adblock Plus 2.0]").is_none());
        assert!(line_to_pattern("").is_none());
        assert!(line_to_pattern("   ").is_none());
    }

    #[test]
    fn wildcards_translate_to_dotstar() {
        let r = compile("||tracker.net/pixel*.gif");
        assert!(r.is_match("https://tracker.net/pixel1.gif"));
        assert!(r.is_match("https://tracker.net/pixel_12345.gif"));
    }

    #[test]
    fn parse_document_skips_noise() {
        let input = "\
! Comment\n\
[Adblock Plus 2.0]\n\
||doubleclick.net^\n\
\n\
||google-analytics.com^\n\
@@||notskipped.com^\n\
example.com##.ad\n\
";
        let patterns = parse_document(input);
        assert_eq!(patterns.len(), 2);
    }
}
